// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One instance of a guest: its store and what the store keeps, the
//! imports the guest calls, entering it under the limits, and naming why
//! it stopped.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use std::sync::Mutex;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, AnswerType, Arg, ArgType, Caller as KindCaller, LentHandle, Pass, RawObject, Scalar,
    Tree,
};
use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::value::wire;
use guatiao::{Buffer, List, Map, Status, Value};
use guatiao_bridge::{BridgeError, Kinds, Limit, Limits};
use guatiao_libffi::Libffi;
use wasmtime::{
    Caller, Engine, Instance, Linker, Memory, Module, ResourceLimiter, Store, Trap, TypedFunc,
};

use crate::abi::{self, export, import};
use crate::budget::{self, Refused, Undecoded};
use crate::wasi::WasiState;

/// The deepest a guest's own frames go: wasmtime's `max_wasm_stack`.
pub(crate) const WASM_STACK: usize = 512 << 10;
/// What the host code a guest's imports run, below its frames, may use.
const HOST_STACK: usize = 512 << 10;
/// The stack an entry runs on when its caller's has too little left.
const SEGMENT: usize = 4 << 20;

/// Runs `f`, which enters a guest, where at least a guest's stack and the
/// host's below it are left: on the caller's stack, or on a segment of its
/// own. wasmtime bounds a guest's frames from where it is entered, so a
/// caller with less left would overflow the native stack, not trap.
pub(crate) fn with_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(WASM_STACK + HOST_STACK, SEGMENT, f)
}

/// The kinds a source was given, and a caller per object kind, made once.
pub(crate) struct Shared {
    pub(crate) kinds: Kinds,
    callers: Mutex<BTreeMap<String, Arc<KindCaller>>>,
}

impl Shared {
    pub(crate) fn new(kinds: Kinds) -> Shared {
        Shared {
            kinds,
            callers: Mutex::new(BTreeMap::new()),
        }
    }

    /// What calls a table of the kind named.
    pub(crate) fn caller(&self, kind: &str) -> Result<Arc<KindCaller>, ProviderError> {
        let mut callers = self
            .callers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(c) = callers.get(kind) {
            return Ok(Arc::clone(c));
        }
        let described = self
            .kinds
            .get(kind)
            .cloned()
            .ok_or_else(|| ProviderError::from(BridgeError::UnknownKind(kind.to_string())))?;
        let made = Arc::new(
            KindCaller::new(described, &Libffi)
                .map_err(|e| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &e.to_string()))?,
        );
        callers.insert(kind.to_string(), Arc::clone(&made));
        Ok(made)
    }
}

/// An object of the host's the guest holds a handle on.
pub(crate) enum Held {
    /// Handed over: the guest's until it drops it or the instance goes.
    Owned(RawObject),
    /// Lent for one call.
    Lent(LentHandle),
}

pub(crate) struct HostObject {
    pub(crate) held: Held,
    pub(crate) caller: Arc<KindCaller>,
}

/// What an argument of the current call may be written back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Writable {
    No,
    Tree,
    Buffer,
}

/// A store's data: the limiter's count, what the guest answered and wrote
/// back during the current call, the host's objects it holds, and what
/// it handed over past what the host takes.
pub(crate) struct State {
    memory_limit: Option<usize>,
    memory_bytes: usize,
    table_entries: usize,
    pub(crate) memory_hit: bool,
    /// Tables grew past what the host holds for them with no memory limit.
    tables_past: bool,
    /// The bytes the host takes from the guest at once, and decodes.
    pub(crate) budget: u64,
    /// Bytes past the budget the guest tried to hand over.
    pub(crate) too_large: Option<u64>,
    pub(crate) answer: Option<Vec<u8>>,
    result: Vec<u8>,
    pub(crate) writable: Vec<Writable>,
    pub(crate) written: Vec<(usize, Value)>,
    pub(crate) handles: BTreeMap<u64, HostObject>,
    next_handle: u64,
    pub(crate) wasi: WasiState,
    shared: Arc<Shared>,
}

