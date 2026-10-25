// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai's view of an object of a kind: a handle whose methods are the
//! kind's slots, called through core's `Caller`; Rhai values as a slot's
//! arguments and its answer as Rhai's; refusals both ways.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::sync::Arc;
use core::any::TypeId;
use core::cell::RefCell;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, AnswerType, Arg, ArgType, Caller, KindDescription, LentHandle, LentObject, Pass,
    RawObject, Scalar, SlotDescription, Tree,
};
use guatiao::{List, Map, Status, Value};
use guatiao_bridge::{BridgeError, Kinds, Limit};
use guatiao_libffi::Libffi;
use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Position};

use crate::values::{self, Exact, type_name};

// --- statuses and refusals --------------------------------------------------

/// Every status by the name a script spells it with.
pub(crate) const STATUSES: [(&str, Status); 10] = [
    ("ok", Status::GUATIAO_OK),
    ("bad_value", Status::GUATIAO_ERR_BAD_VALUE),
    ("alloc", Status::GUATIAO_ERR_ALLOC),
    ("wrong_kind", Status::GUATIAO_ERR_WRONG_KIND),
    ("not_found", Status::GUATIAO_ERR_NOT_FOUND),
    ("null", Status::GUATIAO_ERR_NULL),
    ("gone", Status::GUATIAO_ERR_GONE),
    ("unsupported", Status::GUATIAO_ERR_UNSUPPORTED),
    ("busy", Status::GUATIAO_ERR_BUSY),
    ("internal", Status::GUATIAO_ERR_INTERNAL),
];

/// A status's name, or its number for one this build does not name.
pub(crate) fn status_name(status: Status) -> String {
    STATUSES
        .iter()
        .find(|(_, s)| *s == status)
        .map_or_else(|| u32::from(status).to_string(), |(n, _)| (*n).to_string())
}

/// A refusal, as a script raises one (`throw guatiao::refuse(..)`) and
/// catches one from a slot of a handle: its status and its words.
#[derive(Debug, Clone)]
pub struct Refusal {
    /// The status.
    pub status: Status,
    /// The words.
    pub message: String,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.message.is_empty() {
            f.write_str(&status_name(self.status))
        } else {
            f.write_str(&self.message)
        }
    }
}

impl From<&ProviderError> for Refusal {
    fn from(e: &ProviderError) -> Refusal {
        Refusal {
            status: e.status,
            message: e.message().to_string(),
        }
    }
}

impl From<Refusal> for ProviderError {
    fn from(r: Refusal) -> ProviderError {
        ProviderError::new(r.status, &r.message)
    }
}

/// A script error carrying a refusal.
pub(crate) fn raised(r: Refusal) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(Dynamic::from(r), Position::NONE).into()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// What a script error came to. `limits` names the values the configured
/// limits have, for the words of a stop.
pub(crate) fn provider_error(e: &EvalAltResult, limits: &guatiao_bridge::Limits) -> ProviderError {
    let stopped = |limit: Limit, at: Option<u64>| -> ProviderError {
        BridgeError::Limit {
            limit,
            at: at.unwrap_or(0),
        }
        .into()
    };
    match e {
        EvalAltResult::ErrorInFunctionCall(_, _, inner, _)
        | EvalAltResult::ErrorInModule(_, inner, _) => provider_error(inner, limits),
        EvalAltResult::ErrorRuntime(thrown, _) => {
            if let Some(r) = thrown.read_lock::<Refusal>() {
                return r.clone().into();
            }
            if let Some(text) = thrown.read_lock::<ImmutableString>() {
                return ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, first_line(&text));
            }
            ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                &format!("a script threw a {}", type_name(thrown)),
            )
        }
        EvalAltResult::ErrorTooManyOperations(_) => stopped(Limit::Operations, limits.operations),
        EvalAltResult::ErrorStackOverflow(_) => stopped(Limit::Depth, limits.depth),
        EvalAltResult::ErrorDataTooLarge(..) => stopped(Limit::Memory, limits.memory),
        EvalAltResult::ErrorTerminated(token, _) => {
            let ms = limits
                .time
                .map(|t| u64::try_from(t.as_millis()).unwrap_or(u64::MAX));
            match token
                .read_lock::<ImmutableString>()
                .as_deref()
                .map(|s| s.as_str())
            {
                Some("time_ms") => stopped(Limit::Time, ms),
                Some(crate::limits::DEPTH_TOKEN) => stopped(Limit::Depth, limits.depth),
                _ => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, "the script was stopped"),
            }
        }
        other => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&other.to_string())),
    }
}

