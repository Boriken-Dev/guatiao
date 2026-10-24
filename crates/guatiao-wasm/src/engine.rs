// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! wasmtime as a guatiao-bridge engine: a library is a WebAssembly module
//! speaking guatiao ABI 1, its manifest a `*.guatiao-wasm.toml`, each
//! opened library an instance in a store of its own, called on the
//! caller's thread behind that library's lock.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::time::Duration;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError, mpsc};
use std::thread::{self, JoinHandle, ThreadId};

use guatiao::library::ProviderError;
use guatiao::library::kind::{AnswerType, ArgType};
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::value::wire;
use guatiao::{Status, Value};
use guatiao_bridge::{
    BridgeError, Call, CallArg, Config, Engine, Kinds, Limits, Manifest, ManifestFormat, Opened,
    Provided, Reply, Target, ThreadModel,
};
use wasmtime::{ExternType, FuncType, Linker, Module};

use crate::abi::{self, export};
use crate::instance::{self, Held, Live, Shared, State, Stopped, Writable};
use crate::{section, wasi};

/// Increments the engine's epoch every millisecond while a time limit is
/// set; stops when dropped.
struct Ticker {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: wasmtime::Engine) -> Result<Ticker, BridgeError> {
        let (stop, stopped) = mpsc::channel::<()>();
        let thread = thread::Builder::new()
            .name("guatiao-wasm-epoch".to_string())
            .spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) =
                    stopped.recv_timeout(Duration::from_millis(1))
                {
                    engine.increment_epoch();
                }
            })
            .map_err(|e| {
                BridgeError::Unavailable(format!("the epoch thread did not start: {e}"))
            })?;
        Ok(Ticker {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The engine: one wasmtime engine and linker for the source, the kinds it
/// was given, and its limits. `Send + Sync`: every library locks its own
/// store.
pub struct WasmEngine {
    engine: wasmtime::Engine,
    linker: Linker<State>,
    limits: Limits,
    shared: Arc<Shared>,
    _ticker: Option<Ticker>,
}

/// One opened library: its compiled module, its providers in the order it
/// declared them, and its instance behind a lock.
pub struct WasmLibrary {
    id: String,
    module: Module,
    providers: Vec<String>,
    slot: Mutex<Slot>,
    inside: Mutex<Option<ThreadId>>,
    deferred: Mutex<Vec<u64>>,
}

impl core::fmt::Debug for WasmLibrary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WasmLibrary")
            .field("id", &self.id)
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

/// The instance, made again after a stop, and the objects it answered:
/// the host's id for each, and the guest's.
struct Slot {
    live: Option<Live>,
    objects: BTreeMap<u64, i64>,
    next_object: u64,
}

impl Slot {
    /// After a stop the guest's state is unknown: its instance goes, and
    /// with it every object it answered and every host object it held.
    fn discard(&mut self) {
        self.live = None;
        self.objects.clear();
    }
}

/// A library entered by this thread: its lock, and the thread named as
/// inside it until dropped.
struct Entered<'a> {
    slot: MutexGuard<'a, Slot>,
    inside: &'a Mutex<Option<ThreadId>>,
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        *self.inside.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn busy() -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_BUSY,
        "the library is in a call on this thread already: what it called cannot call it back",
    )
}

impl WasmLibrary {
    /// Takes the library's lock: waits for another thread's call, refuses
    /// one on this thread (`None`).
    fn enter(&self) -> Option<Entered<'_>> {
        let me = thread::current().id();
        let guard = match self.slot.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                if *self.inside.lock().unwrap_or_else(PoisonError::into_inner) == Some(me) {
                    return None;
                }
                self.slot.lock().unwrap_or_else(|poisoned| {
                    let mut slot = poisoned.into_inner();
                    slot.discard();
                    slot
                })
            }
            Err(TryLockError::Poisoned(poisoned)) => {
                let mut slot = poisoned.into_inner();
                slot.discard();
                slot
            }
        };
        *self.inside.lock().unwrap_or_else(PoisonError::into_inner) = Some(me);
        Some(Entered {
            slot: guard,
            inside: &self.inside,
        })
    }

    /// Its id.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Where a manifest's module is: `<module>.wasm` in the first of its
