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
    Answer, AnswerType, Arg, ArgType, Caller, KindDescription, Pass, RawObject, SlotDescription,
};
use guatiao::{Status, Value};
use guatiao_bridge::{Held, Kinds, ObjectHome, PreparedArgs, Reading};
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

/// An object of a described kind, held by a script.
pub(crate) struct Handle {
    held: Held,
    caller: Arc<Caller>,
    shared: Rc<Shared>,
    /// The script closed it: nothing goes back in.
    closed: bool,
}

impl Drop for Handle {
    /// Freed by QuickJS's collector, so it only hands the object to the
    /// engine to destroy later.
    fn drop(&mut self) {
        if let Held::Object(object) = &mut self.held
            && let Some(object) = object.take()
        {
            self.shared.bury(object);
        }
    }
}

impl Handle {
    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        self.held.call(&self.caller, slot, args)
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
    held: Held,
) -> rquickjs::Result<(Object<'js>, Rc<RefCell<Handle>>)> {
    let kind = Arc::clone(caller.kind());
    let state = Rc::new(RefCell::new(Handle {
        held,
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
            if let Held::Object(object) = &mut handle.held
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
    held: Held,
) -> Result<Object<'js>, ProviderError> {
    let caller = shared.caller(kind)?;
    handle_object(ctx, shared, caller, held)
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
    let Handle { held, caller, .. } = &mut *handle;
    held.hand_back(caller, kind)
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
    let mut prepared =
        prepare(ctx, &core, shared, slot, args).map_err(|e| throw(ctx, &core, &e))?;
    let answer = Prepared::with_args(&mut prepared, |args| state.borrow().call(index, args));
    settle(ctx, &core, prepared).map_err(|e| throw_conversion(ctx, &core, shared, e))?;
    match answer {
        Ok(answer) => {
            answer_to_js(ctx, &core, shared, slot, answer).map_err(|e| throw(ctx, &core, &e))
        }
        Err(e) => Err(throw(ctx, &core, &e)),
    }
}

/// The handle an object argument was taken out of.
struct Home(Rc<RefCell<Handle>>);

impl ObjectHome for Home {
    fn give_back(self, object: RawObject) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            if state.closed {
                // Closed while it was out: not for the script to have again.
                state.shared.bury(object);
            } else {
                state.held.put_back(object);
            }
        }
    }
}

/// One argument, held while the call runs: a lent tree or buffer goes
/// back into the script's object it came from, an object into its handle.
type Prepared<'js> = guatiao_bridge::PreparedArg<Js<'js>, Js<'js>, Home>;

/// A call's arguments: whatever ends the call, an object taken out of a
/// handle and not handed over goes back in.
type Args<'js> = PreparedArgs<Js<'js>, Js<'js>, Home>;

/// A script's value as a scalar argument reads it.
fn reading<'js>(core: &Core<'js>, v: &Js<'js>) -> Result<Reading, ProviderError> {
    Ok(match v.type_of() {
        Type::Bool => Reading::Bool(v.as_bool().unwrap_or(false)),
        Type::Int => Reading::Int(i64::from(v.as_int().unwrap_or(0))),
        Type::Float => Reading::Float(v.as_float().unwrap_or(f64::NAN)),
        Type::BigInt => Reading::Exact(text(v)?),
        Type::Object if v.as_object().is_some_and(|o| o.is_instance_of(&core.exact)) => {
            Reading::Exact(text(v)?)
        }
        _ => Reading::Other,
    })
}

fn text<'js>(v: &Js<'js>) -> Result<String, ProviderError> {
    v.get::<rquickjs::Coerced<String>>()
        .map(|c| c.0)
        .map_err(|e| bad_value(e.to_string()))
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
    let Handle { held, caller, .. } = &mut *state;
    held.argument(caller, kind, pass, Home(Rc::clone(&handle)), what)
        .map_err(bad_value)
}

/// A script's values as `slot`'s arguments, held for the call.
fn prepare<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    slot: &SlotDescription,
    args: Vec<Js<'js>>,
) -> Result<Args<'js>, ProviderError> {
    let mut args = args.into_iter();
    let mut out = Args::with_capacity(slot.args().len());
    for described in slot.args() {
        let v = args
            .next()
            .unwrap_or_else(|| Js::new_undefined(ctx.clone()));
        let what = format!("`{}`'s argument `{}`", slot.name(), described.name());
        out.push(match described.ty() {
            ArgType::Scalar(s) => {
                Prepared::scalar(reading(core, &v)?, v.type_name(), *s, &what).map_err(bad_value)?
            }
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
                if (v.is_null() || v.is_undefined()) && *nullable && *pass == Pass::Borrowed {
                    Prepared::Value(None)
                } else {
                    let home = (*pass == Pass::Lent && v.is_object()).then(|| v.clone());
                    let value = values::to_value(core, &v)
                        .map_err(|e| errors::conversion(ctx, Some(core), &e, shared.words))?;
                    Prepared::tree(value, *tree, *pass, home, &what).map_err(bad_value)?
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

/// After the call: each lent tree written back over the object it came
/// from, each buffer into its `Uint8Array`; each object lent from a handle
/// goes back in as `args` drops, whether or not a write-back failed.
fn settle<'js>(ctx: &Ctx<'js>, core: &Core<'js>, mut args: Args<'js>) -> Result<(), Conversion> {
    for p in args.iter_mut() {
        match p {
            Prepared::LentValue(v, Some(home)) => values::refill(core, ctx, home, v)?,
            Prepared::LentMap(m, Some(home)) => {
                values::refill(core, ctx, home, &Value::from(core::mem::take(m)))?;
            }
            Prepared::LentList(l, Some(home)) => {
                values::refill(core, ctx, home, &Value::from(core::mem::take(l)))?;
            }
            Prepared::Buffer(bytes, Some(home)) => values::overwrite(ctx, home, bytes)?,
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
                object_for(ctx, shared, kind, Held::Object(Some(o)))?.into_value()
            }
            _ => return Err(bad_value("an object answered where the slot names none")),
        },
        _ => return Err(bad_value("an answer this binding does not carry")),
    })
}
