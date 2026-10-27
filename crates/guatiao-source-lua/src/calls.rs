// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Lua's view of a kind: a provider, an object or a lent object as a
//! userdata whose methods are the kind's slots, called through core's
//! `Caller`; Lua values as a slot's arguments and its answer as Lua's;
//! refusals both ways.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    Answer, AnswerType, Arg, ArgType, Caller, KindDescription, Pass, RawObject, SlotDescription,
    Tree,
};
use guatiao::{Status, Value};
use guatiao_bridge::{BridgeError, Held, Kinds, Limit, ObjectHome, PreparedArgs, Reading};
use guatiao_libffi::Libffi;
use mlua::prelude::*;
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};

use crate::values::{self, Empty};

// --- statuses and refusals --------------------------------------------------

/// Every status by the name Lua code spells it with.
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

/// A status from its name or its number.
pub(crate) fn status_of(v: &LuaValue) -> LuaResult<Status> {
    match v {
        LuaValue::String(s) => {
            let name = s.to_str()?;
            STATUSES
                .iter()
                .find(|(n, _)| *n == &*name)
                .map(|(_, s)| *s)
                .ok_or_else(|| LuaError::runtime(format!("`{}` is not a status", &*name)))
        }
        LuaValue::Integer(i) => u32::try_from(*i)
            .map(Status::from)
            .map_err(|_| LuaError::runtime("a status is a number from 0")),
        other => Err(LuaError::runtime(format!(
            "a status is a name or a number, not a {}",
            other.type_name()
        ))),
    }
}

/// A refusal, as Lua code sees and raises one (`guatiao.error`): its
/// status by name and number, and its words.
#[derive(Debug, Clone)]
pub struct Refusal {
    /// The status.
    pub status: Status,
    /// The words.
    pub message: String,
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for Refusal {}

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

impl UserData for Refusal {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.error");
        fields.add_field_method_get("status", |_, this| Ok(status_name(this.status)));
        fields.add_field_method_get("code", |_, this| Ok(u32::from(this.status)));
        fields.add_field_method_get("message", |_, this| Ok(this.message.clone()));
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(if this.message.is_empty() {
                status_name(this.status)
            } else {
                this.message.clone()
            })
        });
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// What a Lua error came to, walking through the wrappers mlua puts
/// around one that crossed a Rust function.
pub(crate) fn provider_error(e: &LuaError) -> ProviderError {
    match e {
        LuaError::CallbackError { cause, .. } | LuaError::WithContext { cause, .. } => {
            provider_error(cause)
        }
        LuaError::ExternalError(inner) => {
            if let Some(r) = inner.downcast_ref::<Refusal>() {
                return r.clone().into();
            }
            if let Some(b) = inner.downcast_ref::<BridgeError>() {
                return b.clone().into();
            }
            ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&inner.to_string()))
        }
        LuaError::MemoryError(_) => {
            ProviderError::new(Status::GUATIAO_ERR_ALLOC, "the Lua state ran out of memory")
        }
        LuaError::RuntimeError(m) | LuaError::SyntaxError { message: m, .. } => {
            ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(m))
        }
        other => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&other.to_string())),
    }
}

/// What an error value a protected call caught came to: a
/// `guatiao.error` is its status; text or anything else is
/// `GUATIAO_ERR_INTERNAL` in its words; Lua's own memory error is the
/// memory limit when one is set.
pub(crate) fn caught(value: &LuaValue, memory_limit: Option<u64>) -> ProviderError {
    match value {
        LuaValue::UserData(ud) if ud.is::<Refusal>() => match ud.borrow::<Refusal>() {
            Ok(r) => r.clone().into(),
            Err(e) => provider_error(&e),
        },
        LuaValue::Error(e) => provider_error(e),
        LuaValue::String(s) => {
            let text = s.to_string_lossy();
            match memory_limit {
                Some(at) if text == "not enough memory" => BridgeError::Limit {
                    limit: Limit::Memory,
                    at,
                }
                .into(),
                _ => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, first_line(&text)),
            }
        }
        LuaValue::Nil => ProviderError::new(Status::GUATIAO_ERR_INTERNAL, "an error with no words"),
        other => ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            &format!("an error that is a {}", other.type_name()),
        ),
    }
}

