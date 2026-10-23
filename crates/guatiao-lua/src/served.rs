// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A Lua table of an object kind's methods, served as an object of that
//! kind: what Lua code hands a slot that takes a listener.
//!
//! The object is called on its Lua state's thread, while that thread is
//! inside the call that handed it over or a later one; a call from any
//! other thread is refused, since the state cannot be reached from there.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::cell::RefCell;
use std::thread::{self, ThreadId};

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, Arg, KindDescription, LentHandle, LentObject, RawObject, Serve, ServedTable,
};
use guatiao::{List, Map, Status, Value};
use mlua::prelude::*;

use crate::calls::{self, Handle, Outcome, ReadBack, Received, Target};

/// One thread's served objects, and the tables they are served through.
#[derive(Default)]
struct Served {
    next: usize,
    objects: BTreeMap<usize, (mlua::WeakLua, LuaTable)>,
    tables: BTreeMap<(String, u32, String), Arc<ServedTable>>,
}

std::thread_local! {
    static SERVED: RefCell<Served> = RefCell::new(Served::default());
}

/// Serves the objects of the thread it was made on.
struct LuaServe {
    thread: ThreadId,
}

impl Serve for LuaServe {
    fn call(
        &self,
        ctx: usize,
        kind: &KindDescription,
        slot: usize,
        args: Vec<Arg<'_>>,
    ) -> Result<Answer, ProviderError> {
        if thread::current().id() != self.thread {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                "a Lua object was called from a thread other than its Lua state's",
            ));
        }
        let gone = || ProviderError::new(Status::GUATIAO_ERR_GONE, "the Lua object is gone");
        let (weak, table) = SERVED
            .with(|s| s.borrow().objects.get(&ctx).cloned())
            .ok_or_else(gone)?;
        let lua = weak.try_upgrade().ok_or_else(gone)?;
        let described = &kind.slots()[slot];
        let answer = call_table(&lua, &table, described, args, None)?;
        Ok(match answer {
            Outcome::Nothing => Answer::Nothing,
            Outcome::Value(v) => Answer::Value(v),
            Outcome::Handed(o) => Answer::Object(o),
            Outcome::Table(kind, t) => {
                Answer::Object(object(&lua, &kind, t).map_err(|e| calls::provider_error(&e))?)
            }
        })
    }

    fn destroy(&self, ctx: usize) {
        if thread::current().id() == self.thread {
            let gone = SERVED.with(|s| s.borrow_mut().objects.remove(&ctx));
            drop(gone);
        }
    }
}

