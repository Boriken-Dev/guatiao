// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! wasmtime as a guatiao-bridge engine: a library is a WebAssembly module
//! speaking guatiao ABI 1, its manifest a `*.guatiao` of type `wasm`, each
//! opened library an instance in a store of its own, called on the
//! caller's thread behind that library's lock.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::cell::Cell;
use core::time::Duration;
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::{self, JoinHandle};

use guatiao::library::ProviderError;
use guatiao::library::kind::{AnswerType, ArgType};
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{Status, Value};
use guatiao_bridge::{
    BridgeError, Call, CallArg, Config, Engine, Held, Kinds, Limits, Manifest, ManifestFormat,
    Opened, Provided, Reply, Target, ThreadModel,
};
use wasmtime::{ExternType, FuncType, Linker, Module};

use crate::abi::{self, export};
use crate::budget::{self, Refused, Undecoded};
use crate::instance::{self, Live, Shared, State, Stopped, Writable, with_stack};
use crate::{meter, section, wasi};

/// The largest module a source reads and compiles.
pub(crate) const MOST_MODULE_BYTES: u64 = 128 << 20;

/// How many guest calls run, and whether the ticker should stop.
#[derive(Default)]
struct Ticks {
    running: usize,
    stop: bool,
}

/// Increments the engine's epoch about every millisecond while a call
/// runs, and sleeps while none does; stops when dropped. Each tick has a
/// running call look at the clock (`instance::time_check`).
struct Ticker {
    ticks: Arc<(Mutex<Ticks>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(engine: wasmtime::Engine) -> Result<Ticker, BridgeError> {
        let ticks = Arc::new((Mutex::new(Ticks::default()), Condvar::new()));
        let shared = Arc::clone(&ticks);
        let thread = thread::Builder::new()
            .name("guatiao-wasm-epoch".to_string())
            .spawn(move || {
                let (lock, wake) = &*shared;
                let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
                while !state.stop {
                    if state.running == 0 {
                        state = wake.wait(state).unwrap_or_else(PoisonError::into_inner);
                        continue;
                    }
                    state = wake
                        .wait_timeout(state, Duration::from_millis(1))
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                    if state.running > 0 {
                        engine.increment_epoch();
                    }
                }
            })
            .map_err(|e| {
                BridgeError::Unavailable(format!("the epoch thread did not start: {e}"))
            })?;
        Ok(Ticker {
            ticks,
            thread: Some(thread),
        })
    }

    /// Counts a call running until the answer drops; the first wakes the
    /// ticker from its idle wait.
    fn running(&self) -> Running<'_> {
        let (lock, wake) = &*self.ticks;
        let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
        state.running += 1;
        if state.running == 1 {
            wake.notify_one();
        }
        Running(self)
    }
}

/// A call the ticker counts.
struct Running<'a>(&'a Ticker);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let (lock, _) = &*self.0.ticks;
        let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
        state.running = state.running.saturating_sub(1);
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.ticks;
        lock.lock().unwrap_or_else(PoisonError::into_inner).stop = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

std::thread_local! {
    /// How many libraries this thread is inside a call of.
    static INSIDE: Cell<usize> = const { Cell::new(0) };
}

/// The engine: one wasmtime engine and linker for the source, the kinds it
/// was given, and its limits. `Send + Sync`: every library locks its own
/// store.
pub struct WasmEngine {
    engine: wasmtime::Engine,
    linker: Linker<State>,
    limits: Limits,
    shared: Arc<Shared>,
    ticker: Option<Ticker>,
}

/// One opened library: its compiled module, its providers in the order it
/// declared them, and its instance behind a lock.
pub struct WasmLibrary {
    id: String,
    module: Module,
    providers: Vec<String>,
    slot: Mutex<Slot>,
    deferred: Mutex<Vec<Gone>>,
}

/// What was let go of while the library was busy on the thread doing it.
#[derive(Clone, Copy)]
enum Gone {
    Object(u64),
    Instance(u64),
}

impl core::fmt::Debug for WasmLibrary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WasmLibrary")
            .field("id", &self.id)
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

/// The instance, made again after a stop, and the objects it answered
/// and the instances it built: the host's id for each, and the guest's.
struct Slot {
    live: Option<Live>,
    objects: BTreeMap<u64, i64>,
    next_object: u64,
    instances: BTreeMap<u64, i64>,
    next_instance: u64,
}

impl Slot {
    /// After a stop the guest's state is unknown: its instance goes, and
    /// with it every object it answered, every instance it built and
    /// every host object it held.
    fn discard(&mut self) {
        self.live = None;
        self.objects.clear();
        self.instances.clear();
    }
}