/// A refusal a function answered with `nil, why`: text is
/// `GUATIAO_ERR_BAD_VALUE`, a `guatiao.error` its own status.
pub(crate) fn answered_refusal(why: &LuaValue) -> ProviderError {
    match why {
        LuaValue::UserData(ud) if ud.is::<Refusal>() => match ud.borrow::<Refusal>() {
            Ok(r) => r.clone().into(),
            Err(e) => provider_error(&e),
        },
        LuaValue::String(s) => {
            ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &s.to_string_lossy())
        }
        other => caught(other, None),
    }
}

// --- the kinds a state knows ---------------------------------------------------

/// The kinds a Lua state was given, and a caller per kind.
#[derive(Default)]
pub(crate) struct Known {
    pub(crate) kinds: Kinds,
    callers: BTreeMap<String, Arc<Caller>>,
}

/// The state's kinds.
pub(crate) fn known(lua: &Lua) -> LuaResult<mlua::AppDataRefMut<'_, Known>> {
    if lua.app_data_ref::<Known>().is_none() {
        lua.set_app_data(Known::default());
    }
    lua.app_data_mut::<Known>()
        .ok_or_else(|| LuaError::runtime("the kinds are in use"))
}

/// The kind named `name`, described.
pub(crate) fn kind(lua: &Lua, name: &str) -> LuaResult<Arc<KindDescription>> {
    known(lua)?
        .kinds
        .get(name)
        .cloned()
        .ok_or_else(|| LuaError::runtime(format!("no description of the kind `{name}` was given")))
}

/// A caller of the kind named `name`.
pub(crate) fn caller(lua: &Lua, name: &str) -> LuaResult<Arc<Caller>> {
    let mut known = known(lua)?;
    if let Some(c) = known.callers.get(name) {
        return Ok(Arc::clone(c));
    }
    let described = known.kinds.get(name).cloned().ok_or_else(|| {
        LuaError::runtime(format!("no description of the kind `{name}` was given"))
    })?;
    let made =
        Arc::new(Caller::new(described, &Libffi).map_err(|e| LuaError::runtime(e.to_string()))?);
    known.callers.insert(name.to_string(), Arc::clone(&made));
    Ok(made)
}

/// A kind, as Lua code names one to `bind`: its name, whether it is an
/// object kind, and its slots.
#[derive(Debug, Clone)]
pub struct KindView(pub(crate) Arc<KindDescription>);

impl UserData for KindView {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.kind");
        fields.add_field_method_get("name", |_, this| Ok(this.0.name().to_string()));
        fields.add_field_method_get("object", |_, this| Ok(this.0.object()));
        fields.add_field_method_get("slots", |lua, this| {
            let t =
                lua.create_sequence_from(this.0.slots().iter().map(|s| s.name().to_string()))?;
            Ok(t)
        });
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            Ok(format!("guatiao.kind: {}", this.0.name()))
        });
    }
}

/// A kind named by Lua code: a `KindView` or its name.
pub(crate) fn kind_named(lua: &Lua, v: &LuaValue) -> LuaResult<Arc<KindDescription>> {
    match v {
        LuaValue::UserData(ud) if ud.is::<KindView>() => {
            Ok(Arc::clone(&ud.borrow::<KindView>()?.0))
        }
        LuaValue::String(s) => kind(lua, &s.to_str()?),
        other => Err(LuaError::runtime(format!(
            "a kind is named by a guatiao.kind or its name, not a {}",
            other.type_name()
        ))),
    }
}

// --- handles ---------------------------------------------------------------------

/// A provider, an object or a lent object of a described kind: its slots
/// are its methods (`greeter:greet("ana")`).
pub struct Handle {
    pub(crate) held: Held,
    pub(crate) caller: Arc<Caller>,
}

impl Handle {
    fn kind(&self) -> &Arc<KindDescription> {
        self.caller.kind()
    }

    /// Calls slot `slot` with `args`.
    fn call(&mut self, slot: usize, args: Vec<Arg<'_>>) -> Result<Answer, ProviderError> {
        self.held.call(&self.caller, slot, args)
    }
}

impl UserData for Handle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.handle");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_function(
            MetaMethod::Index,
            |lua, (_, name): (LuaAnyUserData, LuaValue)| -> LuaResult<LuaValue> {
                let LuaValue::String(name) = name else {
                    return Ok(LuaValue::Nil);
                };
                let name = name.to_str()?.to_string();
                Ok(LuaValue::Function(lua.create_function(
                    move |lua, args: LuaMultiValue| call_slot(lua, &name, args),
                )?))
            },
        );
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let what = match this.held {
                Held::Provider(_) => "provider",
                Held::Object(_) => "object",
                Held::Lent(_) => "lent object",
            };
            Ok(format!(
                "guatiao.handle: a {what} of {}",
                this.kind().name()
            ))
        });
    }
}

