// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a value differs from its schema's defaults or from a baseline:
//! the shared vectors, the leaf rule, the `$ref` rule and a derived type.

use guatiao::schema::read::SchemaRef;
use guatiao::schema::{differences, differences_value, inline_refs};
use guatiao::value::convert::TryAsRef;
use guatiao::value::read::Dump;
use guatiao::{List, Map, Value};

#[path = "vectors/read.rs"]
mod vectors;

const HIDDEN: &str = "***";

fn doc() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vectors/differences.json"
    );
    let text = std::fs::read_to_string(path).expect("the vector file is readable");
    serde_json::from_str(&text).expect("the vector file is JSON")
}

fn run(schema: &Value, value: &Value, baseline: Option<&Value>) -> Value {
    differences_value(SchemaRef::new(schema).unwrap(), value, baseline, HIDDEN).unwrap()
}

#[test]
fn every_vector_reports_exactly_its_differences() {
    let doc = doc();
    assert_eq!(doc["placeholder"], HIDDEN);
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 15);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let schema = vectors::to_value(&case["schema"]);
        let value = vectors::to_value(&case["value"]);
        let baseline = case.get("baseline").map(vectors::to_value);
        let expected = vectors::to_value(&case["expected"]);
        let got = run(&schema, &value, baseline.as_ref());
        same(&got, &expected, name);
    }
}

