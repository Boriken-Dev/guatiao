// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A derived type holding another crate's open table: `toml::Table` and
//! `toml::Value` with `toml`, `serde_json::Map` and `serde_json::Value`
//! with `json`. Each writes, reads back unchanged, and its type's own
//! schema accepts what it wrote.

use core::fmt::Debug;

use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::{FromValue, Schema, ToValue, Value};

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

/// The text under `key` of a map value.
fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    <&guatiao::Map>::try_from(value)
        .ok()?
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
}

#[cfg(feature = "toml")]
mod toml_types {
    use super::*;
    use guatiao::MapError;
    use guatiao_serde::Presentation;
    use guatiao_serde::text::toml as toml_text;

    #[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
    struct Plugin {
        name: String,
        /// Whatever the plugin reads, which this type does not describe.
        settings: toml::Table,
        extra: Option<toml::Value>,
        layers: Vec<toml::Table>,
    }

    const DOCUMENT: &str = r#"
name = "scaler"
extra = [1, "two", true]

[settings]
zeta = 1
alpha = 1.5
big = 9223372036854775807
mode = "fast"

[settings.nested]
deep = ["a", { b = false }]

[[layers]]
size = 3

[[layers]]
"#;

    fn plugin() -> Plugin {
        let mut whole: toml::Table = DOCUMENT.parse().expect("a TOML document");
        Plugin {
            name: "scaler".to_string(),
            settings: whole
                .remove("settings")
                .and_then(|v| v.as_table().cloned())
                .expect("settings"),
            extra: whole.remove("extra"),
            layers: whole
                .remove("layers")
                .and_then(|v| v.as_array().cloned())
                .expect("layers")
                .into_iter()
                .map(|v| v.as_table().cloned().expect("a table"))
                .collect(),
        }
    }

    #[test]
    fn a_struct_holding_tables_round_trips_and_validates() {
        let value = check(&plugin());
        assert_eq!(text_at(&value, "name"), Some("scaler"));
    }

    #[test]
    fn reading_the_document_through_guatiao_gives_the_same_struct() {
        let read = toml_text::from_str(DOCUMENT, guatiao::Alloc::rust(), Presentation::new())
            .expect("a TOML document");
        assert_eq!(Plugin::from_value(&read).expect("reads"), plugin());
        // The table a document holds is the tree the table writes.
        let settings = <&guatiao::Map>::try_from(&read)
            .unwrap()
            .get("settings")
            .unwrap();
        assert_eq!(settings, &plugin().settings.to_value().expect("writes"));
    }

    #[test]
    fn a_table_keeps_its_order_and_its_numbers() {
        let settings = plugin().settings;
        let value = settings.to_value().expect("writes");
        let keys: Vec<&str> = <&guatiao::Map>::try_from(&value)
            .expect("a map")
            .entries()
            .iter()
            .map(|e| e.key())
            .collect();
        assert_eq!(keys, ["zeta", "alpha", "big", "mode", "nested"]);
        let big = <&guatiao::Map>::try_from(&value)
            .unwrap()
            .get("big")
            .unwrap();
        assert_eq!(
            guatiao::TryAsRef::<guatiao::Number>::try_as_ref(big).map(AsRef::<str>::as_ref),
            Some("9223372036854775807")
        );
    }

    #[test]
    fn a_date_crosses_as_its_text() {
        let table: toml::Table = "when = 1979-05-27T07:32:00Z".parse().unwrap();
        let value = table.to_value().unwrap();
        assert_eq!(text_at(&value, "when"), Some("1979-05-27T07:32:00Z"));
        let back = toml::Table::from_value(&value).unwrap();
        assert_eq!(
            back["when"],
            toml::Value::String("1979-05-27T07:32:00Z".to_string())
        );
    }

    #[test]
    fn what_toml_cannot_hold_is_refused_by_key() {
        let mut map = guatiao::Map::new();
        map.set("missing", Value::null());
        assert!(matches!(
            toml::Table::from_value(&map.into()),
            Err(MapError::BadValue { key, .. }) if key == "missing"
        ));
        let wide = guatiao::Number::new("123456789012345678901234567890").unwrap();
        assert!(toml::Value::from_value(&wide.into()).is_err());
        let float = guatiao::Number::new("1.25").unwrap();
        assert_eq!(
            toml::Value::from_value(&float.into()).unwrap(),
            toml::Value::Float(1.25)
        );
    }

    #[test]
    fn a_table_is_an_open_object_and_a_value_is_any_value() {
        let schema = Plugin::schema().unwrap();
        let s = SchemaRef::new(&schema).unwrap();
        assert!(matches!(
            s.find("settings").unwrap().kind(),
            guatiao::schema::read::Kind::MapOf(_)
        ));
        assert!(matches!(
            s.find("extra").unwrap().kind(),
            guatiao::schema::read::Kind::Unknown(_)
        ));
    }
}

#[cfg(feature = "json")]
mod json_types {
    use super::*;
    use serde_json::json;

    #[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
    struct Hook {
        url: String,
        headers: serde_json::Map<String, serde_json::Value>,
        body: serde_json::Value,
        retries: Vec<serde_json::Value>,
    }

    fn hook() -> Hook {
        let serde_json::Value::Object(headers) = json!({"accept": "text/plain", "x-n": 3}) else {
            unreachable!()
        };
        Hook {
            url: "https://example.invalid/".to_string(),
            headers,
            body: json!({"a": [1, 2.5, null, true, "x"], "b": {"c": -7}}),
            retries: vec![json!(null), json!(18446744073709551615u64)],
        }
    }

    #[test]
    fn a_struct_holding_json_round_trips_and_validates() {
        let value = check(&hook());
        assert_eq!(text_at(&value, "url"), Some("https://example.invalid/"));
    }

    #[test]
    fn a_u64_number_keeps_its_digits() {
        let value = json!(18446744073709551615u64).to_value().unwrap();
        assert_eq!(
            guatiao::TryAsRef::<guatiao::Number>::try_as_ref(&value).map(AsRef::<str>::as_ref),
            Some("18446744073709551615")
        );
    }

    #[test]
    fn bytes_have_no_json_spelling() {
        let bytes = guatiao::Bytes(vec![1, 2]).to_value().unwrap();
        assert!(serde_json::Value::from_value(&bytes).is_err());
    }
}
