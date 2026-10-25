// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's side as a script sees it: an object a host handed over, or
//! lent for one call, is a JavaScript object whose methods are its kind's
//! slots, called through core's `Caller`.

use alloc::collections::BTreeMap;
use alloc::rc::{Rc, Weak};
use alloc::sync::Arc;
use core::cell::{Cell, RefCell};

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, AnswerType, Arg, ArgType, Caller, KindDescription, LentHandle, LentObject, Pass,
    RawObject, Scalar, SlotDescription, Tree,
};
use guatiao::{List, Map, Status, Value};
use guatiao_bridge::Kinds;
use guatiao_libffi::Libffi;
use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, Symbol, Type, TypedArray, Value as Js};

use crate::errors::{self, Words, bad_value};
use crate::prelude::{self, Bounds, Core};
use crate::values::{self, Conversion};

/// The key of the context a script runs in.
pub(crate) fn context_key(ctx: &Ctx<'_>) -> usize {
    ctx.as_raw().as_ptr() as usize
}

/// A handle known by id: the context it was made for, and the handle.
type Registered = (usize, Weak<RefCell<Handle>>);

/// What every context shares: the kinds the host described, a caller per
/// kind, and the handles scripts hold.
pub(crate) struct Shared {
    kinds: RefCell<Kinds>,
    callers: RefCell<BTreeMap<String, Arc<Caller>>>,
    /// Each handle by its id, with the context it was made for.
    handles: RefCell<BTreeMap<u64, Registered>>,
    /// Objects whose handle was freed. A handle is freed inside QuickJS's
    /// collector, where nothing may run script; destroying an object can
    /// (its owner may be a script), so the engine destroys these once
    /// every frame has returned.
    graveyard: RefCell<Vec<RawObject>>,
    next: Cell<u64>,
    purge_at: Cell<usize>,
    pub(crate) words: Words,
    /// What a conversion may make; the engine's, out of any script's reach.
    pub(crate) bounds: Bounds,
    /// Whether the call running was stopped: a host is not called then.
    stopped: Rc<Cell<bool>>,
}

impl Shared {
    pub(crate) fn new(kinds: Kinds, words: Words, stopped: Rc<Cell<bool>>) -> Rc<Shared> {
        Rc::new(Shared {
            kinds: RefCell::new(kinds),
            callers: RefCell::new(BTreeMap::new()),
            handles: RefCell::new(BTreeMap::new()),
            graveyard: RefCell::new(Vec::new()),
            next: Cell::new(0),
            purge_at: Cell::new(256),
            words,
            bounds: Bounds::of(words.memory),
            stopped,
        })
    }

    /// The kind named `name`, as the host described it.
    pub(crate) fn kind(&self, name: &str) -> Result<Arc<KindDescription>, ProviderError> {
        self.kinds.borrow().get(name).cloned().ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_NOT_FOUND,
                &format!("no description of the kind `{name}` was given"),
            )
        })
    }

    /// A caller of the kind named `name`, made once.
    pub(crate) fn caller(&self, name: &str) -> Result<Arc<Caller>, ProviderError> {
        if let Some(c) = self.callers.borrow().get(name) {
            return Ok(Arc::clone(c));
        }
        let made = Arc::new(
            Caller::new(self.kind(name)?, &Libffi)
                .map_err(|e| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &e.to_string()))?,
        );
        self.callers
            .borrow_mut()
            .insert(name.to_string(), Arc::clone(&made));
        Ok(made)
    }

    /// Keeps `object` until the engine can destroy it.
    fn bury(&self, object: RawObject) {
        self.graveyard.borrow_mut().push(object);
    }

    /// The objects waiting to be destroyed.
    pub(crate) fn take_graveyard(&self) -> Vec<RawObject> {
        core::mem::take(&mut *self.graveyard.borrow_mut())
    }

    fn register(&self, context: usize, handle: &Rc<RefCell<Handle>>) -> u64 {
        let id = self.next.get() + 1;
        self.next.set(id);
        let mut handles = self.handles.borrow_mut();
        if handles.len() >= self.purge_at.get() {
            handles.retain(|_, (_, w)| w.strong_count() > 0);
            self.purge_at.set((handles.len() * 2).max(256));
        }
        handles.insert(id, (context, Rc::downgrade(handle)));
        id
    }

    /// The handle `id` names, if it was made for `context`: a library
    /// holds no other's.
    fn find(&self, id: u64, context: usize) -> Option<Rc<RefCell<Handle>>> {
        let handles = self.handles.borrow();
        let (made_for, handle) = handles.get(&id)?;
        (*made_for == context).then(|| handle.upgrade()).flatten()
    }
}

