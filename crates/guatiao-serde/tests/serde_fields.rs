// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Serde types as fields of a derived type, through `Serde<T, S>` and
//! the `adapt::any` and `adapt::object` modules: each writes, reads back
//! unchanged, and the derived schema accepts what it wrote.

extern crate alloc;

use alloc::collections::BTreeMap;
use core::fmt::Debug;

use guatiao::schema::KindBuilder;
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::{Alloc, FromValue, Map, MapError, Number, Schema, ToValue, TryAsRef, Value};
use guatiao_serde::Serde;
use guatiao_serde::adapt;
use serde_json::json;

/// Writes `v`, checks its own schema accepts the value and that it reads
/// back equal. Returns the value written.
fn check<T: ToValue + FromValue + Schema + PartialEq + Debug>(v: &T) -> Value {
    let value = v.to_value().expect("writes");
    let schema = T::schema().expect("a schema");
    validate_map(SchemaRef::new(&schema).expect("a map"), &value)
        .expect("its own schema accepts it");
    assert_eq!(&T::from_value(&value).expect("reads back"), v);
    value
}

fn at<'a>(value: &'a Value, key: &str) -> &'a Value {
    <&Map>::try_from(value).unwrap().get(key).unwrap()
}

fn number(value: &Value) -> Option<&str> {
    TryAsRef::<Number>::try_as_ref(value).map(AsRef::<str>::as_ref)
}

fn kind_of<T: Schema>(key: &str) -> String {
    let schema = T::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    format!("{:?}", s.find(key).unwrap().kind())
}

/// A user's own serde type, with an enum of every variant shape.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone)]
struct Limits {
    max: u32,
    ratio: f64,
    mode: Mode,
    steps: Vec<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Clone)]
enum Mode {
    Fast,
    Fixed(u8),
    Pair(i64, bool),
    Window { from: u16, to: u16 },
}

fn limits() -> Limits {
    Limits {
        max: 10,
        ratio: 1.0,
        mode: Mode::Fast,
        steps: vec![
            Mode::Fixed(3),
            Mode::Pair(-1, true),
            Mode::Window { from: 1, to: 2 },
        ],
        note: None,
    }
}

const DOCUMENT: &str = r#"
zeta = 1
alpha = 1.5
big = 9223372036854775807
mode = "fast"
when = 1979-05-27T07:32:00Z

[nested]
deep = ["a", { b = false }]
"#;

