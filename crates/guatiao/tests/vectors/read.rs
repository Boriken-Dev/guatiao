// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `time.json`, read into plain structs for the tests that share it.

#![allow(dead_code)]

use serde_json::Value as Json;

/// One text vector: what to read, and either its canonical spelling and
/// fields or the byte offset it is refused at.
pub struct TextCase {
    pub text: String,
    pub expect: Result<(String, Vec<i64>), usize>,
}

/// Applying a span: the start, the span, the result or `None` for a
/// refusal.
pub struct AddCase {
    pub start: String,
    pub span: String,
    pub result: Option<String>,
}

/// Reading an exact length: the value as JSON, `[seconds, nanoseconds]`
/// or `None` for a refusal.
pub struct DurationCase {
    pub value: Json,
    pub expect: Option<(u64, u32)>,
}

pub fn file() -> Json {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/time.json");
    let text = std::fs::read_to_string(path).expect("the vector file is readable");
    serde_json::from_str(&text).expect("the vector file is JSON")
}

fn list<'a>(doc: &'a Json, form: &str) -> &'a [Json] {
    doc[form]
        .as_array()
        .unwrap_or_else(|| panic!("{form} is a list"))
}

fn text(v: &Json, key: &str) -> String {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} is text in {v}"))
        .to_string()
}

/// The vectors of one text form: `date`, `time`, `datetime`, `timestamp`
/// or `span`.
pub fn texts(doc: &Json, form: &str) -> Vec<TextCase> {
    list(doc, form)
        .iter()
        .map(|v| TextCase {
            text: text(v, "text"),
            expect: match v.get("refused_at") {
                Some(at) => Err(at.as_u64().expect("an offset") as usize),
                None => Ok((
                    text(v, "canonical"),
                    v["fields"]
                        .as_array()
                        .expect("fields")
                        .iter()
                        .map(|n| n.as_i64().expect("a field fits an i64"))
                        .collect(),
                )),
            },
        })
        .collect()
}

/// `date_add_span` or `datetime_add_span`.
pub fn additions(doc: &Json, form: &str) -> Vec<AddCase> {
    list(doc, form)
        .iter()
        .map(|v| AddCase {
            start: text(v, "start"),
            span: text(v, "span"),
            result: v["result"].as_str().map(str::to_string),
        })
        .collect()
}

pub fn durations(doc: &Json) -> Vec<DurationCase> {
    list(doc, "duration")
        .iter()
        .map(|v| DurationCase {
            value: v["value"].clone(),
            expect: v["duration"].as_array().map(|d| {
                (
                    d[0].as_u64().expect("seconds"),
                    d[1].as_u64().expect("nanoseconds") as u32,
                )
            }),
        })
        .collect()
}

/// A JSON value as a guatiao one, a number kept as the text JSON wrote.
pub fn to_value(v: &Json) -> guatiao::Value {
    use guatiao::{List, Map, Number, Value};
    match v {
        Json::Null => Value::null(),
        Json::Bool(b) => Value::from(*b),
        Json::Number(n) => Value::from(Number::new(&n.to_string()).expect("a JSON number")),
        Json::String(s) => Value::from(s.as_str()),
        Json::Array(items) => {
            let mut list = List::new();
            for item in items {
                list.push(to_value(item));
            }
            list.into()
        }
        Json::Object(entries) => {
            let mut map = Map::new();
            for (k, item) in entries {
                map.set(k, to_value(item));
            }
            map.into()
        }
    }
}
