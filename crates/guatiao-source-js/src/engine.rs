// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! QuickJS as a guatiao-bridge engine: a library is an ES module whose
//! exported `Provider` classes are its providers, its manifest a
//! `*.guatiao-js.toml`, served through tables the bridge builds from the
//! kinds' descriptions.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use guatiao::library::ProviderError;
use guatiao::value::convert::TryAsRef;
use guatiao::{Alloc, Map, Status};
use guatiao_bridge::{
    BridgeError, Call, Config, Engine, Manifest, ManifestFormat, Opened, Provided, Reply, Target,
    ThreadModel,
};
use rquickjs::function::Args;
use rquickjs::{
    Constructor, Context, Ctx, Function, Module, Object, Persistent, Runtime, Symbol, Value as Js,
};

use crate::errors::{self, Words};
use crate::frame::{Entry, Frames};
use crate::host::{self, Shared};
use crate::limits::Meter;
use crate::modules::{self, Load, Resolve, Roots};
use crate::prelude::{self, Core};
use crate::served::{self, Outcome, ReadBack};
use crate::values;

/// One library opened: its context, its providers by id, and the objects
/// its slots answered, by the id each was given.
struct Library {
    providers: BTreeMap<String, Persistent<Object<'static>>>,
    objects: RefCell<BTreeMap<u64, Persistent<Object<'static>>>>,
    entry: Entry,
}

/// What a library's module declares: the providers it offers, and each
/// one's instance by id.
type Declared = (Vec<Provided>, BTreeMap<String, Persistent<Object<'static>>>);

/// The engine: one QuickJS runtime, on the bridge's thread for the life of
/// the source. Not `Send`.
pub struct JsEngine {
    shared: Rc<Shared>,
    meter: Meter,
    roots: Roots,
    frames: Frames,
    libraries: RefCell<BTreeMap<u64, Rc<Library>>>,
    next: Cell<u64>,
    /// Objects released while a frame was open: their cleanup is script,
    /// which may not run inside another script's frame, so it waits.
    deferred: RefCell<Vec<(u64, u64)>>,
    /// A collection a closed library asked for while a frame was open.
    collect_due: Cell<bool>,
    /// Last, so that it outlives everything that holds a value in it.
    runtime: Runtime,
}

fn words(error: &ProviderError) -> String {
    error.message().to_string()
}

/// A class's name as a provider's default name is made from it:
/// `HelloWorld` is `hello_world`.
fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut before: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase()
            && before.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            out.push('_');
        }
        out.extend(c.to_lowercase());
        before = Some(c);
    }
    out
}

/// The text field `key` of a map.
fn text_field<'a>(map: &'a Map, key: &str) -> Option<&'a str> {
    map.get(key).and_then(TryAsRef::<str>::try_as_ref)
}

/// `package.json` in `dir`, when there is one and it is a JSON map.
fn package_in(dir: &Path) -> Result<Option<Map>, BridgeError> {
    let file = dir.join("package.json");
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(_) if !file.exists() => return Ok(None),
        Err(e) => return Err(BridgeError::engine(format!("{}: {e}", file.display()))),
    };
    let value = guatiao_serde::text::json::from_str(
        &text,
        Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))?;
    Map::try_from(value)
        .map(Some)
        .map_err(|_| BridgeError::engine(format!("{} is not a JSON object", file.display())))
}

/// The file a package's `exports` (or `main`) names for its entry, as
/// Node picks it for an `import`: a string, `.` of a map of subpaths, or
/// the first condition, in the order the package lists them, that an
/// import meets (`import`, `node`, `module-sync`, `default`; never
/// `require`).
fn entry_of(package: &Map) -> Option<String> {
    fn pick(target: &guatiao::Value) -> Option<String> {
        if let Some(text) = TryAsRef::<str>::try_as_ref(target) {
            return Some(text.to_string());
        }
        let conditions: &Map = TryAsRef::try_as_ref(target)?;
        conditions
            .iter()
            .filter(|c| ["import", "node", "module-sync", "default"].contains(&c.key()))
            .find_map(|c| pick(c.value()))
    }
    match package.get("exports") {
        Some(exports) if exports.tag() != Ok(guatiao::Tag::GUATIAO_NULL) => {
            if let Some(map) = TryAsRef::<Map>::try_as_ref(exports) {
                if let Some(dot) = map.get(".") {
                    return pick(dot);
                }
                if map.keys().any(|k| k.starts_with('.')) {
                    return None;
                }
            }
            pick(exports)
        }
        _ => Some(
            text_field(package, "main")
                .unwrap_or("index.js")
                .to_string(),
        ),
    }
}

