// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A call as a provider receives it, what it answers, and the host's
//! objects it is handed.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use guatiao::value::convert::{FromValue, TryAsMut, TryAsRef};
use guatiao::value::wire;
use guatiao::{Buffer, List, Map, Status, Value};

use crate::host;

/// Why a call was refused: a guatiao status and words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    status: Status,
    message: String,
}

impl Refusal {
    /// A refusal with this status.
    pub fn new(status: Status, message: impl Into<String>) -> Refusal {
        Refusal {
            status,
            message: message.into(),
        }
    }

    /// `GUATIAO_ERR_BAD_VALUE`: what the caller passed will not do.
    pub fn bad_value(message: impl Into<String>) -> Refusal {
        Refusal::new(Status::GUATIAO_ERR_BAD_VALUE, message)
    }

    /// `GUATIAO_ERR_NOT_FOUND`.
    pub fn not_found(message: impl Into<String>) -> Refusal {
        Refusal::new(Status::GUATIAO_ERR_NOT_FOUND, message)
    }

    /// `GUATIAO_ERR_UNSUPPORTED`: a slot or a method this does not answer.
    pub fn unsupported(message: impl Into<String>) -> Refusal {
        Refusal::new(Status::GUATIAO_ERR_UNSUPPORTED, message)
    }

    /// Its status.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Its words.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for Refusal {}

/// Plain words are a bad value, as `return nil, why` is in Lua.
impl From<&str> for Refusal {
    fn from(message: &str) -> Refusal {
        Refusal::bad_value(message)
    }
}

impl From<String> for Refusal {
    fn from(message: String) -> Refusal {
        Refusal::bad_value(message)
    }
}

/// What answers the calls of a provider, or of an object a provider
/// answered: one method, called with the kind and the slot named.
pub trait Handler: 'static {
    /// Answers one call.
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal>;
}

impl<F> Handler for F
where
    F: FnMut(&mut Call) -> Result<Answer, Refusal> + 'static,
{
    fn call(&mut self, call: &mut Call) -> Result<Answer, Refusal> {
        self(call)
    }
}

/// What a call answers.
#[non_exhaustive]
pub enum Answer {
    /// For a slot that answers nothing.
    Nothing,
    /// A value, which the host converts to the slot's answer: a number
    /// narrowed to its width, text, a tree.
    Value(Value),
    /// An object, for a slot that answers one: its calls come to this
    /// handler until the caller destroys it.
    Object(Box<dyn Handler>),
}

impl Answer {
    /// An object answering its kind's slots.
    pub fn object(handler: impl Handler) -> Answer {
        Answer::Object(Box::new(handler))
    }

    /// Anything that is a value.
    pub fn value(value: impl Into<Value>) -> Answer {
        Answer::Value(value.into())
    }
}

impl core::fmt::Debug for Answer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Answer::Nothing => f.write_str("Nothing"),
            Answer::Value(v) => f.debug_tuple("Value").field(v).finish(),
            Answer::Object(_) => f.write_str("Object(..)"),
        }
    }
}

impl From<()> for Answer {
    fn from((): ()) -> Answer {
        Answer::Nothing
    }
}

macro_rules! answer_from {
    ($($t:ty),*) => {$(
        impl From<$t> for Answer {
            fn from(v: $t) -> Answer {
                Answer::Value(Value::from(v))
            }
        }
    )*};
}

answer_from!(Value, Map, List, bool, i64, u64, &str, String);

/// One call: the kind and slot it is for and its arguments, one per
/// argument of the slot, in order.
#[derive(Debug)]
pub struct Call {
    kind: String,
    method: String,
    args: Vec<Value>,
    changed: Vec<bool>,
}

impl Call {
    /// A call as the host sent it.
    pub(crate) fn new(kind: String, method: String, args: Vec<Value>) -> Call {
        let changed = alloc::vec![false; args.len()];
        Call {
            kind,
            method,
            args,
            changed,
        }
    }

    /// The kind called.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The slot called.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// How many arguments it has.
    pub fn len(&self) -> usize {
        self.args.len()
    }

    /// Whether it has none.
    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    fn missing(&self, index: usize) -> Refusal {
        Refusal::bad_value(alloc::format!("`{}` has no argument {index}", self.method))
    }

    fn not(&self, index: usize, what: &str) -> Refusal {
        Refusal::bad_value(alloc::format!(
            "`{}`'s argument {index} is not {what}",
            self.method
        ))
    }

    /// Argument `index`, as it arrived.
    pub fn arg(&self, index: usize) -> Result<&Value, Refusal> {
        self.args.get(index).ok_or_else(|| self.missing(index))
    }

    /// A text argument.
    pub fn text(&self, index: usize) -> Result<&str, Refusal> {
        TryAsRef::<str>::try_as_ref(self.arg(index)?).ok_or_else(|| self.not(index, "text"))
    }

    /// A bytes argument.
    pub fn bytes(&self, index: usize) -> Result<&[u8], Refusal> {
        TryAsRef::<[u8]>::try_as_ref(self.arg(index)?).ok_or_else(|| self.not(index, "bytes"))
    }

    /// A signed whole-number argument.
    pub fn int(&self, index: usize) -> Result<i64, Refusal> {
        i64::from_value(self.arg(index)?).map_err(|_| self.not(index, "a signed whole number"))
    }

