// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JavaScript values and guatiao's, both ways. An integer is a `number`
//! while one holds it exactly and a `BigInt` beyond; a decimal is a
//! `number` when its shortest text is the text and an `ExactNumber`
//! otherwise; bytes are a `Uint8Array`; a map is a plain object.

use alloc::string::String;
use core::fmt;

use guatiao::library::kind::Scalar;
use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{Buffer, Number};
use guatiao::{List, Map, Tag, Value};
use guatiao_bridge::Convert;
use rquickjs::object::{Filter, Property};
use rquickjs::{
    Array, ArrayBuffer, BigInt, Coerced, Ctx, Function, Object, Type, TypedArray, Value as Js,
};

use crate::bytes;
use crate::prelude::{self, Core};

/// Trees nest at most this deep on their way across.
const DEPTH: usize = 128;

/// The largest integer a double holds exactly.
const SAFE: u64 = 1 << 53;

/// Why a value could not cross.
#[derive(Debug)]
pub enum Conversion {
    /// QuickJS failed (an exception a getter threw, memory).
    Js(rquickjs::Error),
    /// The value has no counterpart, in words.
    Refused(String),
}

impl fmt::Display for Conversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Conversion::Js(e) => write!(f, "{e}"),
            Conversion::Refused(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for Conversion {}

impl From<rquickjs::Error> for Conversion {
    fn from(e: rquickjs::Error) -> Conversion {
        Conversion::Js(e)
    }
}

pub(crate) fn refuse<T>(why: impl Into<String>) -> Result<T, Conversion> {
    Err(Conversion::Refused(why.into()))
}

// --- guatiao's value as JavaScript's ---------------------------------------------

/// A whole number as a `number` while a double holds it, else a `BigInt`.
pub(crate) fn integer_to_js<'js>(ctx: &Ctx<'js>, text: &str) -> Result<Js<'js>, Conversion> {
    if let Ok(i) = text.parse::<i64>()
        && i.to_string() == text
    {
        return Ok(if i.unsigned_abs() <= SAFE {
            number(ctx, i as f64)
        } else {
            BigInt::from_i64(ctx.clone(), i)?.into_value()
        });
    }
    if let Ok(u) = text.parse::<u64>()
        && u.to_string() == text
    {
        return Ok(BigInt::from_u64(ctx.clone(), u)?.into_value());
    }
    let big: Function<'js> = ctx.globals().get("BigInt")?;
    Ok(big.call((text,))?)
}

/// A JavaScript number holding `f`; an int where it fits one.
fn number<'js>(ctx: &Ctx<'js>, f: f64) -> Js<'js> {
    if f == 0.0 && f.is_sign_negative() {
        // `new_number` would make it the integer 0.
        return Js::new_float(ctx.clone(), f);
    }
    Js::new_number(ctx.clone(), f)
}

/// A slot's integer as a script receives or reads it: a `BigInt` for a
/// 64-bit or wider type, as the Node binding gives it, else a `number`.
pub(crate) fn scalar_to_js<'js>(
    ctx: &Ctx<'js>,
    scalar: Option<Scalar>,
    text: &str,
) -> Result<Js<'js>, Conversion> {
    let wide = matches!(
        scalar,
        Some(
            Scalar::I64 | Scalar::U64 | Scalar::I128 | Scalar::U128 | Scalar::Isize | Scalar::Usize
        )
    );
    if !wide {
        return integer_to_js(ctx, text);
    }
    if let Ok(i) = text.parse::<i64>() {
        return Ok(BigInt::from_i64(ctx.clone(), i)?.into_value());
    }
    if let Ok(u) = text.parse::<u64>() {
        return Ok(BigInt::from_u64(ctx.clone(), u)?.into_value());
    }
    let big: Function<'js> = ctx.globals().get("BigInt")?;
    Ok(big.call((text,))?)
}