impl State {
    pub(crate) fn new(limits: &Limits, shared: Arc<Shared>) -> State {
        State {
            memory_limit: limits
                .memory
                .map(|m| usize::try_from(m).unwrap_or(usize::MAX)),
            memory_bytes: 0,
            table_entries: 0,
            memory_hit: false,
            tables_past: false,
            budget: budget::of(limits),
            too_large: None,
            answer: None,
            result: Vec::new(),
            writable: Vec::new(),
            written: Vec::new(),
            handles: BTreeMap::new(),
            next_handle: 0,
            wasi: WasiState::new(),
            shared,
        }
    }

    /// The handle the last object held was given.
    pub(crate) fn next_handle(&self) -> u64 {
        self.next_handle
    }

    /// Holds an object for the guest, answering its handle.
    pub(crate) fn hold(&mut self, held: Held, caller: Arc<KindCaller>) -> u64 {
        self.next_handle += 1;
        self.handles
            .insert(self.next_handle, HostObject { held, caller });
        self.next_handle
    }
}

/// What the host keeps per table entry, counted against the memory limit.
const TABLE_ENTRY: usize = 8;
/// The most tables a guest may have: compilers emit one.
pub(crate) const MOST_TABLES: usize = 4;

impl State {
    /// Whether memory and tables of these sizes fit: within the memory
    /// limit together, and without one the tables within
    /// [`budget::UNLIMITED`], since wasmtime keeps a table's entries on
    /// the host's heap.
    fn fits(&mut self, memory: usize, entries: usize) -> bool {
        let tables = entries.saturating_mul(TABLE_ENTRY);
        match self.memory_limit {
            Some(limit) if memory.saturating_add(tables) > limit => {
                self.memory_hit = true;
                false
            }
            None if tables as u64 > budget::UNLIMITED => {
                self.tables_past = true;
                false
            }
            _ => true,
        }
    }
}

impl ResourceLimiter for State {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if !self.fits(desired, self.table_entries) {
            return Err(wasmtime::Error::msg("the memory limit"));
        }
        self.memory_bytes = desired;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let entries = self
            .table_entries
            .saturating_add(desired.saturating_sub(current));
        if !self.fits(self.memory_bytes, entries) {
            return Err(wasmtime::Error::msg("the memory limit"));
        }
        self.table_entries = entries;
        Ok(true)
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        MOST_TABLES
    }

    fn memories(&self) -> usize {
        1
    }
}

// --- the imports -------------------------------------------------------------

fn memory_of(caller: &mut Caller<'_, State>) -> wasmtime::Result<Memory> {
    caller
        .get_export(export::MEMORY)
        .and_then(|e| e.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("the guest exports no memory"))
}

/// `len` bytes of the guest's memory at `ptr`, copied; a range outside
/// it, or past the budget, stops the guest.
fn bytes_at(caller: &mut Caller<'_, State>, ptr: i32, len: i32) -> wasmtime::Result<Vec<u8>> {
    let n = u64::from(len as u32);
    if n > caller.data().budget {
        caller.data_mut().too_large = Some(n);
        return Err(wasmtime::Error::msg("more bytes than the host takes"));
    }
    let memory = memory_of(caller)?;
    let start = ptr as u32 as usize;
    start
        .checked_add(len as u32 as usize)
        .and_then(|end| memory.data(&*caller).get(start..end))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| wasmtime::Error::msg("the guest passed bytes outside its memory"))
}

/// Bytes a guest handed over, decoded within the budget.
fn decoded(
    caller: &mut Caller<'_, State>,
    bytes: &[u8],
) -> wasmtime::Result<Result<Value, Undecoded>> {
    Ok(budget::decode(bytes, caller.data().budget))
}

/// The status a refusal to decode crosses as.
pub(crate) fn refusal_status(refused: &Refused) -> Status {
    match refused {
        Refused::TooLarge { .. } => Status::GUATIAO_ERR_ALLOC,
        Refused::Maps { .. } => Status::GUATIAO_ERR_BAD_VALUE,
    }
}

