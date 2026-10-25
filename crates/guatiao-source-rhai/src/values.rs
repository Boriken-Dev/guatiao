// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Rhai values and guatiao's, both ways, keeping the shapes guatiao has
//! and Rhai does not: absent apart from null, a number's exact text, and
//! bytes apart from text. A Rhai map keeps its keys in byte order, so a
//! guatiao map's own key order does not survive the crossing.

use rhai::{Array, Blob, Dynamic, ImmutableString, Map as RhaiMap};

use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{Buffer, Number};
use guatiao::{List, Map, Tag, Value};
use guatiao_bridge::Convert;

/// Trees nest at most this deep on their way across.
const DEPTH: usize = 512;

/// `absent` inside a tree, where `()` already means null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Absent;

/// A number whose exact text no Rhai number holds (`1.10`, a 40-digit
/// integer): it prints as its text, and computes and compares through its
/// float.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exact(pub String);

impl Exact {
    /// The number as a float: what arithmetic on it uses.
    pub fn float(&self) -> f64 {
        self.0.parse().unwrap_or(f64::NAN)
    }
}

/// A number's text as Rhai's: an integer when its text is one, a float
/// when the float's shortest text is the text, else exact.
pub(crate) fn number_to_dynamic(text: &str) -> Dynamic {
    if let Ok(i) = text.parse::<i64>()
        && i.to_string() == text
    {
        return Dynamic::from(i);
    }
    if let Ok(f) = text.parse::<f64>()
        && f.is_finite()
        && f.to_string() == text
    {
        return Dynamic::from(f);
    }
    Dynamic::from(Exact(text.to_string()))
}

/// A float's value: its shortest text; NaN and the infinities have none.
pub(crate) fn float_value(f: f64) -> Result<Value, String> {
    Value::try_from(f).map_err(|_| "a number that is not finite has no value to cross as".into())
}

/// `v` as Rhai's: null is `()`, absent is [`Absent`].
pub(crate) fn to_dynamic(v: &Value) -> Result<Dynamic, String> {
    to_dynamic_at(v, 0)
}

fn to_dynamic_at(v: &Value, depth: usize) -> Result<Dynamic, String> {
    if depth > DEPTH {
        return Err(format!("a tree deeper than {DEPTH} levels"));
    }
    let tag = v
        .tag()
        .map_err(|e| format!("a value guatiao cannot read: {e}"))?;
    Ok(match tag {
        Tag::GUATIAO_ABSENT => Dynamic::from(Absent),
        Tag::GUATIAO_NULL => Dynamic::UNIT,
        Tag::GUATIAO_BOOL => {
            Dynamic::from(TryAsRef::<bool>::try_as_ref(v).copied().unwrap_or(false))
        }
        Tag::GUATIAO_NUMBER => {
            let n: &Number = TryAsRef::try_as_ref(v).ok_or("a malformed number")?;
            number_to_dynamic(n)
        }
        Tag::GUATIAO_STRING => {
            let s: &str = TryAsRef::try_as_ref(v).ok_or("text that is not UTF-8")?;
            Dynamic::from(ImmutableString::from(s))
        }
        Tag::GUATIAO_BYTES => {
            let b: &[u8] = TryAsRef::try_as_ref(v).unwrap_or(&[]);
            Dynamic::from_blob(b.to_vec())
        }
        Tag::GUATIAO_LIST => {
            let list: &List = TryAsRef::try_as_ref(v).ok_or("a malformed list")?;
            list_to_dynamic(list, depth)?
        }
        Tag::GUATIAO_MAP => {
            let map: &Map = TryAsRef::try_as_ref(v).ok_or("a malformed map")?;
            map_to_dynamic(map, depth)?
        }
        _ => return Err("a value of a kind this binding does not know".into()),
    })
}

pub(crate) fn list_to_dynamic(list: &List, depth: usize) -> Result<Dynamic, String> {
    let mut items = Array::with_capacity(list.len());
    for item in list.iter() {
        items.push(to_dynamic_at(item, depth + 1)?);
    }
    Ok(Dynamic::from_array(items))
}

pub(crate) fn map_to_dynamic(map: &Map, depth: usize) -> Result<Dynamic, String> {
    let mut out = RhaiMap::new();
    for entry in map.iter() {
        out.insert(entry.key().into(), to_dynamic_at(entry.value(), depth + 1)?);
    }
    Ok(Dynamic::from_map(out))
}

/// A Rhai value as guatiao's: `()` is null, [`Absent`] is absent.
pub(crate) fn to_value(d: &Dynamic) -> Result<Value, String> {
    to_value_at(d, 0)
}

fn to_value_at(d: &Dynamic, depth: usize) -> Result<Value, String> {
    if depth > DEPTH {
        return Err(format!("a value nested deeper than {DEPTH} levels"));
    }
    // A value a closure captured or a lent argument is shared: look at
    // what it holds.
    let flat;
    let d = if d.is_shared() {
        flat = d.flatten_clone();
        &flat
    } else {
        d
    };
    if d.is_unit() {
        return Ok(Value::null());
    }
    if let Ok(b) = d.as_bool() {
        return Ok(Value::from(b));
    }
    if let Ok(i) = d.as_int() {
        return Ok(Value::from(i));
    }
    if let Ok(f) = d.as_float() {
        return float_value(f);
    }
    if let Some(s) = d.read_lock::<ImmutableString>() {
        return Ok(Value::from(s.as_str()));
    }
    if let Ok(c) = d.as_char() {
        return Ok(Value::from(c.to_string().as_str()));
    }
    if let Some(b) = d.read_lock::<Blob>() {
        return Ok(Value::from(Buffer::from(&b[..])));
    }
    if let Some(items) = d.read_lock::<Array>() {
        let mut list = List::new();
        for item in items.iter() {
            list.push(to_value_at(item, depth + 1)?);
        }
        return Ok(Value::from(list));
    }
    if let Some(entries) = d.read_lock::<RhaiMap>() {
        let mut map = Map::new();
        for (key, item) in entries.iter() {
            map.set(key.as_str(), to_value_at(item, depth + 1)?);
        }
        return Ok(Value::from(map));
    }
    if d.is::<Absent>() {
        return Ok(Value::absent());
    }
    if let Some(n) = d.read_lock::<Exact>() {
        return Number::new(&n.0)
            .map(Value::from)
            .map_err(|_| format!("`{}` is not a number", n.0));
    }
    Err(format!("a {} has no value to cross as", type_name(d)))
}

/// What a value is called in a refusal: Rhai's own names, with ours for
/// the types this crate adds.
pub(crate) fn type_name(d: &Dynamic) -> String {
    let name = d.type_name();
    match name.rsplit("::").next() {
        Some(last) if name.contains("guatiao_source_rhai") => {
            format!("guatiao {}", last.to_lowercase())
        }
        _ => name.to_string(),
    }
}

/// guatiao's values as Rhai's: the conversion the shared vectors check.
#[derive(Debug, Clone, Copy, Default)]
pub struct RhaiValues;

impl Convert for RhaiValues {
    type Native<'a> = Dynamic;
    type Error = String;

    fn to_native(&self, value: &Value) -> Result<Dynamic, String> {
        to_dynamic(value)
    }

    fn to_value(&self, native: Dynamic) -> Result<Value, String> {
        to_value(&native)
    }
}