#[test]
fn the_rust_result_carries_the_same_paths_and_keys() {
    let doc = doc();
    for case in doc["cases"].as_array().unwrap() {
        let schema = vectors::to_value(&case["schema"]);
        let value = vectors::to_value(&case["value"]);
        let baseline = case.get("baseline").map(vectors::to_value);
        let found = differences(
            SchemaRef::new(&schema).unwrap(),
            &value,
            baseline.as_ref(),
            HIDDEN,
        )
        .unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(found.len(), expected.len(), "{}", case["name"]);
        for (d, e) in found.iter().zip(expected) {
            assert_eq!(d.key(), e["key"].as_str().unwrap());
            let path: Vec<&str> = e["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap())
                .collect();
            assert_eq!(d.path, path);
            assert_eq!(d.value.is_some(), e.get("value").is_some());
            assert_eq!(d.baseline.is_some(), e.get("default").is_some());
        }
    }
}

fn list_schema() -> Value {
    vectors::to_value(&serde_json::json!({
        "type": "object",
        "properties": {
            "suites": {"type": "array", "items": {"type": "string"}, "default": ["a", "b"]}
        }
    }))
}

/// A list recursed per element would report `suites[0]` and `suites[1]`.
#[test]
fn a_list_is_one_leaf() {
    let value = vectors::to_value(&serde_json::json!({"suites": ["b", "a", "c"]}));
    let got = run(&list_schema(), &value, None);
    let found = <&List>::try_from(&got).unwrap();
    assert_eq!(found.len(), 1);
    let entry = <&Map>::try_from(&found[0]).unwrap();
    assert_eq!(
        <&str>::try_from(entry.get("key").unwrap()).unwrap(),
        "suites"
    );
}

fn secret_schema() -> Value {
    vectors::to_value(&serde_json::json!({
        "type": "object",
        "properties": {
            "password": {"type": "string", "x-sensitive": true, "default": ""}
        }
    }))
}

/// Neither side of a changed secret is ever shown, and the text of the
/// report holds neither value.
#[test]
fn a_changed_secret_is_never_shown() {
    let value = vectors::to_value(&serde_json::json!({"password": "hunter2"}));
    let baseline = vectors::to_value(&serde_json::json!({"password": "hunter1"}));
    let got = run(&secret_schema(), &value, Some(&baseline));
    let found = <&List>::try_from(&got).unwrap();
    assert_eq!(found.len(), 1);
    let entry = <&Map>::try_from(&found[0]).unwrap();
    assert_eq!(
        <&str>::try_from(entry.get("value").unwrap()).unwrap(),
        HIDDEN
    );
    assert_eq!(
        <&str>::try_from(entry.get("default").unwrap()).unwrap(),
        HIDDEN
    );
    let shown = format!("{:?}", Dump(&got));
    assert!(!shown.contains("hunter"), "{shown}");
}

fn referring() -> Value {
    vectors::to_value(&serde_json::json!({
        "type": "object",
        "properties": {
            "p": {"$ref": "#/$defs/Pass"},
            "n": {"$ref": "#/$defs/Name"}
        },
        "$defs": {
            "Pass": {"type": "string", "x-sensitive": true},
            "Name": {"type": "string", "default": "a"}
        }
    }))
}

/// The caller inlines first, as for redact: a mark on a definition then
/// counts. A `$ref` left in is one leaf shown as the placeholder.
#[test]
fn a_reference_is_inlined_by_the_caller_or_hidden() {
    let value = vectors::to_value(&serde_json::json!({"p": "s3cret", "n": "b"}));
    let baseline = vectors::to_value(&serde_json::json!({"p": "other", "n": "a"}));

    let inlined = inline_refs(&referring()).unwrap();
    let got = run(&inlined, &value, Some(&baseline));
    let expected = vectors::to_value(&serde_json::json!([
        {"path": ["n"], "key": "n", "value": "b", "default": "a"},
        {"path": ["p"], "key": "p", "value": "***", "default": "***"}
    ]));
    same(&got, &expected, "inlined");

    let left = run(&referring(), &value, Some(&baseline));
    let expected = vectors::to_value(&serde_json::json!([
        {"path": ["n"], "key": "n", "value": "***", "default": "***"},
        {"path": ["p"], "key": "p", "value": "***", "default": "***"}
    ]));
    same(&left, &expected, "left in");
}

#[cfg(feature = "derive")]
mod derived {
    use super::*;
    use core::time::Duration;
    use guatiao::{Schema, ToValue};

    #[derive(Debug, PartialEq, Schema, ToValue)]
    struct Waiting {
        #[map(default = 1)]
        per_source: u32,
        #[map(default = 4)]
        max: u32,
    }

    impl Default for Waiting {
        fn default() -> Self {
            Waiting {
                per_source: 1,
                max: 4,
            }
        }
    }

    #[derive(Debug, PartialEq, Schema, ToValue)]
    struct Settings {
        #[map(default)]
        name: String,
        #[map(default)]
        cipher_suites: Vec<String>,
        #[map(default)]
        waiting: Waiting,
        #[map(default = Duration::from_secs(5))]
        timeout: Duration,
        nickname: Option<String>,
    }

    impl Default for Settings {
        fn default() -> Self {
            Settings {
                name: String::new(),
                cipher_suites: Vec::new(),
                waiting: Waiting::default(),
                timeout: Duration::from_secs(5),
                nickname: None,
            }
        }
    }

    fn report(settings: &Settings) -> Value {
        let schema = Settings::schema().unwrap();
        let value = settings.to_value().unwrap();
        differences_value(SchemaRef::new(&schema).unwrap(), &value, None, HIDDEN).unwrap()
    }

    fn keys(report: &Value) -> Vec<String> {
        <&List>::try_from(report)
            .unwrap()
            .iter()
            .map(|d| {
                let entry = <&Map>::try_from(d).unwrap();
                let key = TryAsRef::<str>::try_as_ref(entry.get("key").unwrap());
                key.unwrap().to_string()
            })
            .collect()
    }

    #[test]
    fn the_default_value_has_no_differences() {
        assert!(keys(&report(&Settings::default())).is_empty());
    }

    #[test]
    fn one_changed_nested_field_reports_exactly_its_key() {
        let mut settings = Settings::default();
        settings.waiting.per_source = 3;
        assert_eq!(keys(&report(&settings)), ["waiting.per_source"]);
    }

    #[test]
    fn lists_durations_and_optionals_are_each_one_key() {
        let settings = Settings {
            cipher_suites: vec!["a".into(), "b".into()],
            timeout: Duration::from_millis(1500),
            nickname: Some("bo".into()),
            ..Settings::default()
        };
        assert_eq!(
            keys(&report(&settings)),
            ["cipher_suites", "nickname", "timeout"]
        );
    }
}

/// A tree as text with every map's keys sorted, so two maps holding the
/// same entries compare equal whatever order they were built in.
fn canon(v: &Value) -> String {
    if let Some(map) = TryAsRef::<Map>::try_as_ref(v) {
        let mut parts: Vec<String> = map
            .into_iter()
            .map(|e| format!("{:?}: {}", e.key(), canon(e.value())))
            .collect();
        parts.sort();
        return format!("{{{}}}", parts.join(", "));
    }
    if let Some(list) = TryAsRef::<List>::try_as_ref(v) {
        let parts: Vec<String> = list.iter().map(canon).collect();
        return format!("[{}]", parts.join(", "));
    }
    format!("{:?}", Dump(v))
}

/// `got` and `expected` hold the same entries, printing both when not.
fn same(got: &Value, expected: &Value, what: &str) {
    let (got, expected) = (canon(got), canon(expected));
    assert!(got == expected, "{what}\n got: {got}\nwant: {expected}");
}