/// `handle:slot(args...)`: the slot called, its answer as Lua's. A slot
/// that can fail answers `nil, guatiao.error` for a refusal, and `true`
/// for success when it answers nothing.
fn call_slot(lua: &Lua, name: &str, args: LuaMultiValue) -> LuaResult<LuaMultiValue> {
    let mut args = args.into_iter();
    let Some(LuaValue::UserData(me)) = args.next() else {
        return Err(LuaError::runtime(format!(
            "`{name}` is a method: call it as handle:{name}(...)"
        )));
    };
    let kind = Arc::clone(me.borrow::<Handle>()?.kind());
    let (slot, described) = kind.slot(name).ok_or_else(|| {
        LuaError::runtime(format!("the `{}` kind has no slot `{name}`", kind.name()))
    })?;
    let rest: Vec<LuaValue> = args.collect();
    let mut prepared = prepare(lua, described, rest)?;
    let answer = {
        let mut handle = me.borrow_mut::<Handle>()?;
        Prepared::with_args(&mut prepared, |args| handle.call(slot, args))
    };
    settle(lua, prepared)?;
    match answer {
        Ok(answer) => {
            let v = answer_to_lua(lua, described, answer)?;
            let mut out = LuaMultiValue::new();
            match (v, described.fallible()) {
                (LuaValue::Nil, true) if described.answer().is_none() => {
                    out.push_back(LuaValue::Boolean(true));
                }
                (v, _) => out.push_back(v),
            }
            Ok(out)
        }
        Err(e) if described.fallible() => {
            let mut out = LuaMultiValue::new();
            out.push_back(LuaValue::Nil);
            out.push_back(LuaValue::UserData(lua.create_userdata(Refusal::from(&e))?));
            Ok(out)
        }
        Err(e) => Err(LuaError::external(Refusal::from(&e))),
    }
}

// --- Lua values as a slot's arguments -----------------------------------------------

/// The caller's buffer for a slot that writes bytes into one
/// (`bytes_mut`): `guatiao.buffer(n)`; `tostring` gives what it holds.
#[derive(Debug, Clone)]
pub struct BufferView(pub Vec<u8>);

impl UserData for BufferView {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field("__name", "guatiao.buffer");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| Ok(this.0.len()));
        methods.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            lua.create_string(&this.0)
        });
        // `buf:write(text [, at])`: copies as much as fits from 1-based
        // `at`, and answers how many bytes it wrote.
        methods.add_method_mut(
            "write",
            |_, this, (text, at): (LuaString, Option<usize>)| {
                let at = at.unwrap_or(1).max(1) - 1;
                let bytes = text.as_bytes();
                let room = this.0.len().saturating_sub(at);
                let n = room.min(bytes.len());
                if n > 0 {
                    this.0[at..at + n].copy_from_slice(&bytes[..n]);
                }
                Ok(n)
            },
        );
        // `buf:sub(i [, j])`: as `string.sub` over what it holds.
        methods.add_method(
            "sub",
            |lua, this, (i, j): (Option<usize>, Option<usize>)| {
                let len = this.0.len();
                let i = i.unwrap_or(1).max(1);
                let j = j.unwrap_or(len).min(len);
                if i > j {
                    return lua.create_string("");
                }
                lua.create_string(&this.0[i - 1..j])
            },
        );
    }
}

/// The handle an object argument was taken out of, or none for a table
/// served for the call alone, which is let go after it.
pub(crate) struct Home(Option<LuaAnyUserData>);

impl ObjectHome for Home {
    fn give_back(self, object: RawObject) {
        if let Some(ud) = self.0
            && let Ok(mut handle) = ud.borrow_mut::<Handle>()
        {
            handle.held.put_back(object);
        }
    }
}

/// One argument, held while the call runs: a lent tree goes back into its
/// table, a buffer into its `guatiao.buffer`, and an object lent from a
/// handle into that handle.
pub(crate) type Prepared = guatiao_bridge::PreparedArg<LuaTable, LuaAnyUserData, Home>;

/// A call's arguments: whatever ends the call, an object taken out of a
/// handle and not handed over goes back in.
pub(crate) type Args = PreparedArgs<LuaTable, LuaAnyUserData, Home>;