/// A number's text as JavaScript's: an integer as above, a decimal as a
/// `number` when its shortest text is the text, else an `ExactNumber`.
pub(crate) fn number_to_js<'js>(
    core: &Core<'js>,
    ctx: &Ctx<'js>,
    text: &str,
) -> Result<Js<'js>, Conversion> {
    let whole = text
        .strip_prefix('-')
        .unwrap_or(text)
        .bytes()
        .all(|b| b.is_ascii_digit());
    if whole && text != "-0" {
        return integer_to_js(ctx, text);
    }
    if let Ok(f) = text.parse::<f64>()
        && f.is_finite()
        && f.to_string() == text
    {
        return Ok(number(ctx, f));
    }
    Ok(core.exact.construct((text,))?)
}

/// `v` as JavaScript's; `absent` is `undefined`.
pub(crate) fn to_js<'js>(
    core: &Core<'js>,
    ctx: &Ctx<'js>,
    v: &Value,
) -> Result<Js<'js>, Conversion> {
    to_js_at(core, ctx, v, 0)
}

fn to_js_at<'js>(
    core: &Core<'js>,
    ctx: &Ctx<'js>,
    v: &Value,
    depth: usize,
) -> Result<Js<'js>, Conversion> {
    if depth > DEPTH {
        return refuse(format!("a tree deeper than {DEPTH} levels"));
    }
    let tag = v
        .tag()
        .or_else(|e| refuse(format!("a value guatiao cannot read: {e}")))?;
    Ok(match tag {
        Tag::GUATIAO_ABSENT => Js::new_undefined(ctx.clone()),
        Tag::GUATIAO_NULL => Js::new_null(ctx.clone()),
        Tag::GUATIAO_BOOL => Js::new_bool(
            ctx.clone(),
            TryAsRef::<bool>::try_as_ref(v).copied().unwrap_or(false),
        ),
        Tag::GUATIAO_NUMBER => {
            let n: &Number = TryAsRef::try_as_ref(v)
                .ok_or_else(|| Conversion::Refused("a malformed number".into()))?;
            number_to_js(core, ctx, n)?
        }
        Tag::GUATIAO_STRING => {
            let s: &str = TryAsRef::try_as_ref(v)
                .ok_or_else(|| Conversion::Refused("text that is not UTF-8".into()))?;
            rquickjs::String::from_str(ctx.clone(), s)?.into_value()
        }
        Tag::GUATIAO_BYTES => {
            let b: &[u8] = TryAsRef::try_as_ref(v).unwrap_or(&[]);
            TypedArray::<u8>::new_copy(ctx.clone(), b)?.into_value()
        }
        Tag::GUATIAO_LIST => {
            let list: &List = TryAsRef::try_as_ref(v)
                .ok_or_else(|| Conversion::Refused("a malformed list".into()))?;
            let array = Array::new(ctx.clone())?;
            for (i, item) in list.iter().enumerate() {
                array.set(i, to_js_at(core, ctx, item, depth + 1)?)?;
            }
            array.into_value()
        }
        Tag::GUATIAO_MAP => {
            let map: &Map = TryAsRef::try_as_ref(v)
                .ok_or_else(|| Conversion::Refused("a malformed map".into()))?;
            let object = Object::new(ctx.clone())?;
            for entry in map.iter() {
                // Defined, not assigned: a key named `__proto__` is data.
                object.prop(
                    entry.key(),
                    Property::from(to_js_at(core, ctx, entry.value(), depth + 1)?)
                        .enumerable()
                        .writable()
                        .configurable(),
                )?;
            }
            object.into_value()
        }
        _ => return refuse("a value of a kind this binding does not know"),
    })
}

// --- JavaScript's value as guatiao's ------------------------------------------------

/// The exact text of a number a script holds: a `BigInt` or an
/// `ExactNumber`, through JavaScript's own conversion to text.
fn text_of<'js>(v: &Js<'js>) -> Result<String, Conversion> {
    Ok(v.get::<Coerced<String>>()?.0)
}

fn number_value(text: &str) -> Result<Value, Conversion> {
    Ok(Value::from(Number::new(text).or_else(|_| {
        refuse(format!("`{text}` is not a number"))
    })?))
}