fn code(status: Status) -> i32 {
    u32::from(status) as i32
}

fn write_back(
    caller: &mut Caller<'_, State>,
    index: i32,
    ptr: i32,
    len: i32,
) -> wasmtime::Result<i32> {
    let bytes = bytes_at(caller, ptr, len)?;
    let Some(kind) = usize::try_from(index)
        .ok()
        .and_then(|i| caller.data().writable.get(i).copied())
    else {
        return Ok(code(Status::GUATIAO_ERR_NOT_FOUND));
    };
    let value = match decoded(caller, &bytes)? {
        Ok(value) => value,
        Err(Undecoded::Refused(refused)) => return Ok(code(refusal_status(&refused))),
        Err(Undecoded::Malformed(_)) => return Ok(code(Status::GUATIAO_ERR_BAD_VALUE)),
    };
    let state = caller.data_mut();
    let fits = match kind {
        Writable::No => false,
        Writable::Tree => true,
        Writable::Buffer => TryAsRef::<[u8]>::try_as_ref(&value).is_some(),
    };
    if !fits {
        return Ok(code(Status::GUATIAO_ERR_WRONG_KIND));
    }
    let index = index as usize;
    state.written.retain(|(i, _)| *i != index);
    state.written.push((index, value));
    Ok(0)
}

/// One argument of a host object's slot, from the value a guest sent.
enum Prepared {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Value(Option<Value>),
    Map(Map),
    List(List),
    TakenValue(Option<Value>),
    TakenMap(Option<Map>),
    TakenList(Option<List>),
    Object(Option<RawObject>),
}

fn unsupported(why: String) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, &why)
}

fn bad(why: String) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why)
}

fn prepare(
    state: &mut State,
    ty: &ArgType,
    value: Value,
    what: &str,
) -> Result<Prepared, ProviderError> {
    let scalar = |ok: Option<Prepared>| ok.ok_or_else(|| bad(format!("{what} does not fit")));
    Ok(match ty {
        ArgType::Scalar(Scalar::Bool) => scalar(bool::from_value(&value).ok().map(Prepared::Bool))?,
        ArgType::Scalar(Scalar::F32 | Scalar::F64) => {
            scalar(f64::from_value(&value).ok().map(Prepared::Float))?
        }
        ArgType::Scalar(Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize) => {
            scalar(u64::from_value(&value).ok().map(Prepared::Uint))?
        }
        ArgType::Scalar(_) => scalar(i64::from_value(&value).ok().map(Prepared::Int))?,
        ArgType::Str => Prepared::Str(
            TryAsRef::<str>::try_as_ref(&value)
                .ok_or_else(|| bad(format!("{what} is text")))?
                .to_string(),
        ),
        ArgType::Bytes => Prepared::Bytes(
            TryAsRef::<[u8]>::try_as_ref(&value)
                .ok_or_else(|| bad(format!("{what} is bytes")))?
                .to_vec(),
        ),
        ArgType::Tree {
            tree,
            pass,
            nullable,
        } => {
            let is_null = matches!(value.tag(), Ok(guatiao::Tag::GUATIAO_NULL));
            match (tree, pass) {
                (_, Pass::Lent) => {
                    return Err(unsupported(format!(
                        "{what} is lent, which a guest cannot lend: pass a value"
                    )));
                }
                (_, Pass::Borrowed) if is_null && *nullable => Prepared::Value(None),
                (Tree::Map, Pass::Borrowed) => Prepared::Map(
                    Map::try_from(value).map_err(|_| bad(format!("{what} is a map")))?,
                ),
                (Tree::List, Pass::Borrowed) => Prepared::List(
                    List::try_from(value).map_err(|_| bad(format!("{what} is a list")))?,
                ),
                (_, Pass::Borrowed) => Prepared::Value(Some(value)),
                (Tree::Map, _) => Prepared::TakenMap(Some(
                    Map::try_from(value).map_err(|_| bad(format!("{what} is a map")))?,
                )),
                (Tree::List, _) => Prepared::TakenList(Some(
                    List::try_from(value).map_err(|_| bad(format!("{what} is a list")))?,
                )),
                (_, _) => Prepared::TakenValue(Some(value)),
            }
        }
        ArgType::Object {
            kind,
            pass: Pass::Taken,
        } => {
            let handle = u64::from_value(&value)
                .map_err(|_| bad(format!("{what} is an object's handle")))?;
            let same = state.handles.get(&handle).is_some_and(|h| {
                matches!(h.held, Held::Owned(_)) && h.caller.kind().name() == kind
            });
            if !same {
                return Err(bad(format!(
                    "{what} is a `{kind}` the guest owns, which handle {handle} is not"
                )));
            }
            match state.handles.remove(&handle).map(|h| h.held) {
                Some(Held::Owned(object)) => Prepared::Object(Some(object)),
                _ => unreachable!("checked just above"),
            }
        }
        _ => {
            return Err(unsupported(format!(
                "{what} has a type a guest cannot pass to the host"
            )));
        }
    })
}