/// The module file a manifest names: its `main`, else its package's
/// entry, else `index.js`, beside the manifest.
fn entry_file(manifest: &Manifest) -> Result<PathBuf, BridgeError> {
    let dir = manifest.path().parent().unwrap_or(Path::new("."));
    let named = match text_field(manifest.table(), "main") {
        Some(main) => main.to_string(),
        None => match package_in(dir)? {
            Some(package) => entry_of(&package).ok_or_else(|| {
                BridgeError::engine(format!(
                    "{}: the package's `exports` name no entry for `.`: give the manifest a `main`",
                    manifest.path().display()
                ))
            })?,
            None => "index.js".to_string(),
        },
    };
    let base = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    Ok(modules::normalize(&base.join(named)))
}

fn escape(e: rquickjs::Error) -> BridgeError {
    BridgeError::engine(e.to_string())
}

impl JsEngine {
    /// The bytes QuickJS holds for every library open: what a host watching
    /// a source (`BridgeSource::run`) compares across refreshes.
    /// Asked from inside a call, when the runtime cannot be looked at, it
    /// is 0.
    pub fn memory_used(&self) -> u64 {
        if self.frames.inside() {
            return 0;
        }
        u64::try_from(self.runtime.memory_usage().malloc_size).unwrap_or(0)
    }

    /// Runs `f` in `entry`'s context. When it is the outermost frame that
    /// is about to return, the promise reactions `f` queued run first,
    /// whatever `f` came to: a reaction may not run in the middle of
    /// another script's synchronous code, nor in a later call. Always
    /// inside a [`Meter::run`], so that they are counted.
    fn enter<R>(&self, entry: &Entry, f: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
        if !self.frames.inside() {
            self.meter.set_stack_top_here();
        }
        self.frames.enter(entry, |ctx| {
            let answer = f(ctx.clone());
            if self.frames.depth() == 1 {
                self.run_jobs(&ctx);
            }
            answer
        })
    }

    /// Settles what waited for no frame to be open, once the call that
    /// opened the last one has been accounted for.
    fn leave(&self) {
        if !self.frames.inside() {
            self.settle();
        }
    }

    /// What waited for no frame to be open. Destroying an object can run
    /// the cleanup of one a library serves, and releasing one does; so
    /// can a collection, by freeing handles, which queues more.
    fn settle(&self) {
        loop {
            let dead = self.shared.take_graveyard();
            let released = core::mem::take(&mut *self.deferred.borrow_mut());
            let collect = self.collect_due.replace(false);
            if dead.is_empty() && released.is_empty() && !collect {
                break;
            }
            drop(dead);
            for (library, object) in released {
                self.release_now(library, object);
            }
            if collect {
                self.runtime.run_gc();
            }
        }
    }

    fn release_now(&self, handle: u64, object: u64) {
        let Ok(lib) = self.library(handle) else {
            return;
        };
        let Some(gone) = lib.objects.borrow_mut().remove(&object) else {
            return;
        };
        let _ = self.meter.run::<(), BridgeError>(|| {
            self.enter(&lib.entry, |ctx| {
                let dispose = || -> rquickjs::Result<()> {
                    let object = gone.restore(&ctx)?;
                    let symbol: Object<'_> = ctx.globals().get("Symbol")?;
                    if let Some(key) = symbol.get::<_, Option<Symbol<'_>>>("dispose")?
                        && let Some(f) = object.get::<_, Option<Function<'_>>>(key.as_atom())?
                    {
                        f.call::<_, Js<'_>>((rquickjs::function::This(object.clone()),))?;
                    }
                    Ok(())
                };
                if dispose().is_err() {
                    let _ = ctx.catch();
                }
                Ok(())
            })
        });
    }