/// Calls `table[slot](table, args...)` for a slot of `described`'s with
/// core's arguments, writes lent ones back, and answers what Lua answered.
pub(crate) fn call_table(
    lua: &Lua,
    table: &LuaTable,
    described: &guatiao::library::kind::SlotDescription,
    mut args: Vec<Arg<'_>>,
    memory: Option<u64>,
) -> Result<Outcome, ProviderError> {
    let bad = |e: LuaError| calls::provider_error(&e);
    // Lent objects first: each callable while the call runs.
    let lent: Vec<(usize, LentObject<'_>)> = args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| match a {
            Arg::LentObject(o) => Some((i, *o)),
            _ => None,
        })
        .collect();
    sharing(lent, Vec::new(), |handles| {
        let mut values = Vec::with_capacity(args.len());
        let mut received = Vec::with_capacity(args.len());
        for (i, (arg, d)) in args.iter_mut().zip(described.args()).enumerate() {
            let (v, r) = if let Some(v) = calls::scalar_arg(lua, arg).map_err(bad)? {
                (v, Received::Plain)
            } else {
                match arg {
                    Arg::Value(Some(v)) => calls::received(lua, d.ty(), v).map_err(bad)?,
                    Arg::Value(None) => (LuaValue::Nil, Received::Plain),
                    Arg::Map(m) => {
                        calls::received(lua, d.ty(), &Value::from((*m).clone())).map_err(bad)?
                    }
                    Arg::List(l) => {
                        calls::received(lua, d.ty(), &Value::from((*l).clone())).map_err(bad)?
                    }
                    Arg::TakenValue(v) => calls::received(lua, d.ty(), v).map_err(bad)?,
                    Arg::TakenMap(m) => {
                        calls::received(lua, d.ty(), &Value::from(core::mem::take(m)))
                            .map_err(bad)?
                    }
                    Arg::TakenList(l) => {
                        calls::received(lua, d.ty(), &Value::from(core::mem::take(l)))
                            .map_err(bad)?
                    }
                    Arg::LentValue(v) => calls::received(lua, d.ty(), v).map_err(bad)?,
                    Arg::LentMap(m) => {
                        calls::received(lua, d.ty(), &Value::from((**m).clone())).map_err(bad)?
                    }
                    Arg::LentList(l) => {
                        calls::received(lua, d.ty(), &Value::from((**l).clone())).map_err(bad)?
                    }
                    Arg::Buffer(b) => calls::buffer(lua, b.to_vec()).map_err(bad)?,
                    Arg::Object(_) => {
                        let Arg::Object(o) = core::mem::replace(arg, Arg::Value(None)) else {
                            unreachable!("matched just above")
                        };
                        let kind = match d.ty() {
                            guatiao::library::kind::ArgType::Object { kind, .. } => kind.clone(),
                            _ => String::new(),
                        };
                        (
                            calls::object_handle(lua, &kind, o).map_err(bad)?,
                            Received::Plain,
                        )
                    }
                    Arg::LentObject(_) => {
                        let handle = handles
                            .iter()
                            .find(|(at, _)| *at == i)
                            .map(|(_, h)| h.clone())
                            .ok_or_else(|| {
                                ProviderError::new(
                                    Status::GUATIAO_ERR_INTERNAL,
                                    "a lend went missing",
                                )
                            })?;
                        let kind = match d.ty() {
                            guatiao::library::kind::ArgType::Object { kind, .. } => kind.clone(),
                            _ => String::new(),
                        };
                        let caller = calls::caller(lua, &kind).map_err(bad)?;
                        let ud = lua
                            .create_userdata(Handle {
                                target: Target::Lent(handle),
                                caller,
                            })
                            .map_err(bad)?;
                        (LuaValue::UserData(ud), Received::Plain)
                    }
                    _ => {
                        return Err(ProviderError::new(
                            Status::GUATIAO_ERR_UNSUPPORTED,
                            "an argument this binding does not carry",
                        ));
                    }
                }
            };
            values.push(v);
            received.push(r);
        }
        let results = protected_method(lua, table, described.name(), values, memory)?;
        // Lent trees and buffers back over the caller's.
        for ((arg, d), r) in args.iter_mut().zip(described.args()).zip(&received) {
            match (arg, calls::read_back(lua, d.ty(), r).map_err(bad)?) {
                (Arg::LentValue(v), Some(ReadBack::Tree(new))) => **v = new,
                (Arg::LentMap(m), Some(ReadBack::Tree(new))) => {
                    **m = Map::try_from(new).map_err(|_| {
                        ProviderError::new(
                            Status::GUATIAO_ERR_WRONG_KIND,
                            "a lent map came back as something else",
                        )
                    })?;
                }
                (Arg::LentList(l), Some(ReadBack::Tree(new))) => {
                    **l = List::try_from(new).map_err(|_| {
                        ProviderError::new(
                            Status::GUATIAO_ERR_WRONG_KIND,
                            "a lent list came back as something else",
                        )
                    })?;
                }
                (Arg::Buffer(b), Some(ReadBack::Bytes(new))) => {
                    let n = b.len().min(new.len());
                    b[..n].copy_from_slice(&new[..n]);
                }
                _ => {}
            }
        }
        calls::outcome(lua, described, results)
    })
}