/// A Lua value as a scalar argument reads it.
fn reading(v: &LuaValue) -> LuaResult<Reading> {
    Ok(match v {
        LuaValue::Boolean(b) => Reading::Bool(*b),
        LuaValue::Integer(i) => Reading::Int(*i),
        LuaValue::Number(f) => Reading::Float(*f),
        LuaValue::UserData(ud) if ud.is::<values::Exact>() => {
            Reading::Exact(ud.borrow::<values::Exact>()?.0.clone())
        }
        _ => Reading::Other,
    })
}

fn text_of(v: &LuaValue, what: &str) -> LuaResult<String> {
    match v {
        LuaValue::String(s) => Ok(s
            .to_str()
            .map_err(|_| LuaError::runtime(format!("{what} is not UTF-8 text")))?
            .to_string()),
        LuaValue::Integer(_) | LuaValue::Number(_) => Ok(v.to_string()?),
        other => Err(LuaError::runtime(format!(
            "{what} is text, not a {}",
            other.type_name()
        ))),
    }
}

fn bytes_of(v: &LuaValue, what: &str) -> LuaResult<Vec<u8>> {
    match v {
        LuaValue::String(s) => Ok(s.as_bytes().to_vec()),
        LuaValue::UserData(ud) if ud.is::<values::Bytes>() => {
            Ok(ud.borrow::<values::Bytes>()?.0.clone())
        }
        other => Err(LuaError::runtime(format!(
            "{what} is bytes (a string), not a {}",
            other.type_name()
        ))),
    }
}

/// An object argument from Lua: a handle handed over or lent, or a Lua
/// table of the kind's methods, served for as long as the callee holds it.
fn object_arg(lua: &Lua, v: LuaValue, kind: &str, pass: Pass, what: &str) -> LuaResult<Prepared> {
    match v {
        LuaValue::UserData(ud) if ud.is::<Handle>() => {
            let mut handle = ud.borrow_mut::<Handle>()?;
            let Handle { held, caller } = &mut *handle;
            held.argument(caller, kind, pass, Home(Some(ud.clone())), what)
                .map_err(LuaError::runtime)
        }
        LuaValue::Table(t) => {
            let object = crate::served::object(lua, kind, t)?;
            Ok(match pass {
                Pass::Taken => Prepared::Object(Some(object), Home(None)),
                _ => Prepared::LentOwned(Some(object), Home(None)),
            })
        }
        other => Err(LuaError::runtime(format!(
            "{what} is an object of `{kind}` (a handle or a table of its methods), not a {}",
            other.type_name()
        ))),
    }
}

/// Lua values as `slot`'s arguments, held for the call.
pub(crate) fn prepare(lua: &Lua, slot: &SlotDescription, args: Vec<LuaValue>) -> LuaResult<Args> {
    let mut args = args.into_iter();
    let mut out = Args::with_capacity(slot.args().len());
    for described in slot.args() {
        let v = args.next().unwrap_or(LuaValue::Nil);
        let what = format!("`{}`'s argument `{}`", slot.name(), described.name());
        out.push(match described.ty() {
            ArgType::Scalar(s) => Prepared::scalar(reading(&v)?, v.type_name(), *s, &what)
                .map_err(LuaError::runtime)?,
            ArgType::Str => Prepared::Str(text_of(&v, &what)?),
            ArgType::Bytes => Prepared::Bytes(bytes_of(&v, &what)?),
            ArgType::BytesMut => match v {
                LuaValue::UserData(ud) if ud.is::<BufferView>() => {
                    let bytes = ud.borrow::<BufferView>()?.0.clone();
                    Prepared::Buffer(bytes, Some(ud))
                }
                LuaValue::Integer(n) => {
                    Prepared::Buffer(vec![0; usize::try_from(n).unwrap_or(0)], None)
                }
                other => {
                    return Err(LuaError::runtime(format!(
                        "{what} is a guatiao.buffer, not a {}",
                        other.type_name()
                    )));
                }
            },
            ArgType::Tree {
                tree,
                pass,
                nullable,
            } => {
                let lent_table = match (&v, pass) {
                    (LuaValue::Table(t), Pass::Lent) => Some(t.clone()),
                    _ => None,
                };
                if v.is_nil() && *nullable && *pass == Pass::Borrowed {
                    Prepared::Value(None)
                } else {
                    let empty = if *tree == Tree::List {
                        Empty::List
                    } else {
                        Empty::Map
                    };
                    let value = values::to_value_as(lua, &v, empty)?;
                    Prepared::tree(value, *tree, *pass, lent_table, &what)
                        .map_err(LuaError::runtime)?
                }
            }
            ArgType::Object { kind, pass } => object_arg(lua, v, kind, *pass, &what)?,
            _ => {
                return Err(LuaError::runtime(format!(
                    "{what} has a type this binding does not carry"
                )));
            }
        });
    }
    Ok(out)
}

