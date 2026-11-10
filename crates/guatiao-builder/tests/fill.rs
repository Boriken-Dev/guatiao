// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `fill_from` over a guatiao value gives the type back: from the value a built type writes,
//! from the value guatiao's own derive writes for the same declaration, and from a form's flat
//! store. A missing key is named, and an undeclared key is refused or listed as its group says.

extern crate alloc;

mod support;

use alloc::collections::BTreeMap;
use core::time::Duration;

use coqui_builder::{Describe, Partial};
use guatiao::{Alloc, ToValue};
use guatiao_builder::{
    FromError, ValueSource, form_source, from_value, from_value_over, partial, to_value,
};
use support::*;

#[test]
fn a_built_value_fills_back() {
    let built = everything();
    let value = to_value(&built).unwrap();
    assert_eq!(
        partial::<Everything>(&value).unwrap().build().unwrap(),
        built
    );
    assert_eq!(from_value::<Everything>(&value).unwrap(), built);
}

#[test]
fn the_value_is_the_one_guatiaos_derive_writes() {
    let built = everything();
    let (ours, theirs) = (to_value(&built).unwrap(), built.to_value().unwrap());
    assert!(
        ours == theirs,
        "(bridge | derive):\n{}",
        diff(&ours, &theirs).join("\n")
    );
    // And guatiao's value reads back through the source.
    assert_eq!(from_value::<Everything>(&theirs).unwrap(), built);
}