    fn library(&self, handle: u64) -> Result<Rc<Library>, ProviderError> {
        self.libraries
            .borrow()
            .get(&handle)
            .cloned()
            .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_GONE, "the library was closed"))
    }

    fn words(&self) -> Words {
        self.shared.words
    }

    /// Runs the promise reactions that are queued until none are. The
    /// queue is the runtime's and QuickJS cannot empty it, so once a limit
    /// has stopped the call what is left must die with it: the jobs are
    /// taken off on a runtime with no stack, so each fails at its first
    /// call without running script, and no memory, so none can queue
    /// another. The queue then only shrinks, a job at a time, at a cost
    /// that does not depend on what the jobs would have run.
    fn run_jobs(&self, ctx: &Ctx<'_>) {
        let mut starved = None;
        loop {
            if starved.is_none() && self.meter.stopped() {
                starved = Some(self.meter.starve());
            }
            if !ctx.execute_pending_job() {
                break;
            }
            if let Some(starved) = &starved {
                starved.again();
            }
        }
    }

    /// Calls the slot `call` names on `receiver`: arguments converted for
    /// the method, lent ones read back, the answer made the slot's.
    fn invoke<'js>(
        &self,
        lib: &Library,
        ctx: &Ctx<'js>,
        receiver: &Object<'js>,
        call: &mut Call,
        registered: &Cell<Option<u64>>,
    ) -> Result<Reply, ProviderError> {
        let words = self.words();
        let caught = |e: rquickjs::Error| errors::caught(ctx, None, &e, words);
        let core = prelude::core(ctx, self.shared.bounds).map_err(caught)?;
        let slot = call.slot_description().clone();
        let method = host::method_name(slot.name());
        let function: Js<'js> = receiver.get(method.as_str()).map_err(caught)?;
        let Some(function) = function.into_function() else {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                &format!("`{method}` is not implemented"),
            ));
        };

        let mut arguments = Args::new(ctx.clone(), slot.args().len());
        arguments.this(receiver.clone()).map_err(caught)?;
        let mut received = Vec::with_capacity(slot.args().len());
        for (arg, described) in call.args.iter_mut().zip(slot.args()) {
            let (value, back) = served::argument(ctx, &core, &self.shared, described.ty(), arg)?;
            arguments.push_arg(value).map_err(caught)?;
            received.push(back);
        }
        let returned: Js<'js> = arguments
            .apply(&function)
            .map_err(|e| errors::caught(ctx, Some(&core), &e, words))?;

        for (arg, back) in call.args.iter_mut().zip(&received) {
            match served::read_back(ctx, &core, &self.shared, back)? {
                Some(ReadBack::Tree(v)) => *arg = guatiao_bridge::CallArg::Lent(v),
                Some(ReadBack::Bytes(b)) => *arg = guatiao_bridge::CallArg::Buffer(b),
                None => {}
            }
        }
        Ok(
            match served::outcome(ctx, &core, &self.shared, &slot, returned)? {
                Outcome::Nothing => Reply::Nothing,
                Outcome::Value(v) => Reply::Value(v),
                Outcome::Handed(o) => Reply::Handed(o),
                Outcome::Served(kind, object) => {
                    let id = self.next.get() + 1;
                    self.next.set(id);
                    lib.objects
                        .borrow_mut()
                        .insert(id, Persistent::save(ctx, object));
                    registered.set(Some(id));
                    Reply::Object { kind, id }
                }
            },
        )
    }

    /// Runs the entry module and declares the providers it exports.
    fn load<'js>(
        &self,
        ctx: &Ctx<'js>,
        manifest: &Manifest,
        entry: &Path,
    ) -> Result<Declared, BridgeError> {
        let id = manifest.id();
        let words = self.words();
        let failed = |e: rquickjs::Error| {
            BridgeError::engine(format!(
                "{id}: {}",
                words_of(errors::caught(ctx, None, &e, words))
            ))
        };
        prelude::install(ctx).map_err(failed)?;
        let core = prelude::core(ctx, self.shared.bounds).map_err(failed)?;
        let promise = Module::import(ctx, entry.display().to_string()).map_err(failed)?;
        // The module's jobs run until it settles, and no further once a
        // limit stopped the opening: what is left is ended when the frame
        // returns, as a call's is.
        let settled = loop {
            if let Some(settled) = promise.result::<Object<'js>>() {
                break settled;
            }
            if self.meter.stopped() {
                return Err(BridgeError::engine(format!(
                    "{id}: {} was stopped while it loaded",
                    entry.display()
                )));
            }
            if !ctx.execute_pending_job() {
                return Err(BridgeError::engine(format!(
                    "{id}: {} never finishes loading: it awaits something nothing here provides",
                    entry.display()
                )));
            }
        };
        let namespace = settled.map_err(|e| {
            BridgeError::engine(format!(
                "{id}: {}",
                words_of(errors::caught(ctx, Some(&core), &e, words))
            ))
        })?;

        let mut classes: Vec<(String, Constructor<'js>)> = Vec::new();
        for entry in namespace.props::<String, Js<'js>>() {
            let (name, value) = entry.map_err(failed)?;
            if let Some(class) = provider_class(&core, &value)
                && !classes
                    .iter()
                    .any(|(_, c)| c.as_value() == class.as_value())
            {
                classes.push((name, class));
            }
        }
        if classes.is_empty() {
            return Err(BridgeError::engine(format!(
                "{id}: {} exports no Provider subclass",
                entry.display()
            )));
        }

        let mut offered = Vec::new();
        let mut providers = BTreeMap::new();
        for (_, class) in classes {
            let Some((declared, instance)) = self.declare(ctx, manifest, &core, &class)? else {
                continue;
            };
            let key = declared.id().to_string();
            if providers.contains_key(&key) {
                return Err(BridgeError::engine(format!(
                    "two providers of {id} share an id: {key}"
                )));
            }
            offered.push(declared);
            providers.insert(key, Persistent::save(ctx, instance));
        }
        Ok((offered, providers))
    }

    /// What a provider class declares, checked, and its one instance;
    /// `None` when it says it cannot run here.
    fn declare<'js>(
        &self,
        ctx: &Ctx<'js>,
        manifest: &Manifest,
        core: &Core<'js>,
        class: &Constructor<'js>,
    ) -> Result<Option<(Provided, Object<'js>)>, BridgeError> {
        let id = manifest.id();
        let words = self.words();
        let failed = |e: rquickjs::Error| {
            BridgeError::engine(format!(
                "{id}: {}",
                words_of(errors::caught(ctx, Some(core), &e, words))
            ))
        };
        let statics: &Object<'js> = class;
        let text = |key: &str| -> Result<Option<String>, BridgeError> {
            let v: Js<'js> = statics.get(key).map_err(failed)?;
            Ok(v.as_string().and_then(|s| s.to_string().ok()))
        };
        let class_name = text("name")?.unwrap_or_default();
        let defined = |key: &str| -> Result<bool, BridgeError> {
            let v: Js<'js> = statics.get(key).map_err(failed)?;
            Ok(!v.is_undefined() && !v.is_null())
        };
        for key in ["config", "configSchema"] {
            if defined(key)? {
                return Err(BridgeError::engine(format!(
                    "{id}: {class_name} declares `{key}`: a provider built from a configuration is not served here"
                )));
            }
        }

        // `static available()`: a reason it cannot run is the provider left out.
        let available: Function<'js> = statics.get("available").map_err(failed)?;
        let base: Function<'js> = core.provider.get("available").map_err(failed)?;
        if available.as_value() != base.as_value() {
            let why: Js<'js> = available
                .call((rquickjs::function::This(class.clone()),))
                .map_err(failed)?;
            if why.as_string().is_some() {
                return Ok(None);
            }
        }

        let named = text("named")?.unwrap_or_else(|| snake(&class_name));
        let provider_id = text("id")?.unwrap_or_else(|| format!("{id}_{named}"));
        let kinds: Js<'js> = statics.get("kinds").map_err(failed)?;
        let listed: Vec<Js<'js>> = match kinds.as_array() {
            Some(array) => (0..array.len())
                .map(|i| array.get::<Js<'js>>(i))
                .collect::<Result<_, _>>()
                .map_err(failed)?,
            None => Vec::new(),
        };
        if listed.is_empty() {
            return Err(BridgeError::engine(format!(
                "{id}: {class_name} lists no kind in static kinds"
            )));
        }
        let prototype: Object<'js> = statics.get("prototype").map_err(failed)?;
        let mut declared = Provided::new(&provider_id);
        if let Some(shown) = text("displayName")? {
            declared = declared.name(&shown);
        }
        for kind in &listed {
            let (name, floor_hash, is_object) = kind_of(kind)
                .map_err(|e| BridgeError::engine(format!("{id}: {class_name}: {e}")))?;
            if is_object {
                return Err(BridgeError::engine(format!(
                    "{id}: {class_name}: {name} is an object kind; a provider serves provider kinds"
                )));
            }
            if !manifest.kinds().contains(&name) {
                return Err(BridgeError::engine(format!(
                    "{id}: the provider `{named}` serves `{name}`, which the manifest does not list"
                )));
            }
            let described = self
                .shared
                .kind(&name)
                .map_err(|_| BridgeError::UnknownKind(name.clone()))?;
            if let Some(hash) = floor_hash
                && hash != described.floor_hash()
            {
                return Err(BridgeError::engine(format!(
                    "{id}: {class_name}: the kind `{name}` its module describes (floor hash {hash}) is not the one the host gave ({})",
                    described.floor_hash()
                )));
            }
            let lacking = served::missing(&described, &prototype);
            if !lacking.is_empty() {
                return Err(BridgeError::engine(format!(
                    "{id}: {class_name} does not implement {}, which the {name} kind requires",
                    lacking.join(", ")
                )));
            }
            let implemented = described.slots().iter().filter(|slot| {
                prototype
                    .get::<_, Js<'js>>(host::method_name(slot.name()))
                    .is_ok_and(|f| f.is_function())
            });
            declared = declared.serves(&name, implemented.map(|s| s.name().to_string()));
        }
        let meta: Js<'js> = statics.get("meta").map_err(failed)?;
        if !meta.is_undefined() && !meta.is_null() {
            let value = values::to_value(core, &meta)
                .map_err(|e| BridgeError::engine(format!("{id}: {class_name}: meta: {e}")))?;
            let map = Map::try_from(value).map_err(|_| {
                BridgeError::engine(format!("{id}: {class_name}: meta is a plain object"))
            })?;
            declared = declared.meta(map);
        }
        let instance: Object<'js> = class.construct(()).map_err(failed)?;
        Ok(Some((declared, instance)))
    }
}