fn args_of(prepared: &mut [Prepared]) -> Vec<Arg<'_>> {
    prepared
        .iter_mut()
        .map(|p| match p {
            Prepared::Bool(b) => Arg::Bool(*b),
            Prepared::Int(n) => Arg::Int(*n),
            Prepared::Uint(n) => Arg::Uint(*n),
            Prepared::Float(x) => Arg::Float(*x),
            Prepared::Str(s) => Arg::Str(s),
            Prepared::Bytes(b) => Arg::Bytes(b),
            Prepared::Value(v) => Arg::Value(v.as_ref()),
            Prepared::Map(m) => Arg::Map(m),
            Prepared::List(l) => Arg::List(l),
            Prepared::TakenValue(v) => Arg::TakenValue(v.take().unwrap_or_else(Value::null)),
            Prepared::TakenMap(m) => Arg::TakenMap(m.take().unwrap_or_default()),
            Prepared::TakenList(l) => Arg::TakenList(l.take().unwrap_or_default()),
            Prepared::Object(o) => match o.take() {
                Some(o) => Arg::Object(o),
                None => Arg::Value(None),
            },
        })
        .collect()
}

/// A host object's answer as the bytes a guest reads: nothing for none.
fn answered(
    state: &mut State,
    answer_type: Option<&AnswerType>,
    answer: Answer,
) -> Result<Vec<u8>, ProviderError> {
    let value = match answer {
        Answer::Nothing => return Ok(Vec::new()),
        Answer::Bool(b) => Value::from(b),
        Answer::Int(n) => Value::from(n),
        Answer::Uint(n) => Value::from(n),
        Answer::Float(x) => Value::try_from(x)
            .map_err(|_| bad("the answer is a float that is not finite".to_string()))?,
        Answer::Text(t) => Value::from(t.as_str()),
        Answer::Value(v) => v,
        Answer::Map(m) => Value::from(m),
        Answer::List(l) => Value::from(l),
        Answer::Object(object) => {
            let Some(AnswerType::Object { kind }) = answer_type else {
                return Err(bad(
                    "an object answered where the slot names none".to_string()
                ));
            };
            let caller = state.shared.caller(kind)?;
            Value::from(state.hold(Held::Owned(object), caller))
        }
        _ => return Err(unsupported("an answer a guest cannot read".to_string())),
    };
    wire::to_bytes(&value).map_err(|e| bad(e.to_string()))
}