// --- the kinds an engine knows ------------------------------------------------

/// The kinds an engine was given, and a caller per kind.
pub(crate) struct Known {
    pub(crate) kinds: Kinds,
    callers: RefCell<BTreeMap<String, Arc<Caller>>>,
}

impl Known {
    pub(crate) fn new(kinds: Kinds) -> Known {
        Known {
            kinds,
            callers: RefCell::new(BTreeMap::new()),
        }
    }

    /// The kind named `name`, described.
    pub(crate) fn kind(&self, name: &str) -> Option<Arc<KindDescription>> {
        self.kinds.get(name).cloned()
    }

    /// A caller of the kind named `name`.
    pub(crate) fn caller(&self, name: &str) -> Result<Arc<Caller>, String> {
        if let Some(c) = self.callers.borrow().get(name) {
            return Ok(Arc::clone(c));
        }
        let described = self
            .kind(name)
            .ok_or_else(|| format!("no description of the kind `{name}` was given"))?;
        let made = Arc::new(Caller::new(described, &Libffi).map_err(|e| e.to_string())?);
        self.callers
            .borrow_mut()
            .insert(name.to_string(), Arc::clone(&made));
        Ok(made)
    }

    /// Every slot name of every kind, with each arity it has.
    fn slots(&self) -> BTreeSet<(String, usize)> {
        let mut all = BTreeSet::new();
        for name in self.kinds.names() {
            if let Some(kind) = self.kinds.get(name) {
                for slot in kind.slots() {
                    all.insert((slot.name().to_string(), slot.args().len()));
                }
            }
        }
        all
    }
}

// --- handles ------------------------------------------------------------------

/// What a handle calls.
enum Target {
    /// An object handed over, until it is handed on.
    Object(Option<RawObject>),
    /// An object lent for a call that may have returned.
    Lent(LentHandle),
}

struct Inner {
    caller: Arc<Caller>,
    target: RefCell<Target>,
}

/// An object of a described kind that a host handed to a script (a
/// listener) or that a slot answered: its methods are the kind's slots
/// (`listener.heard("hi")`). A copy of it is the same object.
#[derive(Clone)]
pub struct Handle(Rc<Inner>);

fn busy() -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_BUSY,
        "the object is already in use: a call re-entered it",
    )
}

impl Handle {
    fn new(target: Target, caller: Arc<Caller>) -> Handle {
        Handle(Rc::new(Inner {
            caller,
            target: RefCell::new(target),
        }))
    }

    pub(crate) fn object(caller: Arc<Caller>, object: RawObject) -> Handle {
        Handle::new(Target::Object(Some(object)), caller)
    }

    pub(crate) fn lent(caller: Arc<Caller>, lent: LentHandle) -> Handle {
        Handle::new(Target::Lent(lent), caller)
    }