/// What a handle calls.
pub(crate) enum Target {
    /// An object handed over, until it is handed on or closed.
    Object(Option<RawObject>),
    /// An object lent for a call that may have returned.
    Lent(LentHandle),
}

/// An object of a described kind, held by a script.
pub(crate) struct Handle {
    target: Target,
    caller: Arc<Caller>,
    shared: Rc<Shared>,
    /// The script closed it: nothing goes back in.
    closed: bool,
}

impl Drop for Handle {
    /// Freed by QuickJS's collector, so it only hands the object to the
    /// engine to destroy later.
    fn drop(&mut self) {
        if let Target::Object(object) = &mut self.target
            && let Some(object) = object.take()
        {
            self.shared.bury(object);
        }
    }
}

impl Handle {
    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        match &self.target {
            Target::Object(Some(object)) => object.lend().call(&self.caller, slot, args),
            Target::Object(None) => Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the object was handed over or closed",
            )),
            Target::Lent(handle) => handle.call(&self.caller, slot, args),
        }
    }
}

/// A slot's name as a method is: `greet_into` is `greetInto`.
pub(crate) fn method_name(slot: &str) -> String {
    let mut out = String::with_capacity(slot.len());
    let mut up = false;
    for c in slot.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The failure `error` as a value a script catches: a `GuatiaoError` of
/// its status, thrown.
fn throw<'js>(ctx: &Ctx<'js>, core: &Core<'js>, error: &ProviderError) -> rquickjs::Error {
    match core
        .error_for
        .call::<_, Js<'js>>((u32::from(error.status), error.message()))
    {
        Ok(value) => ctx.throw(value),
        Err(e) => e,
    }
}

fn throw_conversion<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    error: Conversion,
) -> rquickjs::Error {
    match error {
        Conversion::Js(e) => e,
        other => throw(
            ctx,
            core,
            &errors::conversion(ctx, Some(core), &other, shared.words),
        ),
    }
}

/// A script's object for a handle: a method per slot of its kind, and
/// `close()`.
pub(crate) fn handle_object<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    caller: Arc<Caller>,
    target: Target,
) -> rquickjs::Result<(Object<'js>, Rc<RefCell<Handle>>)> {
    let kind = Arc::clone(caller.kind());
    let state = Rc::new(RefCell::new(Handle {
        target,
        caller,
        shared: Rc::clone(shared),
        closed: false,
    }));
    let id = shared.register(context_key(ctx), &state);
    let object = Object::new(ctx.clone())?;
    object.set(
        prelude::core(ctx, shared.bounds)?.handle_key.as_atom(),
        id as f64,
    )?;
    for (index, slot) in kind.slots().iter().enumerate() {
        let state = Rc::clone(&state);
        let shared = Rc::clone(shared);
        let method = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Js<'js>>| {
            call_slot(&ctx, &shared, &state, index, args.0)
        })?;
        object.set(method_name(slot.name()), method)?;
    }
    let owned = Rc::clone(&state);
    let close = Function::new(ctx.clone(), move || {
        if let Ok(mut handle) = owned.try_borrow_mut() {
            handle.closed = true;
            if let Target::Object(object) = &mut handle.target
                && let Some(object) = object.take()
            {
                // Destroyed once the call returns, as a freed handle's is.
                handle.shared.bury(object);
            }
        }
    })?;
    object.set("close", close.clone())?;
    let symbol: Object<'js> = ctx.globals().get("Symbol")?;
    if let Some(dispose) = symbol.get::<_, Option<Symbol<'js>>>("dispose")? {
        object.set(dispose.as_atom(), close)?;
    }
    Ok((object, state))
}