/// A float's value: an integer when it is one, else its shortest text.
fn float_value(f: f64) -> Result<Value, Conversion> {
    if !f.is_finite() {
        return refuse("a number that is not finite has no value to cross as");
    }
    if f.fract() == 0.0 && f.abs() < SAFE as f64 && !(f == 0.0 && f.is_sign_negative()) {
        return Ok(Value::from(f as i64));
    }
    Value::try_from(f).or_else(|_| refuse("a number that is not finite has no value to cross as"))
}

/// `v` as guatiao's; `undefined` is absent.
pub(crate) fn to_value<'js>(core: &Core<'js>, v: &Js<'js>) -> Result<Value, Conversion> {
    to_value_at(core, v, 0, &mut Vec::new())
}

fn to_value_at<'js>(
    core: &Core<'js>,
    v: &Js<'js>,
    depth: usize,
    ancestors: &mut Vec<Object<'js>>,
) -> Result<Value, Conversion> {
    if depth > DEPTH {
        return refuse(format!("a value nested deeper than {DEPTH} levels"));
    }
    match v.type_of() {
        Type::Undefined | Type::Uninitialized => Ok(Value::absent()),
        Type::Null => Ok(Value::null()),
        Type::Bool => Ok(Value::from(v.as_bool().unwrap_or(false))),
        Type::Int => Ok(Value::from(i64::from(v.as_int().unwrap_or(0)))),
        Type::Float => float_value(v.as_float().unwrap_or(f64::NAN)),
        Type::BigInt => number_value(&text_of(v)?),
        Type::String => {
            let s = v
                .as_string()
                .ok_or_else(|| Conversion::Refused("a string that is not one".into()))?
                .to_string()
                .or_else(|_| refuse("text that is not valid Unicode"))?;
            Ok(Value::from(s.as_str()))
        }
        Type::Array => {
            let array = v.as_array().expect("typed as an array");
            enter(array.as_object(), ancestors)?;
            let mut list = List::new();
            for i in 0..array.len() {
                let item: Js<'js> = array.get(i)?;
                list.push(to_value_at(core, &item, depth + 1, ancestors)?);
            }
            ancestors.pop();
            Ok(Value::from(list))
        }
        Type::Object => {
            let object = v.as_object().expect("typed as an object");
            if object.is_instance_of(&core.exact) {
                return number_value(&text_of(v)?);
            }
            if let Some(held) = bytes_in(core, object) {
                return Ok(Value::from(Buffer::from(&held[..])));
            }
            let plain = match object.get_prototype() {
                None => true,
                Some(proto) => proto == core.object_prototype,
            };
            if !plain {
                return refuse(format!(
                    "{} is not plain data (an object, array, text, number, bytes or null)",
                    class_of(object)
                ));
            }
            enter(object, ancestors)?;
            let mut map = Map::new();
            // Keys are read as strings, not C strings: a NUL is a character.
            for entry in object.own_props::<Js<'js>, Js<'js>>(Filter::new().string().enum_only()) {
                let (key, item) = entry?;
                let key = key
                    .as_string()
                    .ok_or_else(|| Conversion::Refused("a key that is not text".into()))?
                    .to_string()
                    .or_else(|_| refuse("a key that is not valid Unicode"))?;
                map.set(&key, to_value_at(core, &item, depth + 1, ancestors)?);
            }
            ancestors.pop();
            Ok(Value::from(map))
        }
        other => refuse(format!("a {} has no value to cross as", other.as_str())),
    }
}

/// The bytes a `Uint8Array` or an `ArrayBuffer` holds, copied.
pub(crate) fn bytes_in<'js>(core: &Core<'js>, object: &Object<'js>) -> Option<Vec<u8>> {
    if let Ok(array) = TypedArray::<u8>::from_object(object.clone()) {
        return Some(bytes::of_array(&array));
    }
    // Asking an object that is no buffer leaves QuickJS's TypeError
    // pending, which a later import would take for its own.
    let buffer = ArrayBuffer::from_object(object.clone());
    if buffer.is_none() {
        drop(core.ctx.catch());
    }
    buffer.map(|b| bytes::of_buffer(&b))
}