    /// The object, if this handle still holds it: what an answer that is a
    /// handle hands back.
    pub(crate) fn take_object(&self, kind: &str) -> Result<RawObject, ProviderError> {
        if self.0.caller.kind().name() != kind {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_WRONG_KIND,
                &format!(
                    "an object of `{kind}` is answered, not of `{}`",
                    self.0.caller.kind().name()
                ),
            ));
        }
        match &mut *self.0.target.try_borrow_mut().map_err(|_| busy())? {
            Target::Object(o) => o.take().ok_or_else(|| {
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

    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        let caller = &self.0.caller;
        // A lent object is the host's to guard: only a held one is borrowed
        // for the length of the call.
        let lent = match &*self.0.target.try_borrow().map_err(|_| busy())? {
            Target::Lent(handle) => Some(handle.clone()),
            Target::Object(_) => None,
        };
        if let Some(handle) = lent {
            return handle.call(caller, slot, args);
        }
        match &mut *self.0.target.try_borrow_mut().map_err(|_| busy())? {
            Target::Object(Some(object)) => object.call(caller, slot, args),
            _ => Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the object was handed over",
            )),
        }
    }
}

/// Registers the methods of every slot of `known`'s kinds on [`Handle`]:
/// `handle.slot(args..)` calls it, raising a [`Refusal`] when it fails.
pub(crate) fn register(engine: &mut Engine, known: &Rc<Known>) {
    engine.register_type_with_name::<Handle>("guatiao::handle");
    for (name, arity) in known.slots() {
        let mut types = vec![TypeId::of::<Handle>()];
        types.extend(core::iter::repeat_n(TypeId::of::<Dynamic>(), arity));
        let known = Rc::clone(known);
        let slot = name.clone();
        engine.register_raw_fn(name, types, move |_ctx, args| {
            let handle = args[0].clone().cast::<Handle>();
            let rest: Vec<Dynamic> = args[1..].iter_mut().map(|a| core::mem::take(*a)).collect();
            call_slot(&known, &handle, &slot, rest).map_err(|e| raised(Refusal::from(&e)))
        });
    }
}

/// `handle.slot(args)`: the slot called, its answer as Rhai's.
fn call_slot(
    known: &Known,
    handle: &Handle,
    name: &str,
    args: Vec<Dynamic>,
) -> Result<Dynamic, ProviderError> {
    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    let kind = Arc::clone(handle.0.caller.kind());
    let (slot, described) = kind
        .slot(name)
        .ok_or_else(|| bad(format!("the `{}` kind has no slot `{name}`", kind.name())))?;
    if described.args().len() != args.len() {
        return Err(bad(format!(
            "`{}` of `{}` takes {} arguments, not {}",
            name,
            kind.name(),
            described.args().len(),
            args.len()
        )));
    }
    let mut prepared = prepare(described, args)?;
    let answer = with_args(&mut prepared, |args| handle.call(slot, args));
    settle(prepared);
    answer_to_dynamic(known, described, answer?)
}

// --- Rhai values as a slot's arguments ------------------------------------------

/// One argument, held while the call runs.
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
    /// An object this side owns, lent for the call and put back after.
    LentOwned(Option<RawObject>, Handle),
    /// A lent object passed on.
    LentPassed(LentHandle),
}

fn integer_of(d: &Dynamic, what: &str) -> Result<i128, String> {
    if let Ok(i) = d.as_int() {
        return Ok(i128::from(i));
    }
    if let Ok(f) = d.as_float()
        && f.fract() == 0.0
        && f.abs() < 1.8e19
    {
        return Ok(f as i128);
    }
    if let Some(n) = d.read_lock::<Exact>() {
        return n
            .0
            .parse::<i128>()
            .map_err(|_| format!("{what}: `{}` is not a whole number", n.0));
    }
    Err(format!("{what} is a whole number, not a {}", type_name(d)))
}