/// Calls a slot of a host object the guest holds, with the request it sent.
fn call_object(
    state: &mut State,
    handle: u64,
    request: Result<Value, Undecoded>,
) -> Result<Vec<u8>, ProviderError> {
    let Some(mut object) = state.handles.remove(&handle) else {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_GONE,
            &format!("no object has handle {handle}"),
        ));
    };
    let outcome = (|| {
        let request = request.map_err(|undecoded| match undecoded {
            Undecoded::Refused(refused) => {
                ProviderError::new(refusal_status(&refused), &refused.words("the request"))
            }
            Undecoded::Malformed(e) => bad(format!("the request is not a value: {e}")),
        })?;
        let (method, args) = abi::read_request(request).map_err(bad)?;
        let HostObject { held, caller } = &mut object;
        let kind = caller.kind();
        let (slot, described) = kind.slot(&method).ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                &format!("`{}` has no slot `{method}`", kind.name()),
            )
        })?;
        if args.len() != described.args().len() {
            return Err(bad(format!(
                "`{method}` takes {} arguments, not {}",
                described.args().len(),
                args.len()
            )));
        }
        let mut prepared = Vec::with_capacity(args.len());
        for (value, arg) in args.into_iter().zip(described.args()) {
            let what = format!("`{method}`'s argument `{}`", arg.name());
            prepared.push(prepare(state, arg.ty(), value, &what)?);
        }
        let args = args_of(&mut prepared);
        let answer = match held {
            Held::Owned(raw) => raw.call(caller, slot, args),
            Held::Lent(lent) => lent.call(caller, slot, args),
        }?;
        answered(state, described.answer(), answer)
    })();
    state.handles.insert(handle, object);
    outcome
}

/// Adds the `guatiao` imports to `linker`.
pub(crate) fn add_imports(linker: &mut Linker<State>) -> wasmtime::Result<()> {
    let module = abi::IMPORT_MODULE;
    linker.func_wrap(
        module,
        import::ANSWER,
        |mut caller: Caller<'_, State>, ptr: i32, len: i32| -> wasmtime::Result<()> {
            let bytes = bytes_at(&mut caller, ptr, len)?;
            caller.data_mut().answer = Some(bytes);
            Ok(())
        },
    )?;
    linker.func_wrap(
        module,
        import::WRITE_BACK,
        |mut caller: Caller<'_, State>, index: i32, ptr: i32, len: i32| {
            write_back(&mut caller, index, ptr, len)
        },
    )?;
    linker.func_wrap(
        module,
        import::OBJECT_CALL,
        |mut caller: Caller<'_, State>, handle: i64, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            let bytes = bytes_at(&mut caller, ptr, len)?;
            let request = decoded(&mut caller, &bytes)?;
            let state = caller.data_mut();
            let outcome = call_object(state, handle as u64, request);
            Ok(match outcome {
                Ok(bytes) => {
                    state.result = bytes;
                    0
                }
                Err(e) => {
                    state.result = e.message().as_bytes().to_vec();
                    code(e.status)
                }
            })
        },
    )?;
    linker.func_wrap(
        module,
        import::RESULT,
        |mut caller: Caller<'_, State>, ptr: i32, cap: i32| -> wasmtime::Result<i32> {
            let memory = memory_of(&mut caller)?;
            let (data, state) = memory.data_and_store_mut(&mut caller);
            let n = state.result.len().min(cap as u32 as usize);
            let start = ptr as u32 as usize;
            let into = start
                .checked_add(n)
                .and_then(|end| data.get_mut(start..end))
                .ok_or_else(|| wasmtime::Error::msg("the guest passed bytes outside its memory"))?;
            into.copy_from_slice(&state.result[..n]);
            Ok(i32::try_from(state.result.len()).unwrap_or(i32::MAX))
        },
    )?;
    linker.func_wrap(
        module,
        import::DROP,
        |mut caller: Caller<'_, State>, handle: i64| {
            let gone = caller.data_mut().handles.remove(&(handle as u64));
            drop(gone);
        },
    )?;
    Ok(())
}

// --- a live instance ------------------------------------------------------------

/// An instance of a guest, ready for calls.
pub(crate) struct Live {
    pub(crate) store: Store<State>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    declare: TypedFunc<(), i32>,
    call: TypedFunc<(i32, i64, i32, i32), i32>,
    release: Option<TypedFunc<i64, ()>>,
}

/// A guest's exports, found once.
struct Funcs {
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    declare: TypedFunc<(), i32>,
    call: TypedFunc<(i32, i64, i32, i32), i32>,
    release: Option<TypedFunc<i64, ()>>,
}