fn words_of(error: ProviderError) -> String {
    words(&error)
}

/// A value exported from a module, if it is a class extending `Provider`.
fn provider_class<'js>(core: &Core<'js>, value: &Js<'js>) -> Option<Constructor<'js>> {
    let class = value.clone().into_constructor()?;
    if class.as_value() == core.provider.as_value() {
        return None;
    }
    let prototype: Object<'js> = class.get("prototype").ok()?;
    prototype.is_instance_of(&core.provider).then_some(class)
}

/// A kind a class lists: its name, the floor hash its module's description
/// gave it, and whether it is an object kind.
fn kind_of(kind: &Js<'_>) -> Result<(String, Option<u32>, bool), String> {
    let Some(object) = kind.as_object() else {
        return Err(format!(
            "{} is not a kind: load one with kindsOf()",
            kind.type_name()
        ));
    };
    let name: Option<String> = object.get("name").ok().flatten();
    let name = match name.filter(|n| !n.is_empty()) {
        Some(name) => name,
        None => {
            return Err("a kind in static kinds names none: load one with kindsOf()".to_string());
        }
    };
    let floor_hash: Option<u32> = object.get("floorHash").ok().flatten();
    let is_object: bool = object.get("isObject").unwrap_or(false);
    Ok((name, floor_hash, is_object))
}

