// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `toml::Value` and `toml::Table` as field types (feature `toml`).
//!
//! A table is a map in its own order, an array a list, an integer its
//! digits and a float its shortest text. A date or time is the text it
//! was written as, which is how `guatiao-serde` reads one from a
//! document; read back, it is a TOML string.

#![forbid(unsafe_code)]

use alloc::string::ToString;
use toml::value::Array;
use toml::{Table, Value as Toml};

use super::{FromValue, MapError, ToValue, number_text};
use crate::value::alloc::Alloc;
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::{List, Map, Number, Tag, Text, Value};

impl ToValue for Toml {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        write(self, alloc, 0)
    }
}

impl ToValue for Table {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        write_table(self, alloc, 0).map(Value::from)
    }
}

/// A null or bytes is refused: TOML has neither. An integer must fit 64
/// signed bits, TOML's own limit; a number with a fraction or an
/// exponent becomes the nearest `f64`.
impl FromValue for Toml {
    fn from_value(value: &Value) -> Result<Toml, MapError> {
        read(value, 0)
    }
}

impl FromValue for Table {
    fn from_value(value: &Value) -> Result<Table, MapError> {
        read_table(value, 0)
    }
}

fn write(value: &Toml, alloc: Alloc, depth: u32) -> Result<Value, ValueError> {
    Ok(match value {
        Toml::String(text) => Text::new_in(alloc, text)?.into(),
        Toml::Integer(n) => Number::new_in(alloc, &n.to_string())?.into(),
        Toml::Float(x) => Number::float_in(alloc, *x)?.into(),
        Toml::Boolean(b) => Value::from(*b),
        Toml::Datetime(when) => Text::new_in(alloc, &when.to_string())?.into(),
        Toml::Array(items) => write_array(items, alloc, depth)?.into(),
        Toml::Table(table) => write_table(table, alloc, depth)?.into(),
    })
}

fn write_array(items: &Array, alloc: Alloc, depth: u32) -> Result<List, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    let mut list = List::new_in(alloc);
    for item in items {
        list.push_in(write(item, alloc, depth + 1)?, alloc)?;
    }
    Ok(list)
}

fn write_table(table: &Table, alloc: Alloc, depth: u32) -> Result<Map, ValueError> {
    if depth >= MAX_DEPTH {
        return Err(ValueError::TooDeep);
    }
    let mut map = Map::new_in(alloc);
    for (key, item) in table {
        map.set_in(key, write(item, alloc, depth + 1)?, alloc)?;
    }
    Ok(map)
}

fn read(value: &Value, depth: u32) -> Result<Toml, MapError> {
    match value.tag() {
        Ok(Tag::GUATIAO_BOOL) => bool::from_value(value).map(Toml::Boolean),
        Ok(Tag::GUATIAO_NUMBER) => {
            let text = number_text(value)?;
            if text.contains(['.', 'e', 'E']) {
                f64::from_value(value).map(Toml::Float)
            } else {
                text.parse::<i64>()
                    .map(Toml::Integer)
                    .map_err(|_| MapError::bad_value("an integer TOML can hold: 64 bits, signed"))
            }
        }
        Ok(Tag::GUATIAO_STRING) => {
            <&str>::try_from(value).map(|text| Toml::String(text.to_string()))
        }
        Ok(Tag::GUATIAO_LIST) => read_array(value, depth).map(Toml::Array),
        Ok(Tag::GUATIAO_MAP) => read_table(value, depth).map(Toml::Table),
        _ => Err(MapError::bad_value(
            "a value TOML can hold: a boolean, a number, a string, a list or a map",
        )),
    }
}

fn read_array(value: &Value, depth: u32) -> Result<Array, MapError> {
    let items = &<&List>::try_from(value)?[..];
    if depth >= MAX_DEPTH {
        return Err(too_deep());
    }
    let mut out = Array::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        out.push(read(item, depth + 1).map_err(|e| e.at(i))?);
    }
    Ok(out)
}

fn read_table(value: &Value, depth: u32) -> Result<Table, MapError> {
    let map = <&Map>::try_from(value)?;
    if depth >= MAX_DEPTH {
        return Err(too_deep());
    }
    let mut out = Table::new();
    for entry in map {
        let item = read(entry.value(), depth + 1).map_err(|e| e.under(entry.key()))?;
        out.insert(entry.key().to_string(), item);
    }
    Ok(out)
}

fn too_deep() -> MapError {
    MapError::bad_value("a tree at most 128 containers deep")
}
