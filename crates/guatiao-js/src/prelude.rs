// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What every context is given before a library runs: the script behind
//! the `guatiao` modules, and a `console`.

use rquickjs::function::Rest;
use rquickjs::{Coerced, Constructor, Ctx, Function, Object, Result, Symbol, Value};

const CORE: &str = include_str!("prelude/core.js");

/// The name the script stores its object under, as `Symbol.for` spells it.
const KEY: &str = "guatiao.core";

/// The most items of a list, and bytes of a buffer, a script may make a
/// value of or ask a host for: what a runtime's memory limit could hold,
/// since the copies are made on the Rust heap, outside that limit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bounds {
    pub(crate) items: usize,
    pub(crate) bytes: usize,
}

/// Without a memory limit.
const MOST_ITEMS: usize = 1 << 24;
const MOST_BYTES: usize = 64 << 20;

impl Bounds {
    /// What `memory` (bytes, or none) allows: a list item takes at least
    /// 16 bytes of QuickJS's memory.
    pub(crate) fn of(memory: Option<u64>) -> Bounds {
        match memory.map(|m| usize::try_from(m).unwrap_or(usize::MAX)) {
            None => Bounds {
                items: MOST_ITEMS,
                bytes: MOST_BYTES,
            },
            Some(m) => Bounds {
                items: (m / 16).clamp(1024, MOST_ITEMS),
                bytes: m.min(MOST_BYTES),
            },
        }
    }
}

/// The script's object: the classes and functions Rust needs from it.
pub(crate) struct Core<'js> {
    pub(crate) ctx: Ctx<'js>,
    pub(crate) handle_key: Symbol<'js>,
    pub(crate) bounds: Bounds,
    pub(crate) exact: Constructor<'js>,
    pub(crate) guatiao_error: Constructor<'js>,
    pub(crate) provider: Constructor<'js>,
    pub(crate) error_for: Function<'js>,
    pub(crate) object_prototype: Object<'js>,
}

fn key<'js>(ctx: &Ctx<'js>) -> Result<Symbol<'js>> {
    Symbol::new_global(ctx.clone(), KEY)
}

/// Gives `ctx` the script and a `console`, once, and the bounds a value
/// made in it is held to.
pub(crate) fn install(ctx: &Ctx<'_>, bounds: Bounds) -> Result<()> {
    let globals = ctx.globals();
    if globals.contains_key(key(ctx)?)? {
        return Ok(());
    }
    ctx.eval::<(), _>(CORE)?;
    let object: Object<'_> = globals.get(key(ctx)?)?;
    object.set("maxItems", bounds.items as f64)?;
    object.set("maxBytes", bounds.bytes as f64)?;
    console(ctx, &globals)
}

/// The script's object in `ctx`.
pub(crate) fn core<'js>(ctx: &Ctx<'js>) -> Result<Core<'js>> {
    let object: Object<'js> = ctx.globals().get(key(ctx)?)?;
    let object_ctor: Object<'js> = ctx.globals().get("Object")?;
    let bound = |name: &str| -> Result<usize> { Ok(object.get::<_, f64>(name)? as usize) };
    Ok(Core {
        ctx: ctx.clone(),
        handle_key: object.get("handleKey")?,
        bounds: Bounds {
            items: bound("maxItems")?,
            bytes: bound("maxBytes")?,
        },
        exact: object.get("ExactNumber")?,
        guatiao_error: object.get("GuatiaoError")?,
        provider: object.get("Provider")?,
        error_for: object.get("errorFor")?,
        object_prototype: object_ctor.get("prototype")?,
    })
}

/// `console.log` and the like: text on the process's standard output,
/// `warn` and `error` on its standard error.
fn console<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    let console = Object::new(ctx.clone())?;
    for (name, to_stderr) in [
        ("log", false),
        ("info", false),
        ("debug", false),
        ("warn", true),
        ("error", true),
    ] {
        let write = Function::new(ctx.clone(), move |args: Rest<Value<'js>>| -> Result<()> {
            let line = args
                .0
                .into_iter()
                .map(|v| match v.get::<Coerced<String>>() {
                    Ok(Coerced(text)) => text,
                    Err(_) => String::from("[unprintable]"),
                })
                .collect::<Vec<_>>()
                .join(" ");
            if to_stderr {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
            Ok(())
        })?;
        console.set(name, write)?;
    }
    globals.set("console", console)
}