impl Engine for JsEngine {
    type Library = u64;

    /// A QuickJS runtime is not `Send`: it lives on the bridge's thread.
    const THREADS: ThreadModel = ThreadModel::Pinned;

    const MANIFESTS: ManifestFormat = ManifestFormat {
        suffix: ".guatiao-js.toml",
        id_key: "name",
        code_extensions: &["js", "mjs", "json"],
        skip_dirs: &["node_modules"],
    };

    /// A QuickJS runtime under the configuration's limits, and the kinds
    /// it names.
    fn start(config: &Config) -> Result<JsEngine, BridgeError> {
        let unavailable =
            |e: rquickjs::Error| BridgeError::Unavailable(format!("QuickJS did not start: {e}"));
        let runtime = Runtime::new().map_err(unavailable)?;
        let roots = Roots::default();
        let limits = *config.limits();
        let meter = Meter::install(&runtime, limits).map_err(unavailable)?;
        runtime.set_loader(
            Resolve {
                roots: roots.clone(),
                stopped: meter.stopped_flag(),
            },
            Load {
                stopped: meter.stopped_flag(),
            },
        );
        let mut kinds = guatiao_bridge::Kinds::default();
        for path in config.kinds() {
            kinds.load(path)?;
        }
        Ok(JsEngine {
            shared: Shared::new(
                kinds,
                Words {
                    memory: limits.memory,
                    depth: limits.depth,
                },
                meter.stopped_flag(),
            ),
            meter,
            roots,
            frames: Frames::default(),
            libraries: RefCell::new(BTreeMap::new()),
            next: Cell::new(0),
            deferred: RefCell::new(Vec::new()),
            collect_due: Cell::new(false),
            runtime,
        })
    }