fn scalar(d: &Dynamic, ty: Scalar, what: &str) -> Result<Prepared, String> {
    Ok(match ty {
        Scalar::Bool => Prepared::Bool(
            d.as_bool()
                .map_err(|_| format!("{what} is a boolean, not a {}", type_name(d)))?,
        ),
        Scalar::F32 | Scalar::F64 => {
            if let Ok(f) = d.as_float() {
                Prepared::Float(f)
            } else if let Ok(i) = d.as_int() {
                Prepared::Float(i as f64)
            } else if let Some(n) = d.read_lock::<Exact>() {
                Prepared::Float(n.float())
            } else {
                return Err(format!("{what} is a number, not a {}", type_name(d)));
            }
        }
        Scalar::U8 | Scalar::U16 | Scalar::U32 | Scalar::U64 | Scalar::Usize => {
            let n = integer_of(d, what)?;
            Prepared::Uint(u64::try_from(n).map_err(|_| format!("{what}: {n} does not fit"))?)
        }
        _ => {
            let n = integer_of(d, what)?;
            Prepared::Int(i64::try_from(n).map_err(|_| format!("{what}: {n} does not fit"))?)
        }
    })
}

fn tree_of(d: &Dynamic, tree: Tree, what: &str) -> Result<Value, String> {
    let value = values::to_value(d)?;
    let fits = match tree {
        Tree::Map => guatiao::value::convert::TryAsRef::<Map>::try_as_ref(&value).is_some(),
        Tree::List => guatiao::value::convert::TryAsRef::<List>::try_as_ref(&value).is_some(),
        _ => true,
    };
    if !fits {
        return Err(format!(
            "{what} is a {}",
            if tree == Tree::Map { "map" } else { "list" }
        ));
    }
    Ok(value)
}

/// An object argument: a handle handed over, lent, or passed on.
fn object_arg(d: Dynamic, kind: &str, pass: Pass, what: &str) -> Result<Prepared, ProviderError> {
    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    let shown = type_name(&d);
    let Some(handle) = d.try_cast::<Handle>() else {
        return Err(bad(format!(
            "{what} is an object of `{kind}` (a handle), not a {shown}"
        )));
    };
    if handle.0.caller.kind().name() != kind {
        return Err(bad(format!(
            "{what} is a `{kind}`, not a `{}`",
            handle.0.caller.kind().name()
        )));
    }
    let mut target = handle.0.target.try_borrow_mut().map_err(|_| busy())?;
    match (&mut *target, pass) {
        (Target::Object(object), Pass::Taken) => {
            Ok(Prepared::Object(Some(object.take().ok_or_else(|| {
                bad(format!("{what} was already handed over"))
            })?)))
        }
        (Target::Object(object), _) => {
            let taken = object
                .take()
                .ok_or_else(|| bad(format!("{what} was already handed over")))?;
            drop(target);
            Ok(Prepared::LentOwned(Some(taken), handle))
        }
        (Target::Lent(lent), Pass::Lent) => Ok(Prepared::LentPassed(lent.clone())),
        (Target::Lent(_), _) => Err(bad(format!(
            "{what} was lent to this call, and cannot be handed over"
        ))),
    }
}