    /// An unsigned whole-number argument.
    pub fn uint(&self, index: usize) -> Result<u64, Refusal> {
        u64::from_value(self.arg(index)?).map_err(|_| self.not(index, "an unsigned whole number"))
    }

    /// A floating-point argument.
    pub fn float(&self, index: usize) -> Result<f64, Refusal> {
        f64::from_value(self.arg(index)?).map_err(|_| self.not(index, "a number"))
    }

    /// A boolean argument.
    pub fn flag(&self, index: usize) -> Result<bool, Refusal> {
        bool::from_value(self.arg(index)?).map_err(|_| self.not(index, "a boolean"))
    }

    /// A map argument, borrowed or lent.
    pub fn map(&self, index: usize) -> Result<&Map, Refusal> {
        TryAsRef::<Map>::try_as_ref(self.arg(index)?).ok_or_else(|| self.not(index, "a map"))
    }

    /// An argument handed over (`taken`): the call's from then on.
    pub fn take(&mut self, index: usize) -> Result<Value, Refusal> {
        let slot = self.args.get_mut(index).ok_or_else(|| {
            Refusal::bad_value(alloc::format!("`{}` has no argument {index}", self.method))
        })?;
        Ok(core::mem::replace(slot, Value::null()))
    }

    /// A map handed over.
    pub fn take_map(&mut self, index: usize) -> Result<Map, Refusal> {
        let value = self.take(index)?;
        Map::try_from(value).map_err(|_| self.not(index, "a map"))
    }

    /// A lent tree: what it holds when the call returns is written back
    /// over the caller's.
    pub fn lent(&mut self, index: usize) -> Result<&mut Value, Refusal> {
        if index >= self.args.len() {
            return Err(self.missing(index));
        }
        self.changed[index] = true;
        Ok(&mut self.args[index])
    }

    /// A lent map.
    pub fn lent_map(&mut self, index: usize) -> Result<&mut Map, Refusal> {
        let method = self.method.clone();
        TryAsMut::<Map>::try_as_mut(self.lent(index)?).ok_or_else(|| {
            Refusal::bad_value(alloc::format!("`{method}`'s argument {index} is not a map"))
        })
    }

    /// The caller's buffer (`bytes_mut`), as long as the caller's: what it
    /// holds when the call returns is written back.
    pub fn buffer(&mut self, index: usize) -> Result<&mut [u8], Refusal> {
        let method = self.method.clone();
        let buffer: &mut Buffer = TryAsMut::try_as_mut(self.lent(index)?).ok_or_else(|| {
            Refusal::bad_value(alloc::format!(
                "`{method}`'s argument {index} is not a buffer"
            ))
        })?;
        Ok(&mut buffer[..])
    }

    /// An object handed over: the call's, until it is dropped.
    pub fn object(&self, index: usize) -> Result<HostObject, Refusal> {
        Ok(HostObject {
            handle: self.int(index)?,
            owned: true,
        })
    }

    /// An object lent for this call: the host refuses it once the call
    /// returns.
    pub fn lent_object(&self, index: usize) -> Result<HostObject, Refusal> {
        Ok(HostObject {
            handle: self.int(index)?,
            owned: false,
        })
    }

    /// Writes back every argument the call changed, then answers.
    pub(crate) fn settle(self) -> Result<(), Refusal> {
        for (index, (value, changed)) in self.args.iter().zip(self.changed).enumerate() {
            if !changed {
                continue;
            }
            let bytes = wire::to_bytes(value).map_err(|e| Refusal::bad_value(e.to_string()))?;
            let status = host::write_back(index as u32, &bytes);
            if status != 0 {
                return Err(Refusal::new(
                    Status::from(status),
                    alloc::format!(
                        "argument {index} of `{}` could not be written back: is it lent?",
                        self.method
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// An object of the host's: a listener handed over or lent, or an object a
/// host's slot answered. Its calls go to the host.
#[derive(Debug)]
pub struct HostObject {
    handle: i64,
    owned: bool,
}

impl HostObject {
    /// Calls `method` of the object's kind with `args`, in the slot's
    /// order. Answers its value, or `None` for a slot that answers none.
    pub fn call(
        &self,
        method: &str,
        args: impl IntoIterator<Item = Value>,
    ) -> Result<Option<Value>, Refusal> {
        let mut request = List::new();
        request.push(method);
        for arg in args {
            request.push(arg);
        }
        let bytes =
            wire::to_bytes(&Value::from(request)).map_err(|e| Refusal::bad_value(e.to_string()))?;
        let (status, result) = host::object_call(self.handle, &bytes);
        if status != 0 {
            return Err(Refusal::new(
                Status::from(status),
                String::from_utf8_lossy(&result).into_owned(),
            ));
        }
        if result.is_empty() {
            return Ok(None);
        }
        wire::decode(&result)
            .map(Some)
            .map_err(|e| Refusal::bad_value(alloc::format!("the host's answer: {e}")))
    }

    /// The handle, given up: to hand the object on to a host slot that
    /// takes one, as `Value::from(handle)`.
    pub fn into_handle(self) -> i64 {
        let this = core::mem::ManuallyDrop::new(self);
        this.handle
    }
}

impl Drop for HostObject {
    fn drop(&mut self) {
        if self.owned {
            host::drop(self.handle);
        }
    }
}
