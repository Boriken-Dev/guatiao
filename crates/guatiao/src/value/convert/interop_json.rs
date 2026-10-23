// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `serde_json::Value` and `serde_json::Map<String, Value>` as field
//! types (feature `serde_json`).
//!
//! The kinds match one for one, bytes aside. A number crosses as the
//! text `serde_json` gives it, which is the text it was read from when
//! its `arbitrary_precision` feature is on and the shortest spelling of
//! an `i64`, `u64` or `f64` otherwise.

#![forbid(unsafe_code)]

use alloc::string::ToString;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as Json};

use super::{FromValue, MapError, ToValue, number_text};
use crate::value::alloc::Alloc;
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::{List, Map, Number, Tag, Text, Value};

impl ToValue for Json {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        write(self, alloc, 0)
    }
}

impl ToValue for JsonMap<String, Json> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        write_object(self, alloc, 0).map(Value::from)
    }
}

/// Bytes are refused: JSON has none. A number keeps its text when
/// `serde_json`'s `arbitrary_precision` is on; otherwise it is an `i64`,
/// a `u64` or the nearest `f64`, as `serde_json` reads one.
impl FromValue for Json {
    fn from_value(value: &Value) -> Result<Json, MapError> {
        read(value, 0)
    }
}

impl FromValue for JsonMap<String, Json> {
    fn from_value(value: &Value) -> Result<JsonMap<String, Json>, MapError> {
        read_object(value, 0)
    }
}

fn write(value: &Json, alloc: Alloc, depth: u32) -> Result<Value, ValueError> {
    Ok(match value {
        Json::Null => Value::null(),
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => Number::new_in(alloc, &n.to_string())?.into(),
        Json::String(text) => Text::new_in(alloc, text)?.into(),
        Json::Array(items) => {
            if depth >= MAX_DEPTH {
                return Err(ValueError::TooDeep);
            }
            let mut list = List::new_in(alloc);
            for item in items {
                list.push_in(write(item, alloc, depth + 1)?, alloc)?;
            }
            list.into()
        }
        Json::Object(object) => write_object(object, alloc, depth)?.into(),
    })
}

fn write_object(
    object: &JsonMap<String, Json>,
    alloc: Alloc,
    depth: u32,
) -> Result<Map, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    let mut map = Map::new_in(alloc);
    for (key, item) in object {
        map.set_in(key, write(item, alloc, depth + 1)?, alloc)?;
    }
    Ok(map)
}

fn read(value: &Value, depth: u32) -> Result<Json, MapError> {
    match value.tag() {
        Ok(Tag::GUATIAO_NULL) => Ok(Json::Null),
        Ok(Tag::GUATIAO_BOOL) => bool::from_value(value).map(Json::Bool),
        Ok(Tag::GUATIAO_NUMBER) => number_text(value)?
            .parse::<JsonNumber>()
            .map(Json::Number)
            .map_err(|_| MapError::bad_value("a number serde_json can hold")),
        Ok(Tag::GUATIAO_STRING) => {
            <&str>::try_from(value).map(|text| Json::String(text.to_string()))
        }
        Ok(Tag::GUATIAO_LIST) => {
            let items = &<&List>::try_from(value)?[..];
            if depth >= MAX_DEPTH {
                return Err(too_deep());
            }
            let mut out = alloc::vec::Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                out.push(read(item, depth + 1).map_err(|e| e.at(i))?);
            }
            Ok(Json::Array(out))
        }
        Ok(Tag::GUATIAO_MAP) => read_object(value, depth).map(Json::Object),
        _ => Err(MapError::bad_value(
            "a value JSON can hold: null, a boolean, a number, a string, a list or a map",
        )),
    }
}

fn read_object(value: &Value, depth: u32) -> Result<JsonMap<String, Json>, MapError> {
    let map = <&Map>::try_from(value)?;
    if depth >= MAX_DEPTH {
        return Err(too_deep());
    }
    let mut out = JsonMap::new();
    for entry in map {
        let item = read(entry.value(), depth + 1).map_err(|e| e.under(entry.key()))?;
        out.insert(entry.key().to_string(), item);
    }
    Ok(out)
}

fn too_deep() -> MapError {
    MapError::bad_value("a tree at most 128 containers deep")
}