/// paths that has one.
fn module_file(manifest: &Manifest) -> Result<PathBuf, BridgeError> {
    let name = format!("{}.wasm", manifest.id());
    manifest
        .paths()
        .iter()
        .map(|dir| PathBuf::from(dir).join(&name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            BridgeError::engine(format!(
                "{name} is not in the manifest's paths ({})",
                manifest.paths().join(", ")
            ))
        })
}

fn read(file: &PathBuf) -> Result<Vec<u8>, BridgeError> {
    std::fs::read(file).map_err(|e| BridgeError::engine(format!("{}: {e}", file.display())))
}

/// The version a module states in its custom section, if it states one.
fn stated_version(bytes: &[u8], file: &std::path::Path) -> Result<Option<String>, BridgeError> {
    match section::custom(bytes, abi::VERSION_SECTION) {
        Ok(None) => Ok(None),
        Ok(Some(text)) => core::str::from_utf8(text)
            .map(|t| Some(t.trim().to_string()))
            .map_err(|_| {
                BridgeError::engine(format!(
                    "{}: its `{}` section is not UTF-8",
                    file.display(),
                    abi::VERSION_SECTION
                ))
            }),
        Err(section::SectionError::NotAModule) => Err(BridgeError::engine(format!(
            "{} is not a WebAssembly module (a component, or not wasm at all)",
            file.display()
        ))),
        Err(section::SectionError::Truncated) => Err(BridgeError::engine(format!(
            "{} ends inside a section",
            file.display()
        ))),
    }
}

fn shape(ty: &FuncType) -> String {
    let list = |types: &mut dyn Iterator<Item = wasmtime::ValType>| {
        types.map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
    };
    format!(
        "({}) -> ({})",
        list(&mut ty.params()),
        list(&mut ty.results())
    )
}

/// Checks what a module exports and imports before any of its code runs.
fn check_shape(module: &Module, id: &str) -> Result<(), BridgeError> {
    const WANTED: [(&str, &str, bool); 6] = [
        (export::ABI, "() -> (i32)", true),
        (export::ALLOC, "(i32) -> (i32)", true),
        (export::DECLARE, "() -> (i32)", true),
        (export::CALL, "(i32, i64, i32, i32) -> (i32)", true),
        (export::RELEASE, "(i64) -> ()", false),
        (export::INITIALIZE, "() -> ()", false),
    ];
    let refuse = |why: String| BridgeError::engine(format!("{id}: {why}"));
    let exports: BTreeMap<&str, ExternType> =
        module.exports().map(|e| (e.name(), e.ty())).collect();
    for (name, wanted, required) in WANTED {
        match exports.get(name) {
            Some(ExternType::Func(ty)) if shape(ty) == wanted => {}
            Some(ExternType::Func(ty)) => {
                return Err(refuse(format!(
                    "`{name}` is `{}`, and guatiao ABI {} has it `{wanted}`",
                    shape(ty),
                    abi::VERSION
                )));
            }
            Some(_) => return Err(refuse(format!("`{name}` is not a function"))),
            None if required => {
                return Err(refuse(format!(
                    "it exports no `{name}`: is it built against a guatiao guest library (guatiao ABI {})?",
                    abi::VERSION
                )));
            }
            None => {}
        }
    }
    match exports.get(export::MEMORY) {
        Some(ExternType::Memory(m)) if !m.is_64() && !m.is_shared() => {}
        Some(ExternType::Memory(_)) => {
            return Err(refuse("its memory is 64-bit or shared".to_string()));
        }
        _ => return Err(refuse("it exports no `memory`".to_string())),
    }
    for import in module.imports() {
        let known = match import.module() {
            abi::IMPORT_MODULE => [
                abi::import::ANSWER,
                abi::import::WRITE_BACK,
                abi::import::OBJECT_CALL,
                abi::import::RESULT,
                abi::import::DROP,
            ]
            .contains(&import.name()),
            wasi::MODULE => wasi::NAMES.contains(&import.name()),
            _ => false,
        };
        if !known {
            return Err(refuse(format!(
                "it imports `{}.{}`, which this source does not offer",
                import.module(),
                import.name()
            )));
        }
    }
    Ok(())
}

