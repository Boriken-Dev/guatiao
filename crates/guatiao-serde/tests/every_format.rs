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
use guatiao::value::types::{List, Map, Number, Value};
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
            .map(AsRef::<str>::as_ref),
        Some("5900"),
        "{what}"
    );
    assert_eq!(
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("ratio"))
            .and_then(TryAsRef::<Number>::try_as_ref)
            .map(AsRef::<str>::as_ref),
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

/// **With `exact-numbers`**, a number keeps its spelling through a JSON
/// round trip — not merely its magnitude.
#[test]
#[cfg(feature = "exact-numbers")]
fn json_keeps_a_numbers_spelling() {
    use guatiao_serde::text::json;

    for text in ["1.10", "1.0", "123456789012345678901234567890"] {
        let v = Value::from(Number::new(text).unwrap());
        let doc = json::to_string(&v, Presentation::new()).unwrap();
        assert_eq!(doc, text, "written verbatim");
        let back = json::from_str(&doc, Alloc::rust(), Presentation::new()).unwrap();
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&back).map(AsRef::<str>::as_ref),
            Some(text),
            "and read back verbatim"
        );
    }
}

/// Without it, a number is written verbatim and read through an `f64`
/// when it is wider than an integer: the asymmetry the README states.
#[test]
#[cfg(all(feature = "json", not(feature = "exact-numbers")))]
fn json_without_exact_numbers_writes_verbatim_and_reads_an_f64() {
    use guatiao_serde::text::json;

    let v = Value::from(Number::new("1.10").unwrap());
    let doc = json::to_string(&v, Presentation::new()).unwrap();
    assert_eq!(doc, "1.10", "written verbatim");
    let back = json::from_str(&doc, Alloc::rust(), Presentation::new()).unwrap();
    assert_eq!(
        TryAsRef::<Number>::try_as_ref(&back).map(AsRef::<str>::as_ref),
        Some("1.1"),
        "read through serde_json's own f64"
    );
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
    use guatiao::value::types::Text;
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

// --- what a document says is what is read -------------------------------

/// The keys of a map, in the order it holds them.
#[cfg(any(feature = "toml", feature = "yaml"))]
fn keys_of(v: &Value) -> Vec<String> {
    TryAsRef::<Map>::try_as_ref(v)
        .expect("a map")
        .entries()
        .iter()
        .map(|entry| entry.key().to_string())
        .collect()
}

/// A table keeps the order it was written in, as JSON and YAML do. Asserted
/// on the order and not only on lookup: a sorted read still finds every key.
#[test]
#[cfg(feature = "toml")]
fn toml_reads_in_the_order_the_document_wrote() {
    use guatiao_serde::text::toml;

    let doc = "zebra = 1\naardvark = 2\n[m]\nz = 1\na = 2\n[n]\nb = 1\n";
    let v = toml::from_str(doc, Alloc::rust(), Presentation::new()).unwrap();
    assert_eq!(keys_of(&v), ["zebra", "aardvark", "m", "n"]);
    let m = TryAsRef::<Map>::try_as_ref(&v).unwrap().get("m").unwrap();
    assert_eq!(keys_of(m), ["z", "a"]);
}

#[test]
#[cfg(feature = "yaml")]
fn yaml_reads_in_the_order_the_document_wrote() {
    use guatiao_serde::text::yaml;

    let v = yaml::from_str(
        "zebra: 1\naardvark: 2\nm:\n  z: 1\n  a: 2\n",
        Alloc::rust(),
        Presentation::new(),
    )
    .unwrap();
    assert_eq!(keys_of(&v), ["zebra", "aardvark", "m"]);
}

/// All four TOML date and time forms arrive as the text they were written
/// as, and are written back as text: never a table under toml's private
/// key.
#[test]
#[cfg(feature = "toml")]
fn a_toml_date_or_time_reads_as_text() {
    use guatiao_serde::text::toml;

    for text in [
        "1979-05-27T07:32:00Z",
        "1979-05-27T07:32:00",
        "1979-05-27",
        "07:32:00",
    ] {
        let doc = format!("when = {text}\nafter = 1\n");
        let v = toml::from_str(&doc, Alloc::rust(), Presentation::new()).unwrap();
        let map = TryAsRef::<Map>::try_as_ref(&v).unwrap();
        assert_eq!(
            map.get("when").and_then(TryAsRef::<str>::try_as_ref),
            Some(text),
            "`{text}` reads as its text"
        );
        assert_eq!(keys_of(&v), ["when", "after"], "and nothing else");

        let back = toml::to_string(&v, Presentation::new()).unwrap();
        assert_eq!(back, format!("when = \"{text}\"\nafter = 1\n"));
        assert!(!back.contains('$'), "no private key anywhere: {back}");
    }
}

/// A `!!binary` scalar reads as its base64 text, decoded for nobody: the
/// seed is told nothing of the tag, a payload that is not UTF-8 used to be
/// refused, and one that is used to come back as different text.
#[test]
#[cfg(feature = "yaml")]
fn yaml_binary_reads_as_its_base64_text() {
    use guatiao_serde::text::yaml;

    for (doc, text) in [
        ("b: !!binary 3q0=\n", "3q0="),
        ("b: !!binary //4=\n", "//4="),
    ] {
        let v = yaml::from_str(doc, Alloc::rust(), Presentation::new())
            .unwrap_or_else(|e| panic!("`{doc}` is refused: {e}"));
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&v)
                .and_then(|m| m.get("b"))
                .and_then(TryAsRef::<str>::try_as_ref),
            Some(text)
        );
    }
}