/// After the call: each lent tree written back over the Lua table it came
/// from, each buffer back into its `guatiao.buffer`; each object lent from
/// a handle goes back in as `prepared` drops, whether or not a write-back
/// failed.
pub(crate) fn settle(lua: &Lua, mut prepared: Args) -> LuaResult<()> {
    for p in prepared.iter_mut() {
        match p {
            Prepared::LentValue(v, Some(t)) => values::refill(lua, t, v)?,
            Prepared::LentMap(m, Some(t)) => {
                values::refill(lua, t, &Value::from(core::mem::take(m)))?;
            }
            Prepared::LentList(l, Some(t)) => {
                values::refill(lua, t, &Value::from(core::mem::take(l)))?;
            }
            Prepared::Buffer(b, Some(ud)) => {
                ud.borrow_mut::<BufferView>()?.0 = core::mem::take(b);
            }
            _ => {}
        }
    }
    Ok(())
}

// --- a slot's answer as Lua's --------------------------------------------------------

/// A handle on an object of the kind named.
pub(crate) fn object_handle(lua: &Lua, kind: &str, object: RawObject) -> LuaResult<LuaValue> {
    let caller = caller(lua, kind)?;
    Ok(LuaValue::UserData(lua.create_userdata(Handle {
        held: Held::Object(Some(object)),
        caller,
    })?))
}

/// What `slot` answered, as Lua's.
pub(crate) fn answer_to_lua(
    lua: &Lua,
    slot: &SlotDescription,
    answer: Answer,
) -> LuaResult<LuaValue> {
    Ok(match answer {
        Answer::Nothing => LuaValue::Nil,
        Answer::Bool(b) => LuaValue::Boolean(b),
        Answer::Int(n) => values::to_lua(lua, &Value::from(n))?,
        Answer::Uint(n) => values::to_lua(lua, &Value::from(n))?,
        Answer::Float(x) => LuaValue::Number(x),
        Answer::Text(t) => LuaValue::String(lua.create_string(&t)?),
        Answer::Value(v) => values::to_lua(lua, &v)?,
        Answer::Map(m) => values::to_lua(lua, &Value::from(m))?,
        Answer::List(l) => values::to_lua(lua, &Value::from(l))?,
        Answer::Object(o) => match slot.answer() {
            Some(AnswerType::Object { kind }) => object_handle(lua, kind, o)?,
            _ => {
                return Err(LuaError::runtime(
                    "an object answered where the slot names none",
                ));
            }
        },
        _ => return Err(LuaError::runtime("an answer this binding does not carry")),
    })
}

// --- Lua as the callee: what a Lua function answered -----------------------------------

/// What a Lua function's answer to a slot came to.
pub(crate) enum Outcome {
    Nothing,
    Value(Value),
    /// A table of an object kind's methods: the object it answers.
    Table(String, LuaTable),
    /// An object handed back as it came.
    Handed(RawObject),
}

/// What a Lua function returned to `slot`: `nil, why` is a refusal;
/// otherwise the first value, converted to the slot's answer.
pub(crate) fn outcome(
    lua: &Lua,
    slot: &SlotDescription,
    mut results: LuaMultiValue,
) -> Result<Outcome, ProviderError> {
    let first = results.pop_front().unwrap_or(LuaValue::Nil);
    if first.is_nil()
        && let Some(why) = results.pop_front()
        && !why.is_nil()
    {
        return Err(answered_refusal(&why));
    }
    let bad = |e: LuaError| provider_error(&e);
    Ok(match slot.answer() {
        None => Outcome::Nothing,
        Some(AnswerType::Object { kind }) => match first {
            LuaValue::Table(t) => Outcome::Table(kind.clone(), t),
            LuaValue::UserData(ud) if ud.is::<Handle>() => {
                let mut handle = ud.borrow_mut::<Handle>().map_err(bad)?;
                let Handle { held, caller } = &mut *handle;
                Outcome::Handed(held.hand_back(caller, kind).map_err(|e| {
                    if e.status == Status::GUATIAO_ERR_WRONG_KIND {
                        ProviderError::new(
                            Status::GUATIAO_ERR_WRONG_KIND,
                            &format!("`{}` answers an object of `{kind}`", slot.name()),
                        )
                    } else {
                        e
                    }
                })?)
            }
            other => {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "`{}` answers an object of `{kind}` (a table of its methods), not a {}",
                        slot.name(),
                        other.type_name()
                    ),
                ));
            }
        },
        Some(AnswerType::Tree(Tree::List)) => {
            Outcome::Value(values::to_value_as(lua, &first, Empty::List).map_err(bad)?)
        }
        Some(AnswerType::Text) => match &first {
            LuaValue::String(s) => Outcome::Value(Value::from(&*s.to_str().map_err(bad)?)),
            other => {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "`{}` answers text, not a {}",
                        slot.name(),
                        other.type_name()
                    ),
                ));
            }
        },
        Some(_) => Outcome::Value(values::to_value(lua, &first).map_err(bad)?),
    })
}