/// Rhai values as `slot`'s arguments, held for the call.
fn prepare(slot: &SlotDescription, args: Vec<Dynamic>) -> Result<Vec<Prepared>, ProviderError> {
    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    let mut out = Vec::with_capacity(args.len());
    for (d, described) in args.into_iter().zip(slot.args()) {
        let what = format!("`{}`'s argument `{}`", slot.name(), described.name());
        out.push(match described.ty() {
            ArgType::Scalar(s) => scalar(&d, *s, &what).map_err(bad)?,
            ArgType::Str => {
                if let Some(s) = d.read_lock::<ImmutableString>() {
                    Prepared::Str(s.to_string())
                } else if let Ok(c) = d.as_char() {
                    Prepared::Str(c.to_string())
                } else {
                    return Err(bad(format!("{what} is text, not a {}", type_name(&d))));
                }
            }
            ArgType::Bytes => {
                if let Some(b) = d.read_lock::<rhai::Blob>() {
                    Prepared::Bytes(b.clone())
                } else if let Some(s) = d.read_lock::<ImmutableString>() {
                    Prepared::Bytes(s.as_bytes().to_vec())
                } else {
                    return Err(bad(format!("{what} is a blob, not a {}", type_name(&d))));
                }
            }
            ArgType::Tree {
                tree,
                pass,
                nullable,
            } => {
                if d.is_unit() && *nullable && *pass == Pass::Borrowed {
                    Prepared::Value(None)
                } else if *pass == Pass::Lent {
                    return Err(bad(format!(
                        "{what} is lent to the slot by its caller; a script cannot lend a value"
                    )));
                } else {
                    let value = tree_of(&d, *tree, &what).map_err(bad)?;
                    match (tree, pass) {
                        (Tree::Map, Pass::Borrowed) => {
                            Prepared::Map(Map::try_from(value).unwrap_or_default())
                        }
                        (Tree::List, Pass::Borrowed) => {
                            Prepared::List(List::try_from(value).unwrap_or_default())
                        }
                        (_, Pass::Borrowed) => Prepared::Value(Some(value)),
                        (Tree::Map, _) => Prepared::TakenMap(Map::try_from(value).ok()),
                        (Tree::List, _) => Prepared::TakenList(List::try_from(value).ok()),
                        (_, _) => Prepared::TakenValue(Some(value)),
                    }
                }
            }
            ArgType::Object { kind, pass } => object_arg(d, kind, *pass, &what)?,
            ArgType::BytesMut => {
                return Err(bad(format!(
                    "{what} is a buffer the slot's caller owns; a script cannot lend one"
                )));
            }
            _ => {
                return Err(bad(format!(
                    "{what} has a type this binding does not carry"
                )));
            }
        });
    }
    Ok(out)
}

/// Runs `f` with the arguments `prepared` holds, as core takes them.
fn with_args<R>(prepared: &mut [Prepared], f: impl FnOnce(Vec<Arg<'_>>) -> R) -> R {
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

/// After the call: each object lent from a handle is put back in it.
fn settle(prepared: Vec<Prepared>) {
    for p in prepared {
        if let Prepared::LentOwned(Some(object), handle) = p
            && let Ok(mut target) = handle.0.target.try_borrow_mut()
            && let Target::Object(slot) = &mut *target
        {
            *slot = Some(object);
        }
    }
}

// --- a slot's answer as Rhai's ----------------------------------------------------

/// What `slot` answered, as Rhai's.
fn answer_to_dynamic(
    known: &Known,
    slot: &SlotDescription,
    answer: Answer,
) -> Result<Dynamic, ProviderError> {
    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    Ok(match answer {
        Answer::Nothing => Dynamic::UNIT,
        Answer::Bool(b) => Dynamic::from(b),
        Answer::Int(n) => values::to_dynamic(&Value::from(n)).map_err(bad)?,
        Answer::Uint(n) => values::to_dynamic(&Value::from(n)).map_err(bad)?,
        Answer::Float(x) => Dynamic::from(x),
        Answer::Text(t) => Dynamic::from(ImmutableString::from(t)),
        Answer::Value(v) => values::to_dynamic(&v).map_err(bad)?,
        Answer::Map(m) => values::to_dynamic(&Value::from(m)).map_err(bad)?,
        Answer::List(l) => values::to_dynamic(&Value::from(l)).map_err(bad)?,
        Answer::Object(o) => match slot.answer() {
            Some(AnswerType::Object { kind }) => {
                Dynamic::from(Handle::object(known.caller(kind).map_err(bad)?, o))
            }
            _ => return Err(bad("an object answered where the slot names none".into())),
        },
        _ => return Err(bad("an answer this binding does not carry".into())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_is_named_both_ways() {
        for (name, status) in STATUSES {
            assert_eq!(status_name(status), name);
        }
        assert_eq!(status_name(Status::from(42)), "42");
    }

    #[test]
    fn a_refusal_crosses_with_its_status() {
        let e: ProviderError = Refusal {
            status: Status::GUATIAO_ERR_NOT_FOUND,
            message: "no such greeting".into(),
        }
        .into();
        assert_eq!(e.status, Status::GUATIAO_ERR_NOT_FOUND);
        assert_eq!(e.message(), "no such greeting");
    }
}