/// A handle on an object of the kind named, for a script.
pub(crate) fn object_for<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    kind: &str,
    target: Target,
) -> Result<Object<'js>, ProviderError> {
    let caller = shared.caller(kind)?;
    handle_object(ctx, shared, caller, target)
        .map(|(object, _)| object)
        .map_err(|e| errors::caught(ctx, None, &e, shared.words))
}

/// The handle a script's value is, if it is one.
pub(crate) fn handle_of<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    v: &Js<'js>,
) -> Option<Rc<RefCell<Handle>>> {
    let object = v.as_object()?;
    let id: f64 = object.get(core.handle_key.as_atom()).ok()?;
    shared.find(id as u64, context_key(ctx))
}

/// Takes the object out of a handle the script is handing back.
pub(crate) fn take_object(
    handle: &Rc<RefCell<Handle>>,
    kind: &str,
) -> Result<RawObject, ProviderError> {
    let Ok(mut handle) = handle.try_borrow_mut() else {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_BUSY,
            "the object is in use",
        ));
    };
    if handle.caller.kind().name() != kind {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_WRONG_KIND,
            &format!(
                "an object of `{}` where one of `{kind}` is answered",
                handle.caller.kind().name()
            ),
        ));
    }
    match &mut handle.target {
        Target::Object(object) => object.take().ok_or_else(|| {
            ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the object was already handed over",
            )
        }),
        Target::Lent(_) => Err(ProviderError::new(
            Status::GUATIAO_ERR_WRONG_KIND,
            "an object lent to this call cannot be handed back",
        )),
    }
}

// --- a slot called from a script -------------------------------------------------

/// `handle.slot(args...)`: the slot called, its answer as a script's. A
/// refusal is thrown as a `GuatiaoError` of its status.
fn call_slot<'js>(
    ctx: &Ctx<'js>,
    shared: &Rc<Shared>,
    state: &Rc<RefCell<Handle>>,
    index: usize,
    args: Vec<Js<'js>>,
) -> rquickjs::Result<Js<'js>> {
    let core = prelude::core(ctx, shared.bounds)?;
    if shared.stopped.get() {
        return Err(throw(
            ctx,
            &core,
            &ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                "the call was stopped, and reaches no host",
            ),
        ));
    }
    let caller = Arc::clone(&state.borrow().caller);
    let kind = Arc::clone(caller.kind());
    let slot = &kind.slots()[index];
    let mut held = prepare(ctx, &core, shared, slot, args).map_err(|e| throw(ctx, &core, &e))?;
    let answer = with_args(&mut held.0, |args| state.borrow().call(index, args));
    settle(ctx, &core, &mut held).map_err(|e| throw_conversion(ctx, &core, shared, e))?;
    match answer {
        Ok(answer) => {
            answer_to_js(ctx, &core, shared, slot, answer).map_err(|e| throw(ctx, &core, &e))
        }
        Err(e) => Err(throw(ctx, &core, &e)),
    }
}

/// The arguments of a call. However the call ends, an object of the
/// script's own that was taken out of its handle for it goes back in: a
/// refused later argument, or a failed write-back, must not destroy what
/// was only lent.
struct Held<'js>(Vec<Prepared<'js>>);

impl Drop for Held<'_> {
    fn drop(&mut self) {
        for p in self.0.drain(..) {
            if let Prepared::Object(Some(object), handle)
            | Prepared::LentOwned(Some(object), handle) = p
            {
                give_back(&handle, object);
            }
        }
    }
}