impl Funcs {
    fn of(
        store: &mut Store<State>,
        instance: &Instance,
        memory: Memory,
    ) -> wasmtime::Result<Funcs> {
        Ok(Funcs {
            memory,
            alloc: instance.get_typed_func(&mut *store, export::ALLOC)?,
            declare: instance.get_typed_func(&mut *store, export::DECLARE)?,
            call: instance.get_typed_func(&mut *store, export::CALL)?,
            release: instance.get_typed_func(&mut *store, export::RELEASE).ok(),
        })
    }
}

/// Why entering a guest stopped.
pub(crate) struct Stopped(pub(crate) wasmtime::Error);

impl From<wasmtime::Error> for Stopped {
    fn from(e: wasmtime::Error) -> Stopped {
        Stopped(e)
    }
}

/// Sets the limits for one entry into a guest: fuel and a deadline, each
/// only when the engine counts it.
pub(crate) fn arm(store: &mut Store<State>, limits: &Limits) -> wasmtime::Result<()> {
    if let Some(operations) = limits.operations {
        store.set_fuel(operations)?;
    }
    if let Some(time) = limits.time {
        store.set_epoch_deadline(u64::try_from(time.as_millis()).unwrap_or(u64::MAX).max(1));
    }
    Ok(())
}

impl Live {
    /// Instantiates `module`: its `_initialize`, then its ABI version,
    /// each under the limits.
    pub(crate) fn new(
        engine: &Engine,
        linker: &Linker<State>,
        module: &Module,
        limits: &Limits,
        shared: Arc<Shared>,
    ) -> Result<Live, (Stopped, Store<State>)> {
        let mut store = Store::new(engine, State::new(limits, shared));
        store.limiter(|state| state);
        let made = Live::start(&mut store, linker, module, limits)
            .and_then(|(instance, memory)| Funcs::of(&mut store, &instance, memory));
        match made {
            Ok(funcs) => Ok(Live {
                store,
                memory: funcs.memory,
                alloc: funcs.alloc,
                declare: funcs.declare,
                call: funcs.call,
                release: funcs.release,
            }),
            Err(e) => Err((Stopped(e), store)),
        }
    }

    fn start(
        store: &mut Store<State>,
        linker: &Linker<State>,
        module: &Module,
        limits: &Limits,
    ) -> wasmtime::Result<(Instance, Memory)> {
        arm(store, limits)?;
        let instance = linker.instantiate(&mut *store, module)?;
        if let Ok(init) = instance.get_typed_func::<(), ()>(&mut *store, export::INITIALIZE) {
            arm(store, limits)?;
            init.call(&mut *store, ())?;
        }
        let abi = instance.get_typed_func::<(), i32>(&mut *store, export::ABI)?;
        arm(store, limits)?;
        let version = abi.call(&mut *store, ())?;
        if version != abi::VERSION {
            return Err(wasmtime::Error::msg(format!(
                "it speaks guatiao ABI {version}, and this source ABI {}",
                abi::VERSION
            )));
        }
        let memory = instance
            .get_memory(&mut *store, export::MEMORY)
            .ok_or_else(|| wasmtime::Error::msg("it exports no memory"))?;
        Ok((instance, memory))
    }

    /// The guest's declaration, as it answered it.
    pub(crate) fn declare(&mut self, limits: &Limits) -> Result<(i32, Vec<u8>), Stopped> {
        self.store.data_mut().answer = None;
        arm(&mut self.store, limits)?;
        let status = self.declare.call(&mut self.store, ())?;
        Ok((
            status,
            self.store.data_mut().answer.take().unwrap_or_default(),
        ))
    }