/// What a Lua function was handed for a lent argument, read back after
/// the call.
pub(crate) enum Received {
    /// Nothing to read back.
    Plain,
    /// A lent tree: the table Lua changed in place.
    Tree(LuaTable),
    /// A lent buffer.
    Buffer(LuaAnyUserData),
}

/// A Lua value as an argument a Lua function receives for `ty`: a lent
/// tree as a table read back after the call, a buffer as a
/// `guatiao.buffer`, bytes as a plain string.
pub(crate) fn received(lua: &Lua, ty: &ArgType, v: &Value) -> LuaResult<(LuaValue, Received)> {
    Ok(match ty {
        ArgType::Bytes => {
            let b: &[u8] = guatiao::value::convert::TryAsRef::try_as_ref(v).unwrap_or(&[]);
            (LuaValue::String(lua.create_string(b)?), Received::Plain)
        }
        ArgType::Tree {
            pass: Pass::Lent,
            tree,
            ..
        } => {
            let t = match values::to_lua(lua, v)? {
                LuaValue::Table(t) => t,
                _ => {
                    let t = lua.create_table()?;
                    if *tree == Tree::List {
                        values::mark_list(lua, &t)?;
                    } else {
                        values::mark_map(lua, &t)?;
                    }
                    t
                }
            };
            (LuaValue::Table(t.clone()), Received::Tree(t))
        }
        _ => (values::to_lua(lua, v)?, Received::Plain),
    })
}

/// A buffer a Lua function writes into.
pub(crate) fn buffer(lua: &Lua, bytes: Vec<u8>) -> LuaResult<(LuaValue, Received)> {
    let ud = lua.create_userdata(BufferView(bytes))?;
    Ok((LuaValue::UserData(ud.clone()), Received::Buffer(ud)))
}

/// What a lent argument holds after the call.
pub(crate) fn read_back(
    lua: &Lua,
    ty: &ArgType,
    received: &Received,
) -> LuaResult<Option<ReadBack>> {
    Ok(match received {
        Received::Plain => None,
        Received::Tree(t) => {
            let empty = match ty {
                ArgType::Tree {
                    tree: Tree::List, ..
                } => Empty::List,
                _ => Empty::Map,
            };
            Some(ReadBack::Tree(values::to_value_as(
                lua,
                &LuaValue::Table(t.clone()),
                empty,
            )?))
        }
        Received::Buffer(ud) => Some(ReadBack::Bytes(ud.borrow::<BufferView>()?.0.clone())),
    })
}

/// A lent argument's contents after the call.
pub(crate) enum ReadBack {
    Tree(Value),
    Bytes(Vec<u8>),
}

/// A scalar a slot was called with, as Lua's.
pub(crate) fn scalar_arg(lua: &Lua, arg: &Arg<'_>) -> LuaResult<Option<LuaValue>> {
    Ok(Some(match arg {
        Arg::Bool(b) => LuaValue::Boolean(*b),
        Arg::Int(n) => values::to_lua(lua, &Value::from(*n))?,
        Arg::Uint(n) => values::to_lua(lua, &Value::from(*n))?,
        Arg::Float(x) => LuaValue::Number(*x),
        Arg::Str(s) => LuaValue::String(lua.create_string(s)?),
        Arg::Bytes(b) => LuaValue::String(lua.create_string(b)?),
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use guatiao::value::convert::FromValue;

    use super::*;

    #[test]
    fn a_status_is_named_both_ways() {
        for (name, status) in STATUSES {
            assert_eq!(status_name(status), name);
        }
        assert_eq!(status_name(Status::from(42)), "42");
    }

    #[test]
    fn a_narrowed_answer_is_checked_by_core() {
        assert!(i64::from_value(&Value::from(5i64)).is_ok());
        assert!(u64::from_value(&Value::from(-1i64)).is_err());
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