/// Notes that `object` is being converted; a cycle is refused.
fn enter<'js>(object: &Object<'js>, ancestors: &mut Vec<Object<'js>>) -> Result<(), Conversion> {
    if ancestors.iter().any(|a| a == object) {
        return refuse("an object that contains itself has no value to cross as");
    }
    ancestors.push(object.clone());
    Ok(())
}

/// What to call `object` in a refusal: its constructor's name.
fn class_of(object: &Object<'_>) -> String {
    object
        .get::<_, Object<'_>>("constructor")
        .and_then(|c| c.get::<_, String>("name"))
        .map_or_else(|_| "an object".to_string(), |name| format!("a {name}"))
}

/// Writes `v` over what the tree `home` holds, keeping the object: how a
/// lent tree a host's callee changed comes back to the script that lent it.
pub(crate) fn refill<'js>(
    core: &Core<'js>,
    ctx: &Ctx<'js>,
    home: &Js<'js>,
    v: &Value,
) -> Result<(), Conversion> {
    let fresh = to_js(core, ctx, v)?;
    if let (Some(home), Some(fresh)) = (home.as_array(), fresh.as_array()) {
        home.as_object().set("length", 0)?;
        for i in 0..fresh.len() {
            home.set(i, fresh.get::<Js<'js>>(i)?)?;
        }
        return Ok(());
    }
    let (Some(home), Some(fresh)) = (home.as_object(), fresh.as_object()) else {
        return refuse("a lent tree came back as something else");
    };
    let keys: Vec<Js<'js>> = home
        .own_keys::<Js<'js>>(Filter::new().string().enum_only())
        .collect::<Result<_, _>>()?;
    for key in keys {
        home.remove(key)?;
    }
    for entry in fresh.own_props::<Js<'js>, Js<'js>>(Filter::new().string().enum_only()) {
        let (key, item) = entry?;
        home.prop(
            key,
            Property::from(item).enumerable().writable().configurable(),
        )?;
    }
    Ok(())
}

/// Writes `bytes` over what the `Uint8Array` `home` holds.
pub(crate) fn overwrite<'js>(
    ctx: &Ctx<'js>,
    home: &Js<'js>,
    bytes: &[u8],
) -> Result<(), Conversion> {
    let Some(array) = home.as_object() else {
        return refuse("a buffer came back as something else");
    };
    let fresh = TypedArray::<u8>::new_copy(ctx.clone(), bytes)?;
    let set: Function<'js> = array.get("set")?;
    set.call::<_, ()>((rquickjs::function::This(array.clone()), fresh))?;
    Ok(())
}

// --- the conversion the shared vectors check -------------------------------------------

/// guatiao's values as a context's: the conversion the shared vectors
/// check.
pub struct JsValues<'js> {
    ctx: Ctx<'js>,
}

impl<'js> JsValues<'js> {
    /// Conversions in `ctx`, which is given what a library is (the
    /// `guatiao` script) if it has not got it yet.
    pub fn new(ctx: &Ctx<'js>) -> rquickjs::Result<JsValues<'js>> {
        prelude::install(ctx)?;
        Ok(JsValues { ctx: ctx.clone() })
    }
}

impl<'js> Convert for JsValues<'js> {
    type Native<'a>
        = Js<'js>
    where
        Self: 'a;
    type Error = Conversion;

    fn to_native<'a>(&'a self, value: &Value) -> Result<Js<'js>, Conversion> {
        let core = prelude::core(&self.ctx)?;
        to_js(&core, &self.ctx, value)
    }

    fn to_value<'a>(&'a self, native: Js<'js>) -> Result<Value, Conversion> {
        let core = prelude::core(&self.ctx)?;
        to_value(&core, &native)
    }
}
