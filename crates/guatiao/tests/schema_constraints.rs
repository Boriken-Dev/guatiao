// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What JSON Schema can say about a value's shape beyond its kind and its
//! bounds: string lengths and patterns, formats, steps, list sizes.
//!
//! Every renderer builds its input from these -- a `maxlength`, an email
//! keyboard, a slider's step, whether "add" is offered -- so each one has
//! to survive the trip builder -> value -> reader, and each one the
//! validator enforces has a test that it refuses.

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::{validate_text, validate_value};
use guatiao::schema::vocab;
use guatiao::value::convert::TryAsRef;
use guatiao::{List, Map, Number, Value};

fn one(key: &str, kind: KindBuilder) -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new(key, kind))
        .finish()
        .expect("the schema builds")
}

fn list_of(n: usize) -> Value {
    let mut l = List::new();
    for i in 0..n {
        l.push(Value::from(Number::new(&i.to_string()).unwrap()))
            .unwrap();
    }
    l.into()
}

#[test]
fn string_constraints_round_trip_through_the_reader() {
    let schema = one(
        "email",
        KindBuilder::string()
            .min_length(3)
            .max_length(64)
            .pattern("^[^@]+@[^@]+$")
            .format("email"),
    );
    let s = SchemaRef::new(&schema).unwrap();
    let Kind::Str {
        min_length,
        max_length,
        pattern,
        format,
    } = s.find("email").unwrap().kind()
    else {
        panic!("a string with constraints is still a string");
    };
    assert_eq!(min_length, Some(3));
    assert_eq!(max_length, Some(64));
    assert_eq!(pattern, Some("^[^@]+@[^@]+$"));
    assert_eq!(format, Some("email"));
}

#[test]
fn a_plain_string_says_nothing_about_its_shape() {
    let schema = one("name", KindBuilder::string());
    let s = SchemaRef::new(&schema).unwrap();
    assert!(matches!(
        s.find("name").unwrap().kind(),
        Kind::Str {
            min_length: None,
            max_length: None,
            pattern: None,
            format: None
        }
    ));
}

#[test]
fn length_is_counted_in_code_points_not_bytes() {
    let schema = one("code", KindBuilder::string().min_length(2).max_length(3));
    let f = SchemaRef::new(&schema).unwrap().find("code").unwrap();

    assert!(validate_text(f, "ab").is_ok());
    assert!(validate_text(f, "abc").is_ok());
    assert!(validate_text(f, "a").is_err(), "one short");
    assert!(validate_text(f, "abcd").is_err(), "one long");
    // Three code points, six bytes: accepted, where a byte count would
    // refuse it.
    assert!(validate_text(f, "ñño").is_ok(), "ñ is one code point");
    // `e` plus a combining acute is two code points, as JSON Schema
    // counts, though a person sees one character.
    assert!(validate_text(f, "e\u{301}").is_ok(), "two code points");
}

#[test]
fn a_length_refusal_says_what_it_wanted_and_not_what_it_got() {
    let schema = one("pin", KindBuilder::string().max_length(4));
    let f = SchemaRef::new(&schema).unwrap().find("pin").unwrap();
    let err = validate_text(f, "123456789").unwrap_err().to_string();
    assert!(err.contains("at most 4 characters"), "{err}");
    assert!(!err.contains("123456789"), "never echo the value: {err}");
}

#[test]
fn a_format_is_carried_and_never_checked() {
    let schema = one("email", KindBuilder::string().format("email"));
    let f = SchemaRef::new(&schema).unwrap().find("email").unwrap();
    // JSON Schema 2020-12's default: `format` annotates, it does not
    // assert.
    assert!(validate_text(f, "not an email").is_ok());
}

#[cfg(feature = "regex")]
#[test]
fn a_pattern_is_enforced_unanchored() {
    let schema = one("digits", KindBuilder::string().pattern("[0-9]+"));
    let f = SchemaRef::new(&schema).unwrap().find("digits").unwrap();
    assert!(validate_text(f, "42").is_ok());
    assert!(
        validate_text(f, "a1").is_ok(),
        "unanchored, as JSON Schema says"
    );
    assert!(validate_text(f, "none").is_err());

    let anchored = one("digits", KindBuilder::string().pattern("^[0-9]+$"));
    let f = SchemaRef::new(&anchored).unwrap().find("digits").unwrap();
    assert!(
        validate_text(f, "a1").is_err(),
        "`^...$` means the whole string"
    );
}

#[cfg(feature = "regex")]
#[test]
fn a_pattern_this_build_cannot_compile_is_carried_not_enforced() {
    // Lookahead is ECMA-262 and not the `regex` crate's; refusing every
    // value for a schema we cannot evaluate would make the field unusable.
    let schema = one("x", KindBuilder::string().pattern("^(?=a)"));
    let f = SchemaRef::new(&schema).unwrap().find("x").unwrap();
    assert!(validate_text(f, "b").is_ok());
}

#[cfg(not(feature = "regex"))]
#[test]
fn without_the_regex_feature_a_pattern_is_only_carried() {
    let schema = one("digits", KindBuilder::string().pattern("^[0-9]+$"));
    let f = SchemaRef::new(&schema).unwrap().find("digits").unwrap();
    assert!(validate_text(f, "none").is_ok());
}

