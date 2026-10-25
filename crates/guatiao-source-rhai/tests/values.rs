// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! guatiao's values as Rhai's and back: the shared vectors, and the rules
//! for what a script writes itself.

use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{Buffer, Number};
use guatiao::{List, Map, Value};
use guatiao_bridge::Convert;
use guatiao_bridge::conformance::{Outcome, round_trip};
use guatiao_source_rhai::RhaiValues;
use guatiao_source_rhai::rhai::{Dynamic, Engine};

/// The vectors that come back with their map keys in byte order, because
/// a Rhai map keeps them so: no other vector differs.
const KEYS_IN_BYTE_ORDER: [&str; 3] = ["the documented map", "a map keeps its order", "nested"];

/// `v` with every map's keys in byte order, as Rhai hands them back.
fn sorted(v: &Value) -> Value {
    if let Some(map) = TryAsRef::<Map>::try_as_ref(v) {
        let mut entries: Vec<(&str, Value)> =
            map.iter().map(|e| (e.key(), sorted(e.value()))).collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = Map::new();
        for (key, value) in entries {
            out.set(key, value);
        }
        return Value::from(out);
    }
    if let Some(list) = TryAsRef::<List>::try_as_ref(v) {
        let mut out = List::new();
        for item in list.iter() {
            out.push(sorted(item));
        }
        return Value::from(out);
    }
    v.clone()
}

#[test]
fn every_shared_vector_comes_back_the_same_but_for_key_order() {
    let originals = guatiao_bridge::conformance::values();
    let outcomes = round_trip(&RhaiValues);
    assert!(outcomes.len() >= 20, "every vector ran");
    let mut reordered = Vec::new();
    for ((name, outcome), (_, original)) in outcomes.into_iter().zip(originals) {
        match outcome {
            Outcome::Same => {}
            Outcome::Changed(back) => {
                assert_eq!(
                    back,
                    sorted(&original),
                    "{name}: only the key order differs"
                );
                reordered.push(name);
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(
        reordered, KEYS_IN_BYTE_ORDER,
        "these are the vectors whose maps hold keys out of byte order"
    );
}

fn eval(code: &str) -> Dynamic {
    Engine::new().eval(code).expect("evaluated")
}

fn to_value(code: &str) -> Result<Value, String> {
    RhaiValues.to_value(eval(code))
}

#[test]
fn numbers_are_rhai_numbers_where_one_holds_them_exactly() {
    for (text, ty) in [
        ("0", "i64"),
        ("-7", "i64"),
        ("9007199254740993", "i64"),
        ("9223372036854775807", "i64"),
        ("1.5", "f64"),
        ("0.1", "f64"),
        ("1.10", "guatiao_source_rhai::values::Exact"),
        ("1e3", "guatiao_source_rhai::values::Exact"),
        ("9223372036854775808", "guatiao_source_rhai::values::Exact"),
        ("18446744073709551615", "guatiao_source_rhai::values::Exact"),
    ] {
        let native = RhaiValues
            .to_native(&Value::from(Number::new(text).expect("a number")))
            .expect("converted");
        assert_eq!(native.type_name(), ty, "{text}");
        let back = RhaiValues.to_value(native).expect("back");
        let back: &Number = TryAsRef::try_as_ref(&back).expect("a number");
        assert_eq!(&**back, text, "comes back as it went");
    }
}

#[test]
fn null_and_absent_are_told_apart() {
    let null = RhaiValues.to_native(&Value::null()).expect("null");
    assert!(null.is_unit(), "null is ()");
    let absent = RhaiValues.to_native(&Value::absent()).expect("absent");
    assert!(!absent.is_unit(), "absent is not ()");
    assert_eq!(RhaiValues.to_value(absent).expect("back"), Value::absent());
    assert_eq!(RhaiValues.to_value(null).expect("back"), Value::null());
}

#[test]
fn bytes_are_a_blob_and_text_is_a_string() {
    let bytes = RhaiValues
        .to_native(&Value::from(Buffer::from(&[0u8, 255, 16][..])))
        .expect("bytes");
    assert!(bytes.is_blob());
    let text = RhaiValues.to_native(&Value::from("hi")).expect("text");
    assert!(text.is_string());
    let from_blob = to_value("blob(2, 7)").expect("a blob");
    assert_eq!(
        TryAsRef::<[u8]>::try_as_ref(&from_blob),
        Some(&[7u8, 7][..])
    );
    let from_char = to_value("'x'").expect("a char");
    assert_eq!(TryAsRef::<str>::try_as_ref(&from_char), Some("x"));
}

#[test]
fn arrays_and_maps_are_lists_and_maps() {
    let list = to_value("[1, \"two\", true, ()]").expect("a list");
    let list: &List = TryAsRef::try_as_ref(&list).expect("a list");
    assert_eq!(list.len(), 4);
    let map = to_value("#{ b: 1, a: 2 }").expect("a map");
    let keys: Vec<&str> = TryAsRef::<Map>::try_as_ref(&map)
        .expect("a map")
        .keys()
        .collect();
    assert_eq!(keys, ["a", "b"], "a Rhai map's keys are in byte order");
    let empty_map = to_value("#{}").expect("empty");
    assert!(TryAsRef::<Map>::try_as_ref(&empty_map).is_some());
    let empty_list = to_value("[]").expect("empty");
    assert!(TryAsRef::<List>::try_as_ref(&empty_list).is_some());
}

#[test]
fn what_has_no_value_is_refused_in_words() {
    let function = to_value("#{ f: |x| x }").unwrap_err();
    assert!(function.contains("has no value to cross as"), "{function}");
    let nan = to_value("0.0 / 0.0").unwrap_err();
    assert!(nan.contains("not finite"), "{nan}");
    let infinite = to_value("1.0 / 0.0").unwrap_err();
    assert!(infinite.contains("not finite"), "{infinite}");
    let deep = {
        let mut v = Value::from(1i64);
        for _ in 0..600 {
            let mut list = List::new();
            list.push(v);
            v = Value::from(list);
        }
        v
    };
    let too_deep = RhaiValues.to_native(&deep).unwrap_err();
    assert!(too_deep.contains("deeper than 512"), "{too_deep}");
}