impl WasmEngine {
    fn stopped(&self, lib: &WasmLibrary, error: &wasmtime::Error, state: &State) -> ProviderError {
        instance::stopped(&lib.id, error, state, &self.limits)
    }

    /// The library's instance, made afresh after a stop.
    fn live<'s>(
        &self,
        lib: &WasmLibrary,
        slot: &'s mut Slot,
    ) -> Result<&'s mut Live, ProviderError> {
        if slot.live.is_none() {
            let live = Live::new(
                &self.engine,
                &self.linker,
                &lib.module,
                &self.limits,
                Arc::clone(&self.shared),
            )
            .map_err(|(Stopped(e), store)| self.stopped(lib, &e, store.data()))?;
            slot.live = Some(live);
        }
        Ok(slot.live.as_mut().expect("made just above"))
    }

    /// Tells the guest one object it answered is gone.
    fn release_in(&self, slot: &mut Slot, object: u64) {
        let Some(guest) = slot.objects.remove(&object) else {
            return;
        };
        if let Some(live) = slot.live.as_mut()
            && live.release(&self.limits, guest).is_err()
        {
            slot.discard();
        }
    }

    /// Releases what was destroyed while the library was busy on this
    /// thread.
    fn settle(&self, lib: &WasmLibrary, slot: &mut Slot) {
        let deferred: Vec<u64> =
            core::mem::take(&mut *lib.deferred.lock().unwrap_or_else(PoisonError::into_inner));
        for object in deferred {
            self.release_in(slot, object);
        }
    }

    /// The providers a guest declared, as the bridge serves them.
    fn provided(
        &self,
        manifest: &Manifest,
        declared: Vec<abi::Declared>,
    ) -> Result<(Vec<Provided>, Vec<String>), BridgeError> {
        let id = manifest.id();
        let mut offered = Vec::with_capacity(declared.len());
        let mut ids = Vec::with_capacity(declared.len());
        for provider in declared {
            let pid = format!("{}_{}", id.replace('.', "_"), provider.id);
            let mut served = Provided::new(&pid).name(&provider.name);
            for (kind, slots) in &provider.serves {
                if !manifest.kinds().iter().any(|k| k == kind) {
                    return Err(BridgeError::engine(format!(
                        "{id}: the provider `{}` serves `{kind}`, which the manifest does not list",
                        provider.id
                    )));
                }
                let described = self
                    .shared
                    .kinds
                    .get(kind)
                    .ok_or_else(|| BridgeError::UnknownKind(kind.clone()))?;
                // A slot the host's description lacks can never be called.
                let implements: Vec<&String> = slots
                    .iter()
                    .filter(|s| described.slot(s).is_some())
                    .collect();
                served = served.serves(kind, implements);
            }
            offered.push(served);
            ids.push(pid);
        }
        Ok((offered, ids))
    }
}

impl Engine for WasmEngine {
    type Library = Arc<WasmLibrary>;

    /// Thread-safe: each library is a store behind its own lock, called
    /// on the caller's thread.
    const THREADS: ThreadModel = ThreadModel::Locked;

    const MANIFESTS: ManifestFormat = ManifestFormat {
        suffix: ".guatiao-wasm.toml",
        id_key: "module",
        code_extensions: &["wasm"],
        skip_dirs: &[],
    };

    fn start(config: &Config) -> Result<WasmEngine, BridgeError> {
        let limits = *config.limits();
        if limits.depth.is_some() {
            return Err(BridgeError::config(
                Limits::KEY,
                "names `depth`, which a WebAssembly guest cannot be held to: its stack is its own, and overflowing it stops the call",
            ));
        }
        let mut wasm = wasmtime::Config::new();
        wasm.consume_fuel(limits.operations.is_some())
            .epoch_interruption(limits.time.is_some())
            .wasm_multi_memory(false)
            .wasm_memory64(false);
        let engine = wasmtime::Engine::new(&wasm)
            .map_err(|e| BridgeError::Unavailable(format!("wasmtime did not start: {e}")))?;
        let mut linker = Linker::new(&engine);
        instance::add_imports(&mut linker)
            .and_then(|()| wasi::add(&mut linker))
            .map_err(|e| BridgeError::Unavailable(format!("the host's imports: {e}")))?;
        let mut kinds = Kinds::new();
        for path in config.kinds() {
            kinds.load(path)?;
        }
        let ticker = match limits.time {
            Some(_) => Some(Ticker::start(engine.clone())?),
            None => None,
        };
        Ok(WasmEngine {
            engine,
            linker,
            limits,
            shared: Arc::new(Shared::new(kinds)),
            _ticker: ticker,
        })
    }