/// Puts `object` back in the handle it was taken from.
fn give_back(handle: &Rc<RefCell<Handle>>, object: RawObject) {
    if let Ok(mut state) = handle.try_borrow_mut() {
        if state.closed {
            // Closed while it was out: not for the script to have again.
            state.shared.bury(object);
        } else {
            state.target = Target::Object(Some(object));
        }
    }
}

/// One argument, held while the call runs, and where a lent one goes back.
enum Prepared<'js> {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Buffer(Vec<u8>, Option<Js<'js>>),
    Value(Option<Value>),
    Map(Map),
    List(List),
    TakenValue(Option<Value>),
    TakenMap(Option<Map>),
    TakenList(Option<List>),
    LentValue(Value, Option<Js<'js>>),
    LentMap(Map, Option<Js<'js>>),
    LentList(List, Option<Js<'js>>),
    /// An object handed over, with the handle it came out of.
    Object(Option<RawObject>, Rc<RefCell<Handle>>),
    /// An object this side owns, lent for the call and put back after.
    LentOwned(Option<RawObject>, Rc<RefCell<Handle>>),
    /// A lent object passed on.
    LentPassed(LentHandle),
}

fn whole<'js>(core: &Core<'js>, v: &Js<'js>, what: &str) -> Result<i128, ProviderError> {
    let from_text = |text: String| {
        text.parse::<i128>()
            .map_err(|_| bad_value(format!("{what}: `{text}` is not a whole number")))
    };
    match v.type_of() {
        Type::Int => Ok(i128::from(v.as_int().unwrap_or(0))),
        Type::Float => {
            let f = v.as_float().unwrap_or(f64::NAN);
            if f.fract() == 0.0 && f.abs() < 1.8e19 {
                Ok(f as i128)
            } else {
                Err(bad_value(format!("{what}: {f} is not a whole number")))
            }
        }
        Type::BigInt => from_text(text(v)?),
        Type::Object if v.as_object().is_some_and(|o| o.is_instance_of(&core.exact)) => {
            from_text(text(v)?)
        }
        other => Err(bad_value(format!(
            "{what} is a whole number, not a {}",
            other.as_str()
        ))),
    }
}

fn text<'js>(v: &Js<'js>) -> Result<String, ProviderError> {
    v.get::<rquickjs::Coerced<String>>()
        .map(|c| c.0)
        .map_err(|e| bad_value(e.to_string()))
}

fn scalar<'js>(
    core: &Core<'js>,
    v: &Js<'js>,
    ty: Scalar,
    what: &str,
) -> Result<Prepared<'js>, ProviderError> {
    Ok(match ty {
        Scalar::Bool => match v.as_bool() {
            Some(b) => Prepared::Bool(b),
            None => {
                return Err(bad_value(format!(
                    "{what} is a boolean, not a {}",
                    v.type_name()
                )));
            }
        },
        Scalar::F32 | Scalar::F64 => match v.type_of() {
            Type::Int | Type::Float => Prepared::Float(v.as_number().unwrap_or(f64::NAN)),
            Type::Object | Type::BigInt => Prepared::Float(
                text(v)?
                    .parse()
                    .map_err(|_| bad_value(format!("{what} is a number")))?,
            ),
            other => {
                return Err(bad_value(format!(
                    "{what} is a number, not a {}",
                    other.as_str()
                )));
            }
        },
        Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize => {
            let n = whole(core, v, what)?;
            Prepared::Uint(
                u64::try_from(n).map_err(|_| bad_value(format!("{what}: {n} does not fit")))?,
            )
        }
        _ => {
            let n = whole(core, v, what)?;
            Prepared::Int(
                i64::try_from(n).map_err(|_| bad_value(format!("{what}: {n} does not fit")))?,
            )
        }
    })
}