/// The wide numbers each format reads, stated: what is exact and what goes
/// through an `f64`.
#[test]
#[cfg(all(feature = "toml", feature = "yaml"))]
fn the_numbers_toml_and_yaml_keep_and_the_ones_they_do_not() {
    use guatiao_serde::text::{toml, yaml};

    let number = |v: &Value| {
        TryAsRef::<Map>::try_as_ref(v)
            .and_then(|m| m.get("n"))
            .and_then(TryAsRef::<Number>::try_as_ref)
            .map(|n| AsRef::<str>::as_ref(n).to_string())
    };
    let toml =
        |doc: &str| number(&toml::from_str(doc, Alloc::rust(), Presentation::new()).unwrap());
    let yaml =
        |doc: &str| number(&yaml::from_str(doc, Alloc::rust(), Presentation::new()).unwrap());

    assert_eq!(
        toml("n = 123456789012345678901234567890").as_deref(),
        Some("123456789012345678901234567890"),
        "a TOML integer is exact"
    );
    assert_eq!(toml("n = 1.10").as_deref(), Some("1.1"));
    assert_eq!(toml("n = 0x10").as_deref(), Some("16"));

    assert_eq!(yaml("n: 1.10").as_deref(), Some("1.1"));
    assert_eq!(yaml("n: 007").as_deref(), Some("7.0"));
    assert_eq!(
        yaml("n: 123456789012345678901234567890").as_deref(),
        Some("1.2345678901234568e29"),
        "a YAML integer past u64 is a float"
    );
    assert_eq!(yaml("n: 0x10").as_deref(), Some("16"));
}

/// What a repeated key does, per format: JSON replaces, TOML and YAML
/// refuse the document.
#[test]
fn a_repeated_key_is_replaced_or_refused_by_format() {
    #[cfg(feature = "json")]
    {
        use guatiao_serde::text::json;
        let v = json::from_str(r#"{"a":1,"a":2}"#, Alloc::rust(), Presentation::new()).unwrap();
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&v)
                .and_then(|m| m.get("a"))
                .and_then(TryAsRef::<Number>::try_as_ref)
                .map(AsRef::<str>::as_ref),
            Some("2")
        );
    }
    #[cfg(feature = "toml")]
    assert!(
        guatiao_serde::text::toml::from_str("a = 1\na = 2\n", Alloc::rust(), Presentation::new())
            .is_err()
    );
    #[cfg(feature = "yaml")]
    assert!(
        guatiao_serde::text::yaml::from_str("a: 1\na: 2\n", Alloc::rust(), Presentation::new())
            .is_err()
    );
}

/// How deep each format reads before it refuses, which is the parser's own
/// bound where it has one and below `MAX_DEPTH` for YAML and TOML.
#[test]
#[cfg(all(feature = "json", feature = "toml", feature = "yaml"))]
fn nesting_is_refused_past_each_formats_own_bound() {
    use guatiao_serde::text::{json, toml, yaml};

    let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
    let ok = |r: Result<Value, guatiao_serde::Error>| r.is_ok();
    let a = Alloc::rust;
    let p = Presentation::new;

    assert!(ok(json::from_str(&nest(100), a(), p())));
    assert!(!ok(json::from_str(&nest(200), a(), p())));
    assert!(ok(toml::from_str(&format!("a = {}", nest(60)), a(), p())));
    assert!(!ok(toml::from_str(&format!("a = {}", nest(200)), a(), p())));
    assert!(ok(yaml::from_str(&nest(60), a(), p())));
    assert!(!ok(yaml::from_str(&nest(200), a(), p())));
}

// --- an error never quotes the document ---------------------------------