    /// The text of the module's `guatiao.version` section, read without
    /// compiling it.
    fn version_of(&self, manifest: &Manifest) -> Result<String, BridgeError> {
        let file = module_file(manifest)?;
        stated_version(&read(&file)?, &file)?.ok_or_else(|| {
            BridgeError::engine(format!(
                "{} states no version (a `{}` section): give the manifest a `version`",
                file.display(),
                abi::VERSION_SECTION
            ))
        })
    }

    /// Compiles the module, checks its exports and imports, instantiates it
    /// and reads its declaration, each step under the limits.
    fn open(
        &self,
        manifest: &Manifest,
        version: &str,
    ) -> Result<Opened<Arc<WasmLibrary>>, BridgeError> {
        let id = manifest.id();
        let file = module_file(manifest)?;
        let bytes = read(&file)?;
        if let Some(stated) = stated_version(&bytes, &file)?
            && stated != version
        {
            return Err(BridgeError::engine(format!(
                "{id} states version {stated}, and its manifest {version}"
            )));
        }
        let module = Module::new(&self.engine, &bytes).map_err(|e| {
            BridgeError::engine(format!(
                "{id} does not compile: {}",
                e.to_string().lines().next().unwrap_or("")
            ))
        })?;
        check_shape(&module, id)?;
        let engine_error = |e: ProviderError| BridgeError::engine(e.message().to_string());
        let mut live = Live::new(
            &self.engine,
            &self.linker,
            &module,
            &self.limits,
            Arc::clone(&self.shared),
        )
        .map_err(|(Stopped(e), store)| {
            engine_error(instance::stopped(id, &e, store.data(), &self.limits))
        })?;
        let (status, answer) = live.declare(&self.limits).map_err(|Stopped(e)| {
            engine_error(instance::stopped(id, &e, live.store.data(), &self.limits))
        })?;
        if status != 0 {
            return Err(engine_error(instance::refused(id, status, Some(answer))));
        }
        let declared = abi::read_declaration(&answer)
            .map_err(|why| BridgeError::engine(format!("{id}: {why}")))?;
        let (providers, ids) = self.provided(manifest, declared)?;
        Ok(Opened::Described {
            library: Arc::new(WasmLibrary {
                id: id.to_string(),
                module,
                providers: ids,
                slot: Mutex::new(Slot {
                    live: Some(live),
                    objects: BTreeMap::new(),
                    next_object: 0,
                }),
                inside: Mutex::new(None),
                deferred: Mutex::new(Vec::new()),
            }),
            providers,
        })
    }

