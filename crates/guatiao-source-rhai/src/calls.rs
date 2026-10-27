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
    Answer, AnswerType, Arg, ArgType, Caller, KindDescription, LentHandle, Pass, RawObject,
    SlotDescription,
};
use guatiao::{Status, Value};
use guatiao_bridge::{BridgeError, Held, Kinds, Limit, Reading};
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

struct Inner {
    caller: Arc<Caller>,
    held: RefCell<Held>,
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
    fn new(held: Held, caller: Arc<Caller>) -> Handle {
        Handle(Rc::new(Inner {
            caller,
            held: RefCell::new(held),
        }))
    }

    pub(crate) fn object(caller: Arc<Caller>, object: RawObject) -> Handle {
        Handle::new(Held::Object(Some(object)), caller)
    }

    pub(crate) fn lent(caller: Arc<Caller>, lent: LentHandle) -> Handle {
        Handle::new(Held::Lent(lent), caller)
    }

    /// The object, if this handle still holds it: what an answer that is a
    /// handle hands back.
    pub(crate) fn take_object(&self, kind: &str) -> Result<RawObject, ProviderError> {
        self.0
            .held
            .try_borrow_mut()
            .map_err(|_| busy())?
            .hand_back(&self.0.caller, kind)
    }

    fn call(&self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        let caller = &self.0.caller;
        // A lent object is the host's to guard: only a held one is borrowed
        // for the length of the call.
        let lent = match &*self.0.held.try_borrow().map_err(|_| busy())? {
            Held::Lent(handle) => Some(handle.clone()),
            _ => None,
        };
        if let Some(handle) = lent {
            return handle.call(caller, slot, args);
        }
        self.0
            .held
            .try_borrow_mut()
            .map_err(|_| busy())?
            .call(caller, slot, args)
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
    let answer = Prepared::with_args(&mut prepared, |args| handle.call(slot, args));
    settle(prepared);
    answer_to_dynamic(known, described, answer?)
}

// --- Rhai values as a slot's arguments ------------------------------------------

/// One argument, held while the call runs: an object lent from a handle
/// goes back into it.
type Prepared = guatiao_bridge::PreparedArg<(), (), Handle>;

/// A Rhai value as a scalar argument reads it.
fn reading(d: &Dynamic) -> Reading {
    if let Ok(b) = d.as_bool() {
        Reading::Bool(b)
    } else if let Ok(i) = d.as_int() {
        Reading::Int(i)
    } else if let Ok(f) = d.as_float() {
        Reading::Float(f)
    } else if let Some(n) = d.read_lock::<Exact>() {
        Reading::Exact(n.0.clone())
    } else {
        Reading::Other
    }
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
    let mut held = handle.0.held.try_borrow_mut().map_err(|_| busy())?;
    held.argument(&handle.0.caller, kind, pass, handle.clone(), what)
        .map_err(bad)
}

/// Rhai values as `slot`'s arguments, held for the call.
fn prepare(slot: &SlotDescription, args: Vec<Dynamic>) -> Result<Vec<Prepared>, ProviderError> {
    let bad = |why: String| ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why);
    let mut out = Vec::with_capacity(args.len());
    for (d, described) in args.into_iter().zip(slot.args()) {
        let what = format!("`{}`'s argument `{}`", slot.name(), described.name());
        out.push(match described.ty() {
            ArgType::Scalar(s) => {
                Prepared::scalar(reading(&d), &type_name(&d), *s, &what).map_err(bad)?
            }
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
                    let value = values::to_value(&d).map_err(bad)?;
                    Prepared::tree(value, *tree, *pass, None, &what).map_err(bad)?
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

/// After the call: each object lent from a handle is put back in it.
fn settle(prepared: Vec<Prepared>) {
    for p in prepared {
        if let Prepared::LentOwned(Some(object), handle) = p
            && let Ok(mut held) = handle.0.held.try_borrow_mut()
        {
            held.put_back(object);
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