/// Runs `f` with a handle on each lent object, each lend ending when `f`
/// returns.
fn sharing<R>(
    mut lent: Vec<(usize, LentObject<'_>)>,
    mut handles: Vec<(usize, LentHandle)>,
    f: impl FnOnce(Vec<(usize, LentHandle)>) -> R,
) -> R {
    match lent.pop() {
        None => f(handles),
        Some((i, object)) => object.share(move |handle| {
            handles.push((i, handle));
            sharing(lent, handles, f)
        }),
    }
}

/// `table[method](table, values...)` in a protected call: its results, or
/// the refusal its error comes to.
pub(crate) fn protected_method(
    lua: &Lua,
    table: &LuaTable,
    method: &str,
    values: Vec<LuaValue>,
    memory: Option<u64>,
) -> Result<LuaMultiValue, ProviderError> {
    let bad = |e: LuaError| calls::provider_error(&e);
    let f: LuaValue = table.get(method).map_err(bad)?;
    let LuaValue::Function(f) = f else {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            &format!("no method `{method}`"),
        ));
    };
    let mut all = LuaMultiValue::new();
    all.push_back(LuaValue::Function(f));
    all.push_back(LuaValue::Table(table.clone()));
    all.extend(values);
    protected(lua, all, memory)
}

/// `pcall(f, args...)`, so an error keeps the value it was raised with:
/// `all` is the function, then its arguments.
pub(crate) fn protected(
    lua: &Lua,
    all: LuaMultiValue,
    memory: Option<u64>,
) -> Result<LuaMultiValue, ProviderError> {
    let bad = |e: LuaError| calls::provider_error(&e);
    let pcall = pcall(lua).map_err(bad)?;
    let mut results: LuaMultiValue = pcall.call(all).map_err(bad)?;
    match results.pop_front() {
        Some(LuaValue::Boolean(true)) => Ok(results),
        _ => Err(calls::caught(
            &results.pop_front().unwrap_or(LuaValue::Nil),
            memory,
        )),
    }
}

/// The base library's `pcall`, kept before any library runs.
#[derive(Clone)]
struct Pcall(LuaFunction);

fn pcall(lua: &Lua) -> LuaResult<LuaFunction> {
    if let Some(p) = lua.app_data_ref::<Pcall>() {
        return Ok(p.0.clone());
    }
    let p: LuaFunction = lua.globals().raw_get("pcall")?;
    lua.set_app_data(Pcall(p.clone()));
    Ok(p)
}

/// Keeps `pcall` before code that could replace the global runs.
#[cfg(feature = "source")]
pub(crate) fn keep_pcall(lua: &Lua) -> LuaResult<()> {
    pcall(lua).map(drop)
}

/// `table` as an object of `kind`, served on this thread.
pub(crate) fn object(lua: &Lua, kind: &str, table: LuaTable) -> LuaResult<RawObject> {
    let described = calls::kind(lua, kind)?;
    if !described.object() {
        return Err(LuaError::runtime(format!("`{kind}` is not an object kind")));
    }
    let mut implemented = Vec::new();
    for slot in described.slots() {
        let has = matches!(table.get::<LuaValue>(slot.name())?, LuaValue::Function(_));
        if !has && slot.required() {
            return Err(LuaError::runtime(format!(
                "the table has no `{}`, which the `{kind}` kind requires",
                slot.name()
            )));
        }
        if has {
            implemented.push(slot.name().to_string());
        }
    }
    let key = (
        kind.to_string(),
        described.floor_hash(),
        implemented.join(","),
    );
    let served = SERVED.with(|s| -> LuaResult<Arc<ServedTable>> {
        let mut s = s.borrow_mut();
        if let Some(t) = s.tables.get(&key) {
            return Ok(Arc::clone(t));
        }
        let made = Arc::new(
            ServedTable::new(
                Arc::clone(&described),
                |slot| implemented.iter().any(|n| n == slot.name()),
                Arc::new(LuaServe {
                    thread: thread::current().id(),
                }),
            )
            .map_err(|e| LuaError::runtime(e.to_string()))?,
        );
        // An object may be destroyed after this thread ends, by whoever
        // holds it then: its table stays for the process.
        core::mem::forget(Arc::clone(&made));
        s.tables.insert(key, Arc::clone(&made));
        Ok(made)
    })?;
    let ctx = SERVED.with(|s| {
        let mut s = s.borrow_mut();
        s.next += 1;
        let ctx = s.next;
        s.objects.insert(ctx, (lua.weak(), table));
        ctx
    });
    served
        .object(ctx)
        .ok_or_else(|| LuaError::runtime(format!("`{kind}` is not an object kind")))
}