    /// The version of the package beside the manifest, which is not run.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        let dir = manifest.path().parent().unwrap_or(Path::new("."));
        package_in(dir)?
            .and_then(|package| text_field(&package, "version").map(str::to_string))
            .ok_or_else(|| {
                BridgeError::engine(format!(
                    "{}: no `version`, and {} states none: give the manifest one",
                    manifest.path().display(),
                    dir.join("package.json").display()
                ))
            })
    }

    /// Runs the entry module afresh in a context of its own and reads the
    /// providers it exports.
    fn open(&self, manifest: &Manifest, _version: &str) -> Result<Opened<u64>, BridgeError> {
        let opened = self.open_inside(manifest);
        self.leave();
        opened
    }

    fn call(&self, handle: &u64, call: &mut Call) -> Result<Reply, ProviderError> {
        let answer = self.call_inside(*handle, call);
        self.leave();
        answer
    }

    /// The object goes; its `Symbol.dispose`, if it has one, runs first,
    /// when no script is running.
    fn release(&self, handle: &u64, object: u64) {
        if self.frames.inside() {
            self.deferred.borrow_mut().push((*handle, object));
        } else {
            self.release_now(*handle, object);
            self.leave();
        }
    }

    fn close(&self, handle: u64) {
        let Some(lib) = self.libraries.borrow_mut().remove(&handle) else {
            return;
        };
        self.forget_roots(&lib.entry);
        drop(lib);
        self.collect_due.set(true);
        self.leave();
    }
}

impl JsEngine {
    fn forget_roots(&self, entry: &Entry) {
        let _ = self.meter.run::<(), BridgeError>(|| {
            self.enter(entry, |ctx| self.roots.forget(&ctx));
            Ok(())
        });
    }