/// A library this thread is inside a call of: its lock.
struct Entered<'a> {
    slot: MutexGuard<'a, Slot>,
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        INSIDE.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

fn busy() -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_BUSY,
        "the library is in a call, and this thread is inside a call of a library already: waiting could deadlock",
    )
}

impl WasmLibrary {
    /// Takes the library's lock. A thread inside no library's call waits
    /// for it; one inside a call (this library's or another's, through a
    /// host object) is refused (`None`) when it is held, since two such
    /// threads waiting on each other's libraries would wait forever.
    fn enter(&self) -> Option<Entered<'_>> {
        fn recover<'a>(poisoned: PoisonError<MutexGuard<'a, Slot>>) -> MutexGuard<'a, Slot> {
            let mut slot = poisoned.into_inner();
            slot.discard();
            slot
        }
        let guard = match self.slot.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) if INSIDE.with(Cell::get) > 0 => return None,
            Err(TryLockError::WouldBlock) => self.slot.lock().unwrap_or_else(recover),
            Err(TryLockError::Poisoned(poisoned)) => recover(poisoned),
        };
        INSIDE.with(|n| n.set(n.get() + 1));
        Some(Entered { slot: guard })
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

/// A module's bytes, refused unread past [`MOST_MODULE_BYTES`].
fn read(file: &PathBuf) -> Result<Vec<u8>, BridgeError> {
    let failed = |e: std::io::Error| BridgeError::engine(format!("{}: {e}", file.display()));
    let size = std::fs::metadata(file).map_err(failed)?.len();
    if size > MOST_MODULE_BYTES {
        return Err(BridgeError::engine(format!(
            "{} is {size} bytes, larger than a module may be ({MOST_MODULE_BYTES})",
            file.display()
        )));
    }
    std::fs::read(file).map_err(failed)
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

/// Checks what a module exports and imports before any of its code runs;
/// `metered` when the host's rewrite added the charge.
fn check_shape(module: &Module, id: &str, metered: bool) -> Result<(), BridgeError> {
    const WANTED: [(&str, &str, bool); 9] = [
        (export::ABI, "() -> (i32)", true),
        (export::ALLOC, "(i32) -> (i32)", true),
        (export::DECLARE, "() -> (i32)", true),
        (export::CALL, "(i32, i64, i32, i32) -> (i32)", true),
        (export::RELEASE, "(i64) -> ()", false),
        (export::CREATE, "(i64, i32, i32) -> (i32)", false),
        (export::DESTROY, "(i64) -> ()", false),
        (export::AVAILABLE, "(i64) -> (i32)", false),
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
            // The rewrite's own: it refuses a guest importing it first.
            meter::MODULE => metered && import.name() == meter::BULK,
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

    /// Counts a call for the ticker, when there is one.
    fn running(&self) -> Option<Running<'_>> {
        self.ticker.as_ref().map(Ticker::running)
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

    /// Tells the guest one object it answered, or one instance it built,
    /// is gone. One that went with a discarded store reaches no guest.
    fn let_go(&self, slot: &mut Slot, gone: Gone) {
        let guest = match gone {
            Gone::Object(id) => slot.objects.remove(&id),
            Gone::Instance(id) => slot.instances.remove(&id),
        };
        let (Some(guest), Some(live)) = (guest, slot.live.as_mut()) else {
            return;
        };
        let told = match gone {
            Gone::Object(_) => live.release(&self.limits, guest),
            Gone::Instance(_) => live.destroy(&self.limits, guest),
        };
        if told.is_err() {
            slot.discard();
        }
    }

    /// Lets go of what was destroyed while the library was busy on this
    /// thread.
    fn settle(&self, lib: &WasmLibrary, slot: &mut Slot) {
        let deferred: Vec<Gone> =
            core::mem::take(&mut *lib.deferred.lock().unwrap_or_else(PoisonError::into_inner));
        for gone in deferred {
            self.let_go(slot, gone);
        }
    }

    /// Lets go of `gone` now, or at the library's next entry when it is
    /// busy and this thread is inside a call.
    fn let_go_or_defer(&self, lib: &WasmLibrary, gone: Gone) {
        match lib.enter() {
            Some(mut entered) => self.let_go(&mut entered.slot, gone),
            None => lib
                .deferred
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(gone),
        }
    }

    /// The position of the provider declared as `id`.
    fn position(lib: &WasmLibrary, id: &str) -> Result<i64, ProviderError> {
        lib.providers
            .iter()
            .position(|p| p == id)
            .map(|at| at as i64)
            .ok_or_else(|| {
                ProviderError::new(
                    Status::GUATIAO_ERR_NOT_FOUND,
                    &format!("{} declares no provider `{id}`", lib.id),
                )
            })
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
            if let Some(schema) = &provider.config {
                served = served.config(schema.clone());
            }
            if provider.instances_only {
                served = served.instances_only();
            }
            if provider.may_be_unavailable {
                served = served.may_be_unavailable();
            }
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
        serves: "wasm",
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
            .wasm_memory64(false)
            .max_wasm_stack(instance::WASM_STACK);
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
            ticker,
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
        let _running = self.running();
        with_stack(|| self.open_here(manifest, version))
    }

    fn call(&self, lib: &Arc<WasmLibrary>, call: &mut Call) -> Result<Reply, ProviderError> {
        let _running = self.running();
        with_stack(|| self.call_here(lib, call))
    }

    /// Tells the guest its object was destroyed; deferred to the next
    /// call when the library is busy and this thread is inside a call.
    fn release(&self, lib: &Arc<WasmLibrary>, object: u64) {
        let _running = self.running();
        with_stack(|| self.let_go_or_defer(lib, Gone::Object(object)));
    }

    /// `guatiao_guest_create`, under the limits as a call is.
    fn create(
        &self,
        lib: &Arc<WasmLibrary>,
        provider: &str,
        config: &Value,
    ) -> Result<u64, ProviderError> {
        let _running = self.running();
        with_stack(|| self.create_here(lib, provider, config))
    }

    /// Tells the guest its instance was dropped; deferred as `release` is.
    fn destroy(&self, lib: &Arc<WasmLibrary>, instance: u64) {
        let _running = self.running();
        with_stack(|| self.let_go_or_defer(lib, Gone::Instance(instance)));
    }

    /// `guatiao_guest_available`, under the limits as a call is: a stop
    /// or a refusal is unavailable, in its words.
    fn available(&self, lib: &Arc<WasmLibrary>, provider: &str) -> Result<(), String> {
        let _running = self.running();
        with_stack(|| self.available_here(lib, provider)).map_err(|e| e.message().to_string())?
    }

    fn close(&self, lib: Arc<WasmLibrary>) {
        drop(lib);
    }
}

impl WasmEngine {
    fn open_here(
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
        let metered = self.limits.operations.is_some();
        let bytes = if metered {
            meter::metered(&bytes)
                .map_err(|e| BridgeError::engine(format!("{id} does not compile: {e}")))?
        } else {
            bytes
        };
        let module = Module::new(&self.engine, &bytes).map_err(|e| {
            BridgeError::engine(format!(
                "{id} does not compile: {}",
                e.to_string().lines().next().unwrap_or("")
            ))
        })?;
        check_shape(&module, id, metered)?;
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
        let declared = abi::read_declaration(&answer, budget::of(&self.limits))
            .map_err(|why| BridgeError::engine(format!("{id}: {why}")))?;
        let exports = |name: &str| module.exports().any(|e| e.name() == name);
        for provider in &declared {
            for (declares, needs, key) in [
                (provider.config.is_some(), export::CREATE, "config"),
                (
                    provider.may_be_unavailable,
                    export::AVAILABLE,
                    "may_be_unavailable",
                ),
            ] {
                if declares && !exports(needs) {
                    return Err(BridgeError::engine(format!(
                        "{id}: the provider `{}` declares `{key}`, and the module exports no `{needs}`",
                        provider.id
                    )));
                }
            }
        }
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
                    instances: BTreeMap::new(),
                    next_instance: 0,
                }),
                deferred: Mutex::new(Vec::new()),
            }),
            providers,
        })
    }

    fn create_here(
        &self,
        lib: &WasmLibrary,
        provider: &str,
        config: &Value,
    ) -> Result<u64, ProviderError> {
        let position = Self::position(lib, provider)?;
        let bytes = guatiao::value::wire::to_bytes(config).map_err(|e| {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!("the configuration: {e}"),
            )
        })?;
        let mut entered = lib.enter().ok_or_else(busy)?;
        let slot = &mut *entered.slot;
        self.settle(lib, slot);
        let live = self.live(lib, slot)?;
        let (status, answer) = match live.create(&self.limits, position, &bytes) {
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
        let refused = |what: &str| {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!(
                    "{} built an instance of `{provider}` and answered {what}, not its id",
                    lib.id
                ),
            )
        };
        let bytes = answer.ok_or_else(|| refused("nothing"))?;
        let value = budget::decode(&bytes, budget::of(&self.limits))
            .map_err(|_| refused("bytes that are not a value"))?;
        let guest = i64::from_value(&value).map_err(|_| refused("something else"))?;
        slot.next_instance += 1;
        let id = slot.next_instance;
        slot.instances.insert(id, guest);
        Ok(id)
    }

    fn available_here(
        &self,
        lib: &WasmLibrary,
        provider: &str,
    ) -> Result<Result<(), String>, ProviderError> {
        let position = Self::position(lib, provider)?;
        let mut entered = lib.enter().ok_or_else(busy)?;
        let slot = &mut *entered.slot;
        self.settle(lib, slot);
        let live = self.live(lib, slot)?;
        let (status, answer) = match live.available(&self.limits, position) {
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
        let Some(bytes) = answer else {
            return Ok(Ok(()));
        };
        let unreadable = || {
            ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                &format!(
                    "{} answered whether `{provider}` runs with neither nothing nor a reason",
                    lib.id
                ),
            )
        };
        let value = budget::decode(&bytes, budget::of(&self.limits)).map_err(|_| unreadable())?;
        if matches!(value.tag(), Ok(guatiao::Tag::GUATIAO_NULL)) {
            return Ok(Ok(()));
        }
        TryAsRef::<str>::try_as_ref(&value)
            .map(|why| Err(why.to_string()))
            .ok_or_else(unreadable)
    }

    fn call_here(&self, lib: &WasmLibrary, call: &mut Call) -> Result<Reply, ProviderError> {
        let mut entered = lib.enter().ok_or_else(busy)?;
        let slot = &mut *entered.slot;
        self.settle(lib, slot);
        let (object, target) = match call.target() {
            Target::Provider(id) => (0, Self::position(lib, id)?),
            Target::Object(id) => (
                1,
                *slot.objects.get(id).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_GONE,
                        "the object went with the instance that answered it, which stopped",
                    )
                })?,
            ),
            Target::Instance(id) => (
                2,
                *slot.instances.get(id).ok_or_else(|| {
                    ProviderError::new(
                        Status::GUATIAO_ERR_GONE,
                        "the instance went with the guest's store that built it, which stopped",
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

        // Everything that can fail comes first: an object crosses as the
        // handle it will be held under, and moves into the store's handles
        // only once the request is made, so no refusal leaves one behind.
        let state = live.store.data_mut();
        let mut callers = BTreeMap::new();
        for (i, (arg, d)) in call.args.iter().zip(slot_description.args()).enumerate() {
            if let (ArgType::Object { kind, .. }, CallArg::Object(_) | CallArg::LentObject(_)) =
                (d.ty(), arg)
            {
                callers.insert(i, self.shared.caller(kind)?);
            }
        }
        // The handles `hold` gives below, in argument order.
        let handles: BTreeMap<usize, u64> = callers
            .keys()
            .zip(state.next_handle() + 1..)
            .map(|(i, h)| (*i, h))
            .collect();
        let mut values: Vec<Cow<'_, Value>> = Vec::with_capacity(call.args.len());
        let mut writable = Vec::with_capacity(call.args.len());
        for (i, arg) in call.args.iter().enumerate() {
            let (value, w) = match arg {
                _ if handles.contains_key(&i) => {
                    (Cow::Owned(Value::from(handles[&i])), Writable::No)
                }
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
            writable.push(w);
        }
        let request = abi::request(described.name(), &method, values.iter().map(|v| v.as_ref()))
            .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why))?;
        drop(values);
        state.writable = writable;
        state.written.clear();
        let mut lent = Vec::new();
        for (i, caller) in callers {
            let held = match &mut call.args[i] {
                CallArg::LentObject(h) => Held::Lent(h.clone()),
                arg => {
                    let CallArg::Object(object) =
                        core::mem::replace(arg, CallArg::Value(Value::null()))
                    else {
                        unreachable!("only objects have callers")
                    };
                    Held::Object(Some(object))
                }
            };
            let lend = matches!(held, Held::Lent(_));
            let handle = state.hold(held, caller);
            debug_assert_eq!(handle, handles[&i]);
            if lend {
                lent.push(handle);
            }
        }

        let outcome = live.call(&self.limits, object, target, &request);
        let state = live.store.data_mut();
        for handle in lent {
            state.handles.get_mut().remove(&handle);
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
            budget::decode(bytes, budget::of(&self.limits)).map_err(|e| match e {
                Undecoded::Refused(refused) => ProviderError::new(
                    Refused::STATUS,
                    &refused.words(&format!("`{method}`'s answer")),
                ),
                Undecoded::Malformed(e) => ProviderError::new(
                    Status::GUATIAO_ERR_BAD_VALUE,
                    &format!("`{method}` answered bytes that are not a value: {e}"),
                ),
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
}