/// The bytes a script's `Uint8Array` or `ArrayBuffer` holds.
fn bytes_of<'js>(core: &Core<'js>, v: &Js<'js>, what: &str) -> Result<Vec<u8>, ProviderError> {
    if let Some(held) = v.as_object().and_then(|o| values::bytes_in(core, o)) {
        return Ok(held);
    }
    Err(bad_value(format!(
        "{what} is bytes (a Uint8Array), not a {}",
        v.type_name()
    )))
}

fn tree_of<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    v: &Js<'js>,
    tree: Tree,
    what: &str,
) -> Result<Value, ProviderError> {
    let value = values::to_value(core, v)
        .map_err(|e| errors::conversion(ctx, Some(core), &e, shared.words))?;
    let fits = match tree {
        Tree::Map => guatiao::value::convert::TryAsRef::<Map>::try_as_ref(&value).is_some(),
        Tree::List => guatiao::value::convert::TryAsRef::<List>::try_as_ref(&value).is_some(),
        _ => true,
    };
    if fits {
        Ok(value)
    } else {
        Err(bad_value(format!(
            "{what} is a {}",
            if tree == Tree::Map { "map" } else { "list" }
        )))
    }
}

/// An object argument: a handle, handed over or lent on.
fn object_arg<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    v: &Js<'js>,
    kind: &str,
    pass: Pass,
    what: &str,
) -> Result<Prepared<'js>, ProviderError> {
    let Some(handle) = handle_of(ctx, core, shared, v) else {
        return Err(bad_value(format!(
            "{what} is an object of `{kind}` (a handle a host gave this library), not a {}",
            v.type_name()
        )));
    };
    let mut state = handle
        .try_borrow_mut()
        .map_err(|_| ProviderError::new(Status::GUATIAO_ERR_BUSY, "the object is in use"))?;
    if state.caller.kind().name() != kind {
        return Err(bad_value(format!(
            "{what} is a `{kind}`, not a `{}`",
            state.caller.kind().name()
        )));
    }
    match (&mut state.target, pass) {
        (Target::Object(object), pass) => {
            let taken = object
                .take()
                .ok_or_else(|| bad_value(format!("{what} was already handed over")))?;
            drop(state);
            Ok(if pass == Pass::Taken {
                Prepared::Object(Some(taken), handle)
            } else {
                Prepared::LentOwned(Some(taken), handle)
            })
        }
        (Target::Lent(lent), Pass::Lent) => Ok(Prepared::LentPassed(lent.clone())),
        (Target::Lent(_), _) => Err(bad_value(format!(
            "{what} was lent to this call, and cannot be handed over"
        ))),
    }
}