    fn open_inside(&self, manifest: &Manifest) -> Result<Opened<u64>, BridgeError> {
        let id = manifest.id();
        if self.frames.inside() {
            return Err(BridgeError::Unavailable(
                "a library is not opened from inside a call into another".to_string(),
            ));
        }
        let paths: Vec<PathBuf> = manifest
            .paths()
            .iter()
            .map(|p| modules::normalize(Path::new(p)))
            .collect();
        let entry = entry_file(manifest)?;
        if !paths.iter().any(|p| entry.starts_with(p)) {
            return Err(BridgeError::engine(format!(
                "{id}: {} is outside the manifest's paths",
                entry.display()
            )));
        }
        if !entry.is_file() {
            return Err(BridgeError::engine(format!(
                "{id}: {} is not a file",
                entry.display()
            )));
        }
        // Making a context runs QuickJS's own setup, measured from here.
        self.meter.set_stack_top_here();
        let context = Context::full(&self.runtime).map_err(escape)?;
        let raw = context.with(|ctx| ctx.as_raw());
        let entry_point = Entry::new(context, raw);
        let loaded = self.meter.run(|| {
            self.enter(&entry_point, |ctx| {
                self.roots.allow(&ctx, paths);
                let loaded = self.load(&ctx, manifest, &entry);
                if loaded.is_err() {
                    self.roots.forget(&ctx);
                }
                loaded
            })
        });
        let (offered, providers) = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                self.forget_roots(&entry_point);
                drop(entry_point);
                self.collect_due.set(true);
                self.settle();
                return Err(e);
            }
        };
        let handle = self.next.get() + 1;
        self.next.set(handle);
        self.libraries.borrow_mut().insert(
            handle,
            Rc::new(Library {
                providers,
                objects: RefCell::new(BTreeMap::new()),
                entry: entry_point,
            }),
        );
        Ok(Opened::Described {
            library: handle,
            providers: offered,
        })
    }

    fn call_inside(&self, handle: u64, call: &mut Call) -> Result<Reply, ProviderError> {
        let lib = self.library(handle)?;
        let receiver = match call.target() {
            Target::Provider(id) => lib.providers.get(id).cloned(),
            Target::Object(id) => lib.objects.borrow().get(id).cloned(),
            _ => None,
        }
        .ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                &format!("nothing answers `{}`", call.method()),
            )
        })?;
        let registered = Cell::new(None);
        let answer = self.meter.run(|| {
            self.enter(&lib.entry, |ctx| {
                let receiver = receiver
                    .restore(&ctx)
                    .map_err(|e| errors::caught(&ctx, None, &e, self.words()))?;
                self.invoke(&lib, &ctx, &receiver, call, &registered)
            })
        });
        if answer.is_err()
            && let Some(id) = registered.get()
        {
            // An object the stopped call answered: nobody was told of it.
            drop(lib.objects.borrow_mut().remove(&id));
        }
        answer
    }
}

impl Drop for JsEngine {
    /// The libraries go, and what their handles held is destroyed, before
    /// the runtime does.
    fn drop(&mut self) {
        self.libraries.borrow_mut().clear();
        self.deferred.borrow_mut().clear();
        self.collect_due.set(true);
        self.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(json: &str) -> Map {
        let value = guatiao_serde::text::json::from_str(
            json,
            Alloc::rust(),
            guatiao_serde::Presentation::new(),
        )
        .expect("JSON");
        Map::try_from(value).expect("a map")
    }

    #[test]
    fn an_entry_is_picked_as_node_picks_it_for_an_import() {
        let entry = |json: &str| entry_of(&package(json));
        assert_eq!(entry(r#"{"exports": "./a.js"}"#).as_deref(), Some("./a.js"));
        assert_eq!(entry(r#"{"main": "m.js"}"#).as_deref(), Some("m.js"));
        assert_eq!(entry("{}").as_deref(), Some("index.js"));
        assert_eq!(
            entry(r#"{"exports": {".": "./dot.js", "./x": "./x.js"}}"#).as_deref(),
            Some("./dot.js")
        );
        assert_eq!(entry(r#"{"exports": {"./x": "./x.js"}}"#), None);
        // The first condition an import meets, in the package's order.
        assert_eq!(
            entry(r#"{"exports": {"require": "./r.js", "default": "./d.js", "import": "./i.js"}}"#)
                .as_deref(),
            Some("./d.js")
        );
        assert_eq!(
            entry(r#"{"exports": {".": {"types": "./t.d.ts", "node": {"require": "./r.js", "import": "./ni.js"}, "import": "./i.js"}}}"#)
                .as_deref(),
            Some("./ni.js")
        );
        assert_eq!(entry(r#"{"exports": {"require": "./r.js"}}"#), None);
    }
}
