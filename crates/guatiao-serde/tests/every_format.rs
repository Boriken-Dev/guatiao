// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One value through every format this crate names, plus one it does not.
//!
//! The claim being tested is the reason the crate is shaped this way:
//! **nothing here is format-specific**, so a value that survives JSON
//! survives TOML, YAML and MessagePack without a line written per format.

use guatiao::value::alloc::Alloc;
use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{List, Map, Number, Text, Value};
use guatiao_serde::{Presentation, Serializable, ValueSeed};
use serde::de::DeserializeSeed;

/// A map with something of most kinds in it.
fn a_configuration() -> Value {
    let mut inner = Map::new();
    inner.set("timeout", 30).unwrap();
    inner.set("verify", true).unwrap();

    let mut hosts = List::new();
    hosts.push("alpha").unwrap();
    hosts.push("beta").unwrap();

    let mut map = Map::new();
    map.set("name", "example").unwrap();
    map.set("port", 5900).unwrap();
    map.set("ratio", Number::new("1.5").unwrap()).unwrap();
    map.set("hosts", hosts).unwrap();
    map.set("tls", inner).unwrap();
    map.into()
}

/// What a reader should find, whatever the document was written in.
fn check(what: &str, v: &Value) {
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("name"))
            .and_then(TryAsRef::<str>::try_as_ref),
        Some("example"),
        "{what}"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("port"))
            .and_then(TryAsRef::<Number>::try_as_ref)
            .map(Number::as_str),
        Some("5900"),
        "{what}"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("ratio"))
            .and_then(TryAsRef::<Number>::try_as_ref)
            .map(Number::as_str),
        Some("1.5"),
        "{what}"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("hosts"))
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .map(<[_]>::len),
        Some(2),
        "{what}"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("tls"))
            .and_then(|t| TryAsRef::<Map>::try_as_ref(t).and_then(|m| m.get("verify")))
            .and_then(|v| bool::try_from(v).ok()),
        Some(true),
        "{what}"
    );
}

#[test]
#[cfg(feature = "json")]
fn json_round_trips() {
    use guatiao_serde::text::json;

    let original = a_configuration();
    let text = json::to_string(&original, Presentation::new()).unwrap();
    check(
        "json",
        &json::from_str(&text, Alloc::rust(), Presentation::new()).unwrap(),
    );

    // And the pretty form is the same document.
    let pretty = json::to_string_pretty(&original, Presentation::new()).unwrap();
    assert!(pretty.contains('\n'), "pretty output is indented");
    check(
        "json pretty",
        &json::from_str(&pretty, Alloc::rust(), Presentation::new()).unwrap(),
    );
}

/// **`arbitrary_precision` is on**, so a number keeps its spelling through
/// a JSON round trip — not merely its magnitude.
#[test]
#[cfg(feature = "json")]
fn json_keeps_a_numbers_spelling() {
    use guatiao_serde::text::json;

    for text in ["1.10", "1.0", "123456789012345678901234567890"] {
        let v = Value::from(Number::new(text).unwrap());
        let doc = json::to_string(&v, Presentation::new()).unwrap();
        assert_eq!(doc, text, "written verbatim");
        let back = json::from_str(&doc, Alloc::rust(), Presentation::new()).unwrap();
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&back).map(Number::as_str),
            Some(text),
            "and read back verbatim"
        );
    }
}

/// Trailing rubbish is a malformed document, not a value with a tail.
#[test]
#[cfg(feature = "json")]
fn json_refuses_a_document_with_something_after_it() {
    use guatiao_serde::text::json;

    assert!(json::from_str("{} and then some", Alloc::rust(), Presentation::new()).is_err());
}

#[test]
#[cfg(feature = "toml")]
fn toml_round_trips() {
    use guatiao_serde::text::toml;

    let original = a_configuration();
    let text = toml::to_string(&original, Presentation::new()).unwrap();
    check(
        "toml",
        &toml::from_str(&text, Alloc::rust(), Presentation::new()).unwrap(),
    );
}

/// A TOML document is a TABLE. Writing a bare scalar is refused rather
/// than wrapped in a key the value never had.
#[test]
#[cfg(feature = "toml")]
fn toml_refuses_what_it_cannot_spell() {
    use guatiao_serde::text::toml;

    for v in [
        Value::from(Text::new("bare")),
        Value::from(1i64),
        List::new().into(),
    ] {
        let e = toml::to_string(&v, Presentation::new()).expect_err("not a table");
        assert_eq!(e.format(), "toml");
    }
}

#[test]
#[cfg(feature = "yaml")]
fn yaml_round_trips() {
    use guatiao_serde::text::yaml;

    let original = a_configuration();
    let text = yaml::to_string(&original, Presentation::new()).unwrap();
    check(
        "yaml",
        &yaml::from_str(&text, Alloc::rust(), Presentation::new()).unwrap(),
    );
}

/// **A format this crate names nowhere.**
///
/// MessagePack is reached through `to_serde` and `ValueSeed` alone, which
/// is the whole argument for speaking serde rather than a format: this
/// test needed no code in the crate at all.
#[test]
fn a_format_the_crate_never_heard_of_round_trips() {
    let original = a_configuration();
    let packed = rmp_serde::to_vec(&Serializable::from(&original)).unwrap();
    let mut de = rmp_serde::Deserializer::new(&packed[..]);
    let back = ValueSeed::new(Alloc::rust()).deserialize(&mut de).unwrap();
    check("messagepack", &back);
}