/// A script's values as `slot`'s arguments, held for the call.
fn prepare<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    slot: &SlotDescription,
    args: Vec<Js<'js>>,
) -> Result<Held<'js>, ProviderError> {
    let mut args = args.into_iter();
    let mut out = Held(Vec::with_capacity(slot.args().len()));
    for described in slot.args() {
        let v = args
            .next()
            .unwrap_or_else(|| Js::new_undefined(ctx.clone()));
        let what = format!("`{}`'s argument `{}`", slot.name(), described.name());
        out.0.push(match described.ty() {
            ArgType::Scalar(s) => scalar(core, &v, *s, &what)?,
            ArgType::Str => match v.as_string() {
                Some(s) => Prepared::Str(
                    s.to_string()
                        .map_err(|_| bad_value(format!("{what} is not valid Unicode text")))?,
                ),
                None => {
                    return Err(bad_value(format!(
                        "{what} is text, not a {}",
                        v.type_name()
                    )));
                }
            },
            ArgType::Bytes => Prepared::Bytes(bytes_of(core, &v, &what)?),
            ArgType::BytesMut => match v.type_of() {
                Type::Object
                    if v.as_object()
                        .is_some_and(|o| TypedArray::<u8>::from_object(o.clone()).is_ok()) =>
                {
                    Prepared::Buffer(bytes_of(core, &v, &what)?, Some(v))
                }
                Type::Int => {
                    let n = usize::try_from(v.as_int().unwrap_or(0)).unwrap_or(0);
                    if n > core.bounds.bytes {
                        return Err(bad_value(format!(
                            "{what}: {n} bytes is more than the {} this source makes",
                            core.bounds.bytes
                        )));
                    }
                    Prepared::Buffer(vec![0; n], None)
                }
                other => {
                    return Err(bad_value(format!(
                        "{what} is a Uint8Array to write into (or its length), not a {}",
                        other.as_str()
                    )));
                }
            },
            ArgType::Tree {
                tree,
                pass,
                nullable,
            } => {
                let home = (*pass == Pass::Lent && v.is_object()).then(|| v.clone());
                if (v.is_null() || v.is_undefined()) && *nullable && *pass == Pass::Borrowed {
                    Prepared::Value(None)
                } else {
                    let value = tree_of(ctx, core, shared, &v, *tree, &what)?;
                    match (tree, pass) {
                        (Tree::Map, Pass::Borrowed) => {
                            Prepared::Map(Map::try_from(value).unwrap_or_default())
                        }
                        (Tree::List, Pass::Borrowed) => {
                            Prepared::List(List::try_from(value).unwrap_or_default())
                        }
                        (_, Pass::Borrowed) => Prepared::Value(Some(value)),
                        (Tree::Map, Pass::Taken) => Prepared::TakenMap(Map::try_from(value).ok()),
                        (Tree::List, Pass::Taken) => {
                            Prepared::TakenList(List::try_from(value).ok())
                        }
                        (_, Pass::Taken) => Prepared::TakenValue(Some(value)),
                        (Tree::Map, _) => {
                            Prepared::LentMap(Map::try_from(value).unwrap_or_default(), home)
                        }
                        (Tree::List, _) => {
                            Prepared::LentList(List::try_from(value).unwrap_or_default(), home)
                        }
                        (_, _) => Prepared::LentValue(value, home),
                    }
                }
            }
            ArgType::Object { kind, pass } => {
                object_arg(ctx, core, shared, &v, kind, *pass, &what)?
            }
            _ => {
                return Err(bad_value(format!(
                    "{what} has a type this binding does not carry"
                )));
            }
        });
    }
    Ok(out)
}

/// Runs `f` with the arguments `prepared` holds, as core takes them.
fn with_args<R>(prepared: &mut [Prepared<'_>], f: impl FnOnce(Vec<Arg<'_>>) -> R) -> R {
    // A lent object passed on is lent for as long as its own lend lasts:
    // nest one `lend` per such argument.
    let passed: Vec<LentHandle> = prepared
        .iter()
        .filter_map(|p| match p {
            Prepared::LentPassed(h) => Some(h.clone()),
            _ => None,
        })
        .collect();
    lending(&passed, Vec::new(), move |lent| {
        let mut lent = lent.into_iter();
        let mut args = Vec::with_capacity(prepared.len());
        for p in prepared.iter_mut() {
            args.push(match p {
                Prepared::Bool(b) => Arg::Bool(*b),
                Prepared::Int(n) => Arg::Int(*n),
                Prepared::Uint(n) => Arg::Uint(*n),
                Prepared::Float(x) => Arg::Float(*x),
                Prepared::Str(s) => Arg::Str(s),
                Prepared::Bytes(b) => Arg::Bytes(b),
                Prepared::Buffer(b, _) => Arg::Buffer(b),
                Prepared::Value(v) => Arg::Value(v.as_ref()),
                Prepared::Map(m) => Arg::Map(m),
                Prepared::List(l) => Arg::List(l),
                Prepared::TakenValue(v) => Arg::TakenValue(v.take().unwrap_or_else(Value::null)),
                Prepared::TakenMap(m) => Arg::TakenMap(m.take().unwrap_or_default()),
                Prepared::TakenList(l) => Arg::TakenList(l.take().unwrap_or_default()),
                Prepared::LentValue(v, _) => Arg::LentValue(v),
                Prepared::LentMap(m, _) => Arg::LentMap(m),
                Prepared::LentList(l, _) => Arg::LentList(l),
                Prepared::Object(o, _) => match o.take() {
                    Some(o) => Arg::Object(o),
                    None => Arg::Value(None),
                },
                Prepared::LentOwned(Some(o), _) => Arg::LentObject(o.lend()),
                Prepared::LentOwned(None, _) => Arg::Value(None),
                Prepared::LentPassed(_) => match lent.next().flatten() {
                    Some(o) => Arg::LentObject(o),
                    None => Arg::Value(None),
                },
            });
        }
        f(args)
    })
}