    /// Runs one call: `request` copied into the guest's memory, then
    /// `guatiao_guest_call`. Answers its status and answer bytes.
    pub(crate) fn call(
        &mut self,
        limits: &Limits,
        object: bool,
        target: i64,
        request: &[u8],
    ) -> Result<(i32, Option<Vec<u8>>), Stopped> {
        let len = i32::try_from(request.len())
            .map_err(|_| wasmtime::Error::msg("the request is larger than a guest can hold"))?;
        self.store.data_mut().answer = None;
        arm(&mut self.store, limits)?;
        let at = self.alloc.call(&mut self.store, len)?;
        self.memory
            .write(&mut self.store, at as u32 as usize, request)
            .map_err(|_| wasmtime::Error::msg("the guest answered memory it does not have"))?;
        let status = self
            .call
            .call(&mut self.store, (i32::from(object), target, at, len))?;
        Ok((status, self.store.data_mut().answer.take()))
    }

    /// Tells the guest an object it answered was destroyed.
    pub(crate) fn release(&mut self, limits: &Limits, object: i64) -> Result<(), Stopped> {
        if let Some(release) = &self.release {
            arm(&mut self.store, limits)?;
            release.call(&mut self.store, object)?;
        }
        Ok(())
    }
}

// --- why it stopped -------------------------------------------------------------

/// The first line of what a stopped guest last said: what it wrote to fd
/// 2, else an answer it gave before stopping (a Rust guest's panic).
fn last_words(state: &State) -> Option<String> {
    let said = if state.wasi.stderr.is_empty() {
        state.answer.as_deref()?
    } else {
        &state.wasi.stderr
    };
    let text = core::str::from_utf8(said).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(300).collect())
}

/// Why `id`'s guest stopped, as the error its caller gets.
pub(crate) fn stopped(
    id: &str,
    error: &wasmtime::Error,
    state: &State,
    limits: &Limits,
) -> ProviderError {
    let limit = |limit: Limit, at: Option<u64>| {
        ProviderError::from(BridgeError::Limit {
            limit,
            at: at.unwrap_or(0),
        })
    };
    if state.memory_hit {
        return limit(Limit::Memory, limits.memory);
    }
    if state.tables_past {
        return ProviderError::new(
            Status::GUATIAO_ERR_ALLOC,
            &format!(
                "{id}'s tables would take more than {} bytes on the host, the most without a memory limit",
                budget::UNLIMITED
            ),
        );
    }
    if let Some(n) = state.too_large {
        return ProviderError::new(
            Status::GUATIAO_ERR_ALLOC,
            &format!(
                "{id} handed the host {n} bytes at once, more than it takes ({}: the memory limit, or {} without one)",
                state.budget,
                budget::UNLIMITED
            ),
        );
    }
    let words = last_words(state)
        .map(|w| format!(": {w}"))
        .unwrap_or_default();
    let message = match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => return limit(Limit::Operations, limits.operations),
        Some(Trap::Interrupt) => {
            return limit(
                Limit::Time,
                limits
                    .time
                    .map(|t| u64::try_from(t.as_millis()).unwrap_or(u64::MAX)),
            );
        }
        Some(trap) => format!("{id} trapped ({trap}){words}"),
        None => match state.wasi.exited {
            Some(code) => format!("{id} exited with {code}{words}"),
            // A host function's own words, under wasmtime's backtrace.
            None => {
                let first = error.root_cause().to_string();
                format!("{id}: {}{words}", first.lines().next().unwrap_or(""))
            }
        },
    };
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &message)
}

/// A refusal a guest answered: its status (internal for one guatiao does
/// not define) and its words.
pub(crate) fn refused(id: &str, status: i32, words: Option<Vec<u8>>) -> ProviderError {
    let words = words
        .map(|w| {
            String::from_utf8_lossy(&w)
                .chars()
                .take(1000)
                .collect::<String>()
        })
        .unwrap_or_default();
    match abi::status_of(status) {
        Some(known) => ProviderError::new(known, &words),
        None => ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            &format!("{id} answered status {status}, which guatiao does not define: {words}"),
        ),
    }
}

/// The bytes of a buffer argument, as a guest receives them.
pub(crate) fn buffer_value(bytes: &[u8]) -> Value {
    Value::from(Buffer::from(bytes))
}