#[test]
fn a_value_laid_over_a_base_keeps_the_rest() {
    let mut base = everything();
    base.host = "other".into();
    let value = parse(r#"{"port": 1, "ports": [7]}"#);
    let over = from_value_over(base.clone(), &value).unwrap();
    assert_eq!(over.port, 1);
    assert_eq!(over.ports, [7]);
    assert_eq!(over.host, "other");
    assert_eq!(over.auth, base.auth);
}

#[test]
fn a_missing_required_key_is_named() {
    let Err(FromError::Missing(missing)) = from_value::<Everything>(&parse(r#"{"port": 1}"#))
    else {
        panic!("host and the rest are missing");
    };
    let keys: Vec<String> = missing.keys().iter().map(ToString::to_string).collect();
    assert!(keys.contains(&"host".to_owned()), "{keys:?}");
    assert!(!keys.contains(&"port".to_owned()), "{keys:?}");
}

#[test]
fn a_leaf_that_refuses_its_text_is_a_fill_error_at_its_key() {
    let value = parse(r#"{"port": 70000}"#);
    let Err(FromError::Fill(error)) = from_value::<Everything>(&value) else {
        panic!("70000 is not a u16");
    };
    assert_eq!(error.at().to_string(), "port");
    // The refusal names the key and the reason and never quotes the value.
    assert!(!error.to_string().contains("70000"), "{error}");
}

#[derive(Describe, Debug, PartialEq)]
#[map(deny_unknown_fields)]
struct Strict {
    a: u8,
}

#[derive(Describe, Debug, PartialEq)]
struct Lax {
    a: u8,
    inner: StrictInner,
}

#[derive(Describe, Debug, PartialEq)]
struct StrictInner {
    b: u8,
}

#[test]
fn an_undeclared_key_is_refused_by_a_group_that_says_so() {
    let value = parse(r#"{"a": 1, "extra": 2}"#);
    let Err(error) = partial::<Strict>(&value) else {
        panic!("refused");
    };
    assert!(error.to_string().contains("extra"), "{error}");
}

#[test]
fn an_undeclared_key_is_listed_otherwise() {
    let value = parse(r#"{"a": 1, "extra": 2, "inner": {"b": 3, "more": [1]}}"#);
    let mut filled = Partial::<Lax>::new();
    let unknown = filled.fill_from(&mut ValueSource::new(&value)).unwrap();
    let keys: Vec<String> = unknown.keys().iter().map(ToString::to_string).collect();
    assert_eq!(keys, ["extra", "inner.more"]);
    assert_eq!(
        filled.build().unwrap(),
        Lax {
            a: 1,
            inner: StrictInner { b: 3 }
        }
    );
}

#[test]
fn a_list_or_a_table_where_a_leaf_is_expected_is_refused() {
    for text in [r#"{"a": [1]}"#, r#"{"a": {"x": 1}}"#] {
        let Err(error) = partial::<Strict>(&parse(text)) else {
            panic!("{text} is not a text");
        };
        assert!(error.to_string().contains('a'), "{error}");
    }
}

#[test]
fn null_is_a_key_that_is_not_given() {
    let filled = partial::<Strict>(&parse(r#"{"a": null}"#)).unwrap();
    assert!(filled.build().is_err());
}

// ---- a form's flat store ----

#[derive(Describe, Debug, Clone, PartialEq)]
struct CacheOptions {
    #[map(default = 8)]
    size: u32,
}

#[derive(Describe, Debug, Clone, PartialEq)]
#[map(untagged)]
enum Cache {
    Enabled(bool),
    Tuned(CacheOptions),
}

#[derive(Describe, Debug, Clone, PartialEq)]
struct Screen {
    name: String,
    auth: Login,
    items: Vec<u16>,
    #[map(default = Duration::from_secs(5))]
    idle: Duration,
    cache: Cache,
}

fn store(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[derive(Describe, Debug, Clone, PartialEq)]
#[map(tag = "kind")]
enum Login {
    Password { user: String, password: String },
    Anonymous,
}

#[test]
fn a_flat_store_fills_a_tagged_arm_twelve_items_a_duration_and_a_text_arm() {
    let mut entries = vec![
        ("name", "screen"),
        ("auth", "Password"),
        ("auth.user", "root"),
        ("auth.password", "pw"),
        ("idle", "30s"),
        ("cache", "true"),
    ];
    let indexed: Vec<(String, String)> = (0..12)
        .map(|i| (format!("items[{i}]"), (i * 3).to_string()))
        .collect();
    for (key, value) in &indexed {
        entries.push((key, value));
    }
    let value = form_source(&Screen::description(), &store(&entries), Alloc::rust()).unwrap();
    let built = from_value::<Screen>(&value).unwrap();
    assert_eq!(
        built,
        Screen {
            name: "screen".into(),
            auth: Login::Password {
                user: "root".into(),
                password: "pw".into()
            },
            items: (0..12).map(|i| i * 3).collect(),
            idle: Duration::from_secs(30),
            cache: Cache::Enabled(true),
        }
    );
}

#[test]
fn an_untagged_enum_reads_both_ways_from_a_value() {
    let text =
        parse(r#"{"name": "n", "auth": {"kind": "Anonymous"}, "items": [], "cache": "false"}"#);
    let table =
        parse(r#"{"name": "n", "auth": {"kind": "Anonymous"}, "items": [], "cache": {"size": 4}}"#);
    assert_eq!(
        from_value::<Screen>(&text).unwrap().cache,
        Cache::Enabled(false)
    );
    assert_eq!(
        from_value::<Screen>(&table).unwrap().cache,
        Cache::Tuned(CacheOptions { size: 4 })
    );
    // An empty table still chooses the structured arm, with its defaults.
    let empty = parse(r#"{"name": "n", "auth": {"kind": "Anonymous"}, "items": [], "cache": {}}"#);
    assert_eq!(
        from_value::<Screen>(&empty).unwrap().cache,
        Cache::Tuned(CacheOptions { size: 8 })
    );
    // Written by this crate, the structured arm is a table and the text arm a string.
    let cache_of = |source: &guatiao::Value| {
        let written = to_value(&from_value::<Screen>(source).unwrap()).unwrap();
        let map = <&guatiao::Map>::try_from(&written).unwrap();
        map.get("cache").unwrap().clone()
    };
    assert!(cache_of(&table) == parse(r#"{"size": 4}"#));
    assert!(cache_of(&text) == parse(r#""false""#));
}

#[test]
fn a_duration_reads_a_number_of_seconds_or_its_text_and_not_a_pair() {
    let read = |text: &str| from_value::<Screen>(&parse(text));
    let with = |idle: &str| {
        format!(
            r#"{{"name": "n", "auth": {{"kind": "Anonymous"}}, "items": [], "cache": "true", "idle": {idle}}}"#
        )
    };
    assert_eq!(read(&with("30")).unwrap().idle, Duration::from_secs(30));
    assert_eq!(
        read(&with("0.25")).unwrap().idle,
        Duration::from_millis(250)
    );
    assert_eq!(
        read(&with(r#""1.5h""#)).unwrap().idle,
        Duration::from_secs(5400)
    );
    assert!(
        read(&with("[30, 0]")).is_err(),
        "a pair is guatiao's, not the text"
    );
    assert!(read(&with("-1")).is_err());
}

// ---- the shape of what is read ----

#[test]
fn a_value_of_the_wrong_shape_is_refused_at_its_key() {
    for (text, key) in [
        (r#"{"waiting": 5}"#, "waiting"),
        (r#"{"env": [1]}"#, "env"),
        (r#"{"auth": "Anonymous"}"#, "auth"),
        (r#"{"ports": 5}"#, "ports"),
        (r#"{"waiting": {"total": [1]}}"#, "waiting.total"),
    ] {
        let Err(FromError::Fill(error)) = from_value::<Everything>(&parse(text)) else {
            panic!("{text} has the wrong shape");
        };
        assert_eq!(error.at().to_string(), key, "{text}");
    }
}

#[derive(Describe, Debug, PartialEq)]
struct Texts {
    s: String,
    n: u32,
}

#[test]
fn a_number_is_text_by_the_leafs_type() {
    let read = |text: &str| from_value::<Texts>(&parse(text)).unwrap();
    // A string keeps the number as written (the JSON reader here rounds through f64, so the
    // number is built from its text); an integer is read exactly.
    let mut table = guatiao::Map::new();
    table.set("s", guatiao::Number::new("1.10").unwrap());
    table.set("n", guatiao::Number::new("1e3").unwrap());
    assert_eq!(
        from_value::<Texts>(&guatiao::Value::from(table)).unwrap(),
        Texts {
            s: "1.10".into(),
            n: 1000
        }
    );
    let mut huge = guatiao::Map::new();
    huge.set("s", guatiao::Number::new("1e400").unwrap());
    huge.set("n", guatiao::Number::new("2.0").unwrap());
    let huge = from_value::<Texts>(&guatiao::Value::from(huge)).unwrap();
    assert_eq!((huge.s.as_str(), huge.n), ("1e400", 2));
    assert_eq!(read(r#"{"s": "x", "n": -0}"#).n, 0);
    assert!(from_value::<Texts>(&parse(r#"{"s": "x", "n": 1.5}"#)).is_err());
}

#[derive(Describe, Debug, PartialEq)]
struct Cap {
    #[map(default = Some(5u32), none = "-1")]
    cap: Option<u32>,
}

#[test]
fn the_text_that_writes_a_none_stays_text_even_when_it_looks_like_a_number() {
    let written = to_value(&Cap { cap: None }).unwrap();
    assert!(written == parse(r#"{"cap": "-1"}"#));
    assert_eq!(from_value::<Cap>(&written).unwrap(), Cap { cap: None });
    let document = guatiao_builder::schema(&Cap::description()).unwrap();
    guatiao::schema::validate_map(
        guatiao::schema::SchemaRef::new(&document).unwrap(),
        &written,
    )
    .unwrap();
    assert!(to_value(&Cap { cap: Some(3) }).unwrap() == parse(r#"{"cap": 3}"#));
}

#[test]
fn null_leaves_a_key_as_the_base_has_it() {
    let base = everything();
    let over = from_value_over(base.clone(), &parse(r#"{"nickname": null}"#)).unwrap();
    assert_eq!(over.nickname, base.nickname);
}

#[test]
fn a_list_item_that_writes_nothing_is_an_error() {
    #[derive(Describe)]
    struct Holes {
        items: Vec<Option<u8>>,
    }
    assert!(
        to_value(&Holes {
            items: vec![Some(1), None]
        })
        .is_err()
    );
}

#[test]
fn only_an_object_describes_a_schema() {
    assert!(guatiao_builder::schema(&u8::description()).is_err());
}
