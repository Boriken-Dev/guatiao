// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The script as the callee: what a provider's method is handed for each
//! argument of a slot, and what its answer comes to.

use alloc::rc::Rc;

use guatiao::library::ProviderError;
use guatiao::library::kind::{
    AnswerType, ArgType, KindDescription, Pass, RawObject, Scalar, SlotDescription,
};
use guatiao::value::convert::TryAsRef;
use guatiao::{Status, Tag, Value};
use guatiao_bridge::{CallArg, Held};
use rquickjs::{Ctx, Object, TypedArray, Value as Js};

use crate::errors::{self, bad_value};
use crate::host::{self, Shared};
use crate::prelude::Core;
use crate::values;

/// What a method was handed for a lent argument, read back after the call.
pub(crate) enum Received<'js> {
    /// Nothing to read back.
    Plain,
    /// A lent tree: the object the script may have changed in place.
    Tree(Js<'js>),
    /// A buffer the script wrote into.
    Buffer(TypedArray<'js, u8>),
}

/// A lent argument's contents after the call.
pub(crate) enum ReadBack {
    Tree(Value),
    Bytes(Vec<u8>),
}

/// One argument of a call, as the method receives it.
pub(crate) fn argument<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Rc<Shared>,
    ty: &ArgType,
    arg: &mut CallArg,
) -> Result<(Js<'js>, Received<'js>), ProviderError> {
    let wrong = |e: values::Conversion| errors::conversion(ctx, Some(core), &e, shared.words);
    let object_kind = match ty {
        ArgType::Object { kind, .. } => kind.as_str(),
        _ => "",
    };
    Ok(match arg {
        CallArg::Value(v) | CallArg::Lent(v) => match ty {
            ArgType::Scalar(Scalar::Bool) => (
                Js::new_bool(
                    ctx.clone(),
                    TryAsRef::<bool>::try_as_ref(&*v).copied().unwrap_or(false),
                ),
                Received::Plain,
            ),
            ArgType::Scalar(Scalar::F32 | Scalar::F64) => {
                let text: &str = TryAsRef::<guatiao::value::types::Number>::try_as_ref(&*v)
                    .map_or("NaN", |n| &**n);
                (
                    values::number_to_js(core, ctx, text).map_err(wrong)?,
                    Received::Plain,
                )
            }
            ArgType::Scalar(s) => {
                let text: &str = TryAsRef::<guatiao::value::types::Number>::try_as_ref(&*v)
                    .map_or("0", |n| &**n);
                (
                    values::scalar_to_js(ctx, Some(*s), text).map_err(wrong)?,
                    Received::Plain,
                )
            }
            ArgType::Tree {
                pass: Pass::Lent, ..
            } => {
                let tree = values::to_js(core, ctx, v).map_err(wrong)?;
                (tree.clone(), Received::Tree(tree))
            }
            _ => (values::to_js(core, ctx, v).map_err(wrong)?, Received::Plain),
        },
        CallArg::Buffer(bytes) => {
            let array = TypedArray::<u8>::new(ctx.clone(), core::mem::take(bytes))
                .map_err(|e| errors::caught(ctx, Some(core), &e, shared.words))?;
            (array.clone().into_value(), Received::Buffer(array))
        }
        CallArg::Object(_) => {
            let CallArg::Object(o) = core::mem::replace(arg, CallArg::Value(Value::null())) else {
                unreachable!("matched just above")
            };
            let object = host::object_for(ctx, shared, object_kind, Held::Object(Some(o)))?;
            (object.into_value(), Received::Plain)
        }
        CallArg::LentObject(h) => {
            let object = host::object_for(ctx, shared, object_kind, Held::Lent(h.clone()))?;
            (object.into_value(), Received::Plain)
        }
        _ => {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "an argument this engine does not carry",
            ));
        }
    })
}

/// What a lent argument holds after the call.
pub(crate) fn read_back<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Shared,
    received: &Received<'js>,
) -> Result<Option<ReadBack>, ProviderError> {
    Ok(match received {
        Received::Plain => None,
        Received::Tree(tree) => Some(ReadBack::Tree(
            values::to_value(core, tree)
                .map_err(|e| errors::conversion(ctx, Some(core), &e, shared.words))?,
        )),
        Received::Buffer(array) => Some(ReadBack::Bytes(crate::bytes::of_array(array))),
    })
}

/// What a method's answer to a slot came to.
pub(crate) enum Outcome<'js> {
    Nothing,
    Value(Value),
    /// A script object serving an object kind: the object it answers.
    Served(String, Object<'js>),
    /// An object handed back as it came.
    Handed(RawObject),
}

/// The slots of `kind` that `target` (a prototype or an object) leaves
/// out and the kind requires.
pub(crate) fn missing<'js>(kind: &KindDescription, target: &Object<'js>) -> Vec<String> {
    kind.slots()
        .iter()
        .filter(|slot| {
            slot.required()
                && !target
                    .get::<_, Js<'js>>(host::method_name(slot.name()))
                    .is_ok_and(|f| f.is_function())
        })
        .map(|slot| host::method_name(slot.name()))
        .collect()
}

/// What a method returned to `slot`, as the slot's answer.
pub(crate) fn outcome<'js>(
    ctx: &Ctx<'js>,
    core: &Core<'js>,
    shared: &Rc<Shared>,
    slot: &SlotDescription,
    returned: Js<'js>,
) -> Result<Outcome<'js>, ProviderError> {
    if returned.is_promise() {
        return Err(ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            &format!(
                "`{}` returned a promise: a slot answers before it returns",
                host::method_name(slot.name())
            ),
        ));
    }
    let wrong = |e: values::Conversion| errors::conversion(ctx, Some(core), &e, shared.words);
    Ok(match slot.answer() {
        None => Outcome::Nothing,
        Some(AnswerType::Object { kind }) => {
            if let Some(handle) = host::handle_of(ctx, core, shared, &returned) {
                return Ok(Outcome::Handed(host::take_object(&handle, kind)?));
            }
            let Some(object) = returned.into_object() else {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "`{}` answers an object of `{kind}`",
                        host::method_name(slot.name())
                    ),
                ));
            };
            let described = shared.kind(kind)?;
            let lacking = missing(&described, &object);
            if !lacking.is_empty() {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "the object `{}` answers does not implement {}, which the {kind} kind requires",
                        host::method_name(slot.name()),
                        lacking.join(", ")
                    ),
                ));
            }
            Outcome::Served(kind.clone(), object)
        }
        Some(AnswerType::Text) => match returned.as_string() {
            Some(s) => Outcome::Value(Value::from(
                s.to_string()
                    .map_err(|_| bad_value("text that is not valid Unicode"))?
                    .as_str(),
            )),
            None => {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "`{}` answers text, not a {}",
                        host::method_name(slot.name()),
                        returned.type_name()
                    ),
                ));
            }
        },
        Some(_) => {
            let value = values::to_value(core, &returned).map_err(wrong)?;
            if value.tag() == Ok(Tag::GUATIAO_ABSENT)
                && !matches!(slot.answer(), Some(AnswerType::Tree(_)))
            {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_WRONG_KIND,
                    &format!(
                        "`{}` answers a value, and returned undefined",
                        host::method_name(slot.name())
                    ),
                ));
            }
            Outcome::Value(value)
        }
    })
}