/// Lends each of `handles` in turn (each lend lives only inside its
/// closure), then runs `f` with all of them.
fn lending<R>(
    handles: &[LentHandle],
    lent: Vec<Option<LentObject<'_>>>,
    f: impl FnOnce(Vec<Option<LentObject<'_>>>) -> R,
) -> R {
    match handles.split_first() {
        None => f(lent),
        Some((first, rest)) => first.lend(|object| {
            let mut all = lent;
            all.push(object);
            lending(rest, all, f)
        }),
    }
}

/// After the call: each lent tree written back over the object it came
/// from, each buffer into its `Uint8Array`, each object lent from a handle
/// put back in it.
fn settle<'js>(ctx: &Ctx<'js>, core: &Core<'js>, held: &mut Held<'js>) -> Result<(), Conversion> {
    // Popped one at a time, so that what a failure leaves is still in `held`.
    while let Some(p) = held.0.pop() {
        match p {
            Prepared::LentValue(v, Some(home)) => values::refill(core, ctx, &home, &v)?,
            Prepared::LentMap(m, Some(home)) => values::refill(core, ctx, &home, &Value::from(m))?,
            Prepared::LentList(l, Some(home)) => values::refill(core, ctx, &home, &Value::from(l))?,
            Prepared::Buffer(bytes, Some(home)) => values::overwrite(ctx, &home, &bytes)?,
            Prepared::LentOwned(Some(object), handle) => give_back(&handle, object),
            _ => {}
        }
    }
    Ok(())
}

// --- a slot's answer as a script's -------------------------------------------------

/// What `slot` answered, as a script's.
fn answer_to_js<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Rc<Shared>,
    slot: &SlotDescription,
    answer: Answer,
) -> Result<Js<'js>, ProviderError> {
    let wrong = |e: Conversion| errors::conversion(ctx, Some(core), &e, shared.words);
    let scalar = match slot.answer() {
        Some(AnswerType::Scalar(s)) => Some(*s),
        _ => None,
    };
    Ok(match answer {
        Answer::Nothing => Js::new_undefined(ctx.clone()),
        Answer::Bool(b) => Js::new_bool(ctx.clone(), b),
        Answer::Int(n) => values::scalar_to_js(ctx, scalar, &n.to_string()).map_err(wrong)?,
        Answer::Uint(n) => values::scalar_to_js(ctx, scalar, &n.to_string()).map_err(wrong)?,
        Answer::Float(x) => values::number(ctx, x),
        Answer::Text(t) => rquickjs::String::from_str(ctx.clone(), &t)
            .map_err(|e| errors::caught(ctx, Some(core), &e, shared.words))?
            .into_value(),
        Answer::Value(v) => values::to_js(core, ctx, &v).map_err(wrong)?,
        Answer::Map(m) => values::to_js(core, ctx, &Value::from(m)).map_err(wrong)?,
        Answer::List(l) => values::to_js(core, ctx, &Value::from(l)).map_err(wrong)?,
        Answer::Object(o) => match slot.answer() {
            Some(AnswerType::Object { kind }) => {
                object_for(ctx, shared, kind, Target::Object(Some(o)))?.into_value()
            }
            _ => return Err(bad_value("an object answered where the slot names none")),
        },
        _ => return Err(bad_value("an answer this binding does not carry")),
    })
}