fn table() -> toml::Table {
    DOCUMENT.parse().expect("a TOML document")
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Wrapped {
    name: String,
    settings: Serde<toml::Table, Map>,
    extra: Option<Serde<toml::Value>>,
    layers: Vec<Serde<toml::Table, Map>>,
    body: Serde<serde_json::Value>,
    headers: Serde<serde_json::Map<String, serde_json::Value>, Map>,
    limits: Serde<Limits>,
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Bare {
    #[map(with = adapt::object)]
    settings: toml::Table,
    #[map(with = adapt::any)]
    extra: toml::Value,
    #[map(with = adapt::object)]
    layers: BTreeMap<String, toml::Table>,
    #[map(with = adapt::any)]
    body: serde_json::Value,
    #[map(with = adapt::object)]
    headers: serde_json::Map<String, serde_json::Value>,
    #[map(with = adapt::any)]
    limits: Limits,
    #[map(with = limits_described)]
    described: Limits,
}

/// A `with` module of one's own: the generic conversions and a schema.
mod limits_described {
    use super::*;
    pub use guatiao_serde::adapt::{from_value, to_value_in};

    pub fn kind(alloc: Alloc) -> KindBuilder {
        KindBuilder::map_of_in(alloc, KindBuilder::any_in(alloc)).description("limits")
    }
}

fn headers() -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(headers) = json!({"accept": "text/plain", "x-n": 3}) else {
        unreachable!()
    };
    headers
}

fn body() -> serde_json::Value {
    json!({"a": [1, 2.5, null, true, "x"], "b": {"c": -7}, "u": 18446744073709551615u64})
}

#[test]
fn the_wrapper_round_trips_and_validates() {
    let wrapped = Wrapped {
        name: "scaler".to_string(),
        settings: table().into(),
        extra: Some(toml::Value::Array(vec![1.into(), "two".into(), true.into()]).into()),
        layers: vec![table().into(), toml::Table::new().into()],
        body: body().into(),
        headers: headers().into(),
        limits: limits().into(),
    };
    let mut wrapped = wrapped;
    // A TOML date reads back as the text it was written as.
    wrapped.settings.remove("when");
    wrapped.layers[0].remove("when");
    let value = check(&wrapped);
    assert_eq!(
        number(at(at(&value, "body"), "u")),
        Some("18446744073709551615")
    );
    assert!(kind_of::<Wrapped>("settings").starts_with("MapOf"));
    assert!(kind_of::<Wrapped>("body").starts_with("Unknown"));
}

#[test]
fn a_with_module_round_trips_and_validates() {
    let mut settings = table();
    settings.remove("when");
    let bare = Bare {
        settings: settings.clone(),
        extra: toml::Value::Float(0.5),
        layers: [("one".to_string(), settings)].into(),
        body: body(),
        headers: headers(),
        limits: limits(),
        described: limits(),
    };
    check(&bare);
    assert!(kind_of::<Bare>("settings").starts_with("MapOf"));
    assert!(kind_of::<Bare>("limits").starts_with("Unknown"));
    let schema = Bare::schema().unwrap();
    let described = SchemaRef::new(&schema).unwrap().find("described").unwrap();
    assert_eq!(described.description(), "limits");
}

#[test]
fn a_document_read_through_guatiao_gives_the_same_table() {
    let read = guatiao_serde::text::json::from_str(
        &serde_json::to_string(&body()).unwrap(),
        Alloc::rust(),
        guatiao_serde::Presentation::new(),
    )
    .unwrap();
    assert_eq!(read, adapt::to_value_in(&body(), Alloc::rust()).unwrap());
    assert_eq!(
        adapt::from_value::<serde_json::Value>(&read).unwrap(),
        body()
    );
}

#[test]
fn a_table_keeps_its_numbers_and_a_date_its_text() {
    let value = adapt::to_value_in(&table(), Alloc::rust()).unwrap();
    assert_eq!(number(at(&value, "big")), Some("9223372036854775807"));
    assert_eq!(number(at(&value, "alpha")), Some("1.5"));
    assert_eq!(
        <&str>::try_from(at(&value, "when")),
        Ok("1979-05-27T07:32:00Z")
    );
    let back: toml::Table = adapt::from_value(&value).unwrap();
    assert_eq!(
        back["when"],
        toml::Value::String("1979-05-27T07:32:00Z".into())
    );
    assert_eq!(back["alpha"], toml::Value::Float(1.5));
    // A float stays a float, whole or not.
    let one = adapt::to_value_in(&1.0f64, Alloc::rust()).unwrap();
    assert_eq!(number(&one), Some("1.0"));
    assert_eq!(
        adapt::from_value::<toml::Value>(&one).unwrap(),
        toml::Value::Float(1.0)
    );
}

#[test]
fn every_variant_shape_crosses_externally_tagged() {
    let value = adapt::to_value_in(&limits(), Alloc::rust()).unwrap();
    assert_eq!(<&str>::try_from(at(&value, "mode")), Ok("Fast"));
    let steps = <&guatiao::List>::try_from(at(&value, "steps")).unwrap();
    assert_eq!(number(at(&steps[0], "Fixed")), Some("3"));
    assert_eq!(
        <&guatiao::List>::try_from(at(&steps[1], "Pair"))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(number(at(at(&steps[2], "Window"), "to")), Some("2"));
    assert!(<&Map>::try_from(&value).unwrap().get("note").is_none());
    assert_eq!(adapt::from_value::<Limits>(&value).unwrap(), limits());
}

#[test]
fn what_a_type_cannot_hold_is_refused_under_its_key() {
    let mut value = adapt::to_value_in(&limits(), Alloc::rust()).unwrap();
    // A declared field is `a.b`, a position `[i]`.
    let mut map = <&Map>::try_from(&value).unwrap().clone();
    let mut steps = <&guatiao::List>::try_from(at(&value, "steps"))
        .unwrap()
        .clone();
    steps[0] = {
        let mut m = Map::new();
        m.set("Fixed", 300);
        m.into()
    };
    map.set("steps", steps);
    value = map.into();
    let error = adapt::from_value::<Limits>(&value).unwrap_err();
    // The position and the variant, joined as `MapError::at` joins them.
    assert!(
        matches!(&error, MapError::BadValue { key, .. }
            if key.starts_with("steps[0]") && key.ends_with("Fixed")),
        "{error:?}"
    );

    // A missing field names itself; a secret is never quoted.
    let mut map = Map::new();
    map.set("max", "hunter2");
    let error = adapt::from_value::<Limits>(&map.into()).unwrap_err();
    assert!(
        matches!(&error, MapError::BadValue { key, .. } if key == "max"),
        "{error:?}"
    );
    assert!(!error.to_string().contains("hunter2"), "{error}");
    let mut map = Map::new();
    map.set("max", 1);
    let error = adapt::from_value::<Limits>(&map.into()).unwrap_err();
    assert!(
        matches!(&error, MapError::MissingKey { key } if key == "ratio"),
        "{error:?}"
    );

    // An open map's key is data: `[k]`. TOML has no null.
    let mut map = Map::new();
    map.set("missing", Value::null());
    let error = adapt::from_value::<toml::Table>(&map.into()).unwrap_err();
    assert!(
        matches!(&error, MapError::BadValue { key, .. } if key == "[missing]"),
        "{error:?}"
    );

    // A field of a derived type adds its own key.
    let mut bad = <&Map>::try_from(&check_bare_value()).unwrap().clone();
    let mut layers = Map::new();
    layers.set("one", 5);
    bad.set("layers", layers);
    let error = Bare::from_value(&bad.into()).unwrap_err();
    assert!(
        matches!(&error, MapError::BadValue { key, .. } if key == "layers[one]"),
        "{error:?}"
    );

    // Wider than 128 bits, or more than an f64, has no serde spelling.
    let wide = Number::new("123456789012345678901234567890123456789012").unwrap();
    assert!(adapt::from_value::<serde_json::Value>(&wide.clone().into()).is_err());
    assert!(adapt::from_value::<f64>(&wide.into()).is_ok());
    let huge = Number::new("1e400").unwrap();
    assert!(adapt::from_value::<f64>(&huge.into()).is_err());
    let bytes = guatiao::Bytes(vec![1, 2]).to_value().unwrap();
    assert!(adapt::from_value::<serde_json::Value>(&bytes).is_err());
    assert!(adapt::from_value::<toml::Value>(&Value::null()).is_err());
}

fn check_bare_value() -> Value {
    Bare {
        settings: toml::Table::new(),
        extra: toml::Value::Boolean(true),
        layers: Default::default(),
        body: json!(null),
        headers: Default::default(),
        limits: limits(),
        described: limits(),
    }
    .to_value()
    .unwrap()
}

#[test]
fn what_a_value_cannot_hold_is_refused_when_written() {
    assert!(adapt::to_value_in(&f64::NAN, Alloc::rust()).is_err());
    let mut keyed = BTreeMap::new();
    keyed.insert(vec![1u8], 1);
    assert!(adapt::to_value_in(&keyed, Alloc::rust()).is_err());
    let numbered: BTreeMap<u32, bool> = [(7, true)].into();
    let value = adapt::to_value_in(&numbered, Alloc::rust()).unwrap();
    assert_eq!(at(&value, "7"), &Value::from(true));
}

#[test]
fn a_tree_deeper_than_a_value_can_be_is_refused_both_ways() {
    let mut deep = json!(1);
    for _ in 0..200 {
        deep = json!([deep]);
    }
    // serde_json's own recursion limit protects only its parser.
    assert_eq!(
        adapt::to_value_in(&deep, Alloc::rust()),
        Err(guatiao::ValueError::TooDeep)
    );
}

/// `exact-numbers` turns `serde_json/arbitrary_precision` on, under which
/// a JSON number serialises as its own text.
#[cfg(feature = "exact-numbers")]
#[test]
fn a_json_number_keeps_its_spelling_on_the_way_in() {
    let n: serde_json::Value = serde_json::from_str("1.10").unwrap();
    let value = adapt::to_value_in(&n, Alloc::rust()).unwrap();
    assert_eq!(number(&value), Some("1.10"));
}