#[test]
fn an_integer_multiple_is_exact() {
    let schema = one("port", KindBuilder::int_range(0, 1000).multiple_of(5.0));
    let s = SchemaRef::new(&schema).unwrap();
    let f = s.find("port").unwrap();
    assert!(matches!(
        f.kind(),
        Kind::Int {
            multiple_of: Some(5.0),
            ..
        }
    ));
    assert!(validate_text(f, "15").is_ok());
    assert!(validate_text(f, "0").is_ok());
    assert!(validate_text(f, "16").is_err());
    let err = validate_text(f, "16").unwrap_err().to_string();
    assert!(err.contains("a multiple of 5"), "{err}");
    // The document says `5`, not `5.0`: a whole step is written whole,
    // so a strict reader sees an integer where it expects one. Read off the
    // raw document -- `extra` answers annotations only, and `multipleOf`
    // is a keyword.
    let written = TryAsRef::<Map>::try_as_ref(&schema)
        .and_then(|m| m.get(vocab::PROPERTIES))
        .and_then(TryAsRef::<Map>::try_as_ref)
        .and_then(|m| m.get("port"))
        .and_then(TryAsRef::<Map>::try_as_ref)
        .and_then(|m| m.get(vocab::MULTIPLE_OF))
        .and_then(TryAsRef::<Number>::try_as_ref)
        .map(AsRef::<str>::as_ref);
    assert_eq!(written, Some("5"));
    assert!(
        f.extra(vocab::MULTIPLE_OF).is_none(),
        "a keyword, not an annotation"
    );
}

#[test]
fn a_huge_integer_is_checked_exactly_too() {
    let schema = one("n", KindBuilder::int().multiple_of(3.0));
    let f = SchemaRef::new(&schema).unwrap().find("n").unwrap();
    // Past `i64` and past `f64`'s exact integers; `i128` still settles it.
    assert!(validate_text(f, "300000000000000000000000000").is_ok());
    assert!(validate_text(f, "300000000000000000000000001").is_err());
}

#[test]
fn a_real_multiple_forgives_binary_rounding() {
    let schema = one("ratio", KindBuilder::float().multiple_of(0.1));
    let f = SchemaRef::new(&schema).unwrap().find("ratio").unwrap();
    // `0.3 % 0.1` is 0.09999999999999998 in binary. A person meant 3 steps.
    assert!(validate_text(f, "0.3").is_ok());
    assert!(validate_text(f, "1.7").is_ok());
    assert!(validate_text(f, "0.35").is_err());
}

#[test]
fn a_step_of_zero_or_less_is_read_as_absent() {
    // Written through the general door, because the typed one takes a
    // step a caller means; a foreign producer can write anything.
    for bad in ["0", "-2"] {
        let doc = SchemaBuilder::new()
            .field(
                FieldBuilder::new("n", KindBuilder::int())
                    .option(vocab::MULTIPLE_OF, Number::new(bad).unwrap()),
            )
            .finish()
            .unwrap();
        let f = SchemaRef::new(&doc).unwrap().find("n").unwrap();
        assert!(
            matches!(
                f.kind(),
                Kind::Int {
                    multiple_of: None,
                    ..
                }
            ),
            "multipleOf {bad} divides nothing"
        );
        assert!(validate_text(f, "7").is_ok());
    }
}

#[test]
fn a_negative_or_fractional_count_is_read_as_absent() {
    for bad in ["-1", "2.5"] {
        let doc = SchemaBuilder::new()
            .field(
                FieldBuilder::new("s", KindBuilder::string())
                    .option(vocab::MAX_LENGTH, Number::new(bad).unwrap()),
            )
            .finish()
            .unwrap();
        let f = SchemaRef::new(&doc).unwrap().find("s").unwrap();
        assert!(
            matches!(
                f.kind(),
                Kind::Str {
                    max_length: None,
                    ..
                }
            ),
            "maxLength {bad} is no count"
        );
    }
}

#[test]
fn list_sizes_round_trip_and_are_enforced_on_the_value() {
    let schema = one(
        "agents",
        KindBuilder::list(KindBuilder::int())
            .min_items(1)
            .max_items(3),
    );
    let f = SchemaRef::new(&schema).unwrap().find("agents").unwrap();
    assert!(matches!(
        f.kind(),
        Kind::List {
            min: Some(1),
            max: Some(3),
            ..
        }
    ));
    assert!(
        matches!(f.kind().items(), Kind::Int { .. }),
        "items still read"
    );

    assert!(validate_value(f, &list_of(1)).is_ok());
    assert!(validate_value(f, &list_of(3)).is_ok());
    let err = validate_value(f, &list_of(0)).unwrap_err().to_string();
    assert!(err.contains("between 1 and 3 items"), "{err}");
    assert!(validate_value(f, &list_of(4)).is_err());
}

#[test]
fn a_list_of_constrained_strings_checks_each_element() {
    let schema = one(
        "tags",
        KindBuilder::list(KindBuilder::string().max_length(2)),
    );
    let f = SchemaRef::new(&schema).unwrap().find("tags").unwrap();
    let mut ok = List::new();
    ok.push(Value::from(guatiao::Text::new("ab"))).unwrap();
    assert!(validate_value(f, &ok.into()).is_ok());

    let mut bad = List::new();
    bad.push(Value::from(guatiao::Text::new("ab"))).unwrap();
    bad.push(Value::from(guatiao::Text::new("abc"))).unwrap();
    let err = validate_value(f, &bad.into()).unwrap_err().to_string();
    assert!(err.contains("tags[1]"), "the element is named: {err}");
}

#[test]
fn every_new_keyword_is_a_known_key_not_an_annotation() {
    for k in [
        vocab::MULTIPLE_OF,
        vocab::MIN_LENGTH,
        vocab::MAX_LENGTH,
        vocab::PATTERN,
        vocab::FORMAT,
        vocab::MIN_ITEMS,
        vocab::MAX_ITEMS,
    ] {
        assert!(vocab::known(k), "{k} is JSON Schema's own keyword");
    }
}