    fn call(&self, lib: &Arc<WasmLibrary>, call: &mut Call) -> Result<Reply, ProviderError> {
        let mut entered = lib.enter().ok_or_else(busy)?;
        let slot = &mut *entered.slot;
        self.settle(lib, slot);
        let (object, target) = match call.target() {
            Target::Provider(id) => (
                false,
                lib.providers.iter().position(|p| p == id).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_NOT_FOUND,
                        &format!("{} declares no provider `{id}`", lib.id),
                    )
                })? as i64,
            ),
            Target::Object(id) => (
                true,
                *slot.objects.get(id).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_GONE,
                        "the object went with the instance that answered it, which stopped",
                    )
                })?,
            ),
            _ => {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_UNSUPPORTED,
                    "a call this engine does not carry",
                ));
            }
        };
        let live = self.live(lib, slot)?;
        let described = Arc::clone(call.kind());
        let method = call.method().to_string();
        let slot_description = call.slot_description().clone();

        // Objects first: each moves into the store's handles, and crosses
        // as its handle.
        let state = live.store.data_mut();
        state.writable.clear();
        state.written.clear();
        let mut handles = BTreeMap::new();
        let mut lent = Vec::new();
        for (i, (arg, d)) in call
            .args
            .iter_mut()
            .zip(slot_description.args())
            .enumerate()
        {
            let ArgType::Object { kind, .. } = d.ty() else {
                continue;
            };
            let held = match arg {
                CallArg::Object(_) => {
                    let CallArg::Object(object) =
                        core::mem::replace(arg, CallArg::Value(Value::null()))
                    else {
                        unreachable!("matched just above")
                    };
                    Held::Owned(object)
                }
                CallArg::LentObject(h) => Held::Lent(h.clone()),
                _ => continue,
            };
            let handle = state.hold(held, self.shared.caller(kind)?);
            if matches!(arg, CallArg::LentObject(_)) {
                lent.push(handle);
            }
            handles.insert(i, Value::from(handle));
        }
        let mut values: Vec<Cow<'_, Value>> = Vec::with_capacity(call.args.len());
        for (i, arg) in call.args.iter().enumerate() {
            let (value, writable) = match arg {
                _ if handles.contains_key(&i) => (Cow::Owned(handles[&i].clone()), Writable::No),
                CallArg::Value(v) => (Cow::Borrowed(v), Writable::No),
                CallArg::Lent(v) => (Cow::Borrowed(v), Writable::Tree),
                CallArg::Buffer(b) => (Cow::Owned(instance::buffer_value(b)), Writable::Buffer),
                _ => {
                    return Err(ProviderError::new(
                        Status::GUATIAO_ERR_UNSUPPORTED,
                        "an argument this engine does not carry",
                    ));
                }
            };
            values.push(value);
            state.writable.push(writable);
        }
        let request = abi::request(described.name(), &method, values.iter().map(|v| v.as_ref()))
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why))?;
        drop(values);

        let outcome = live.call(&self.limits, object, target, &request);
        let state = live.store.data_mut();
        for handle in lent {
            state.handles.remove(&handle);
        }
        let (status, answer) = match outcome {
            Ok(done) => done,
            Err(Stopped(e)) => {
                let error = self.stopped(lib, &e, live.store.data());
                slot.discard();
                return Err(error);
            }
        };
        if status != 0 {
            return Err(instance::refused(&lib.id, status, answer));
        }
        for (i, value) in core::mem::take(&mut state.written) {
            match &mut call.args[i] {
                CallArg::Lent(v) => *v = value,
                CallArg::Buffer(b) => {
                    let bytes: &[u8] = TryAsRef::try_as_ref(&value).unwrap_or(&[]);
                    *b = bytes.to_vec();
                }
                _ => {}
            }
        }
        let answered = |what: &str| -> Result<Value, ProviderError> {
            let bytes = answer.as_deref().ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &format!("`{method}` answered nothing, and answers {what}"),
                )
            })?;
            wire::decode(bytes).map_err(|e| {
                ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &format!("`{method}` answered bytes that are not a value: {e}"),
                )
            })
        };
        Ok(match slot_description.answer() {
            None => Reply::Nothing,
            Some(AnswerType::Object { kind }) => {
                let guest = i64::from_value(&answered("an object")?).map_err(|_| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_WRONG_KIND,
                        &format!("`{method}` answers an object of `{kind}`: its id, a number"),
                    )
                })?;
                slot.next_object += 1;
                let id = slot.next_object;
                slot.objects.insert(id, guest);
                Reply::Object {
                    kind: kind.clone(),
                    id,
                }
            }
            Some(_) => Reply::Value(answered("a value")?),
        })
    }

    /// Tells the guest its object was destroyed; deferred to the next
    /// call when the library is busy on this thread.
    fn release(&self, lib: &Arc<WasmLibrary>, object: u64) {
        match lib.enter() {
            Some(mut entered) => self.release_in(&mut entered.slot, object),
            None => lib
                .deferred
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(object),
        }
    }

    fn close(&self, lib: Arc<WasmLibrary>) {
        drop(lib);
    }
}