/// A document that is not one, with a secret on a line the parser reports.
/// A parser's own message may show the offending line or its neighbours;
/// this crate's error is the message and the position and nothing else.
#[test]
fn a_parse_error_carries_no_text_from_the_document() {
    use guatiao_serde::{Error, Position};

    let secret = "hunter2-SECRET";
    let check = |what: &str, e: Error, at: Option<Position>| {
        let Error::Syntax {
            format,
            position,
            detail,
        } = &e
        else {
            panic!("{what}: a parse refusal is a Syntax error, got {e:?}");
        };
        assert_eq!(*format, what);
        assert_eq!(*position, at, "{what}: {e}");
        assert_eq!(e.position(), at);
        for shown in [format!("{e}"), format!("{e:?}"), detail.clone()] {
            assert!(
                !shown.contains("hunter2") && !shown.contains("SECRET"),
                "{what}: the error shows the document: {shown}"
            );
        }
    };
    let at = |line, column| Some(Position { line, column });

    #[cfg(feature = "json")]
    {
        let doc = format!("{{\"user\": \"alice\", \"password\": \"{secret}\" x}}");
        let e = guatiao_serde::text::json::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        check("json", e, at(1, 48));
    }
    #[cfg(feature = "toml")]
    {
        let doc = format!("user = \"alice\"\npassword = \"{secret}\" trailing\n");
        let e = guatiao_serde::text::toml::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        check("toml", e, at(2, 29));
    }
    #[cfg(feature = "yaml")]
    {
        let doc = format!("user: alice\npassword: \"{secret}\n  x: [");
        let e = guatiao_serde::text::yaml::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        check("yaml", e, at(2, 11));
    }
}

/// A column counts characters in every format: a key written outside
/// ASCII moves it by its length, not by its UTF-8 bytes.
#[test]
fn a_column_counts_characters() {
    use guatiao_serde::{Error, Position};

    let column = |e: Error| e.position().map(|Position { column, .. }| column);
    #[cfg(feature = "json")]
    {
        let json = |doc: &str| {
            guatiao_serde::text::json::from_str(doc, Alloc::rust(), Presentation::new())
                .unwrap_err()
        };
        assert_eq!(column(json("{\"ab\": 1 x}")), Some(10));
        assert_eq!(
            column(json("{\"\u{e9}\u{e9}\": 1 x}")),
            Some(10),
            "two characters of two bytes each"
        );
        assert_eq!(column(json("{\"\u{1f600}\": 1 x}")), Some(9));
        assert_eq!(
            json("{\n\"\u{e9}\": 1 x}").position(),
            Some(Position { line: 2, column: 8 })
        );
    }
    #[cfg(feature = "toml")]
    {
        let toml = |doc: &str| {
            guatiao_serde::text::toml::from_str(doc, Alloc::rust(), Presentation::new())
                .unwrap_err()
        };
        assert_eq!(column(toml("\"ab\" = 1 x\n")), Some(8));
        assert_eq!(column(toml("\"\u{e9}\u{e9}\" = 1 x\n")), Some(8));
    }
    #[cfg(feature = "yaml")]
    {
        let yaml = |doc: &str| {
            guatiao_serde::text::yaml::from_str(doc, Alloc::rust(), Presentation::new())
                .unwrap_err()
        };
        assert_eq!(
            column(yaml("ab: [1, @\n")),
            column(yaml("\u{e9}\u{e9}: [1, @\n"))
        );
    }
}

/// The same for the other things a parser quotes: a key it refuses to see
/// twice, and a value it will not accept.
#[test]
fn other_refusals_do_not_quote_the_document_either() {
    let secret = "hunter2-SECRET";
    let none_of_it = |what: &str, e: guatiao_serde::Error| {
        for shown in [format!("{e}"), format!("{e:?}")] {
            assert!(
                !shown.contains("hunter2") && !shown.contains("SECRET"),
                "{what}: {shown}"
            );
        }
    };

    #[cfg(feature = "toml")]
    for doc in [
        format!("{secret} = 1\n{secret} = 2\n"),
        format!("n = {secret}\n"),
        format!("[{secret}]\n[{secret}]\n"),
    ] {
        let e = guatiao_serde::text::toml::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        none_of_it("toml", e);
    }
    #[cfg(feature = "yaml")]
    for doc in [
        format!("{secret}: 1\n{secret}: 2\n"),
        format!("n: .inf\npassword: {secret}\n"),
        format!("password: {secret}\n\t- x\n"),
        format!("&{secret} a: *{secret}x\n"),
    ] {
        let e = guatiao_serde::text::yaml::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        none_of_it("yaml", e);
    }
    #[cfg(feature = "json")]
    for doc in [
        format!("{{\"{secret}\": 1, \"{secret}\" 2}}"),
        format!("[1, {secret}]"),
    ] {
        let e = guatiao_serde::text::json::from_str(&doc, Alloc::rust(), Presentation::new())
            .unwrap_err();
        none_of_it("json", e);
    }
}
