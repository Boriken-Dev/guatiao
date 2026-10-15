// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A schema refuses what the type it describes refuses: a value is checked
//! as the type its kind names, not as the text it would print.

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::{validate_map, validate_text, validate_value};
use guatiao::{List, Map, Value};

fn one(key: &str, kind: KindBuilder) -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new(key, kind))
        .finish()
        .expect("the schema builds")
}

fn field_of<'a>(schema: &'a Value, key: &str) -> guatiao::schema::read::FieldRef<'a> {
    SchemaRef::new(schema)
        .expect("a schema is a map")
        .find(key)
        .expect("the field is declared")
}

fn list_of(items: Vec<Value>) -> Value {
    let mut list = List::new();
    for item in items {
        list.push(item);
    }
    list.into()
}

fn number(text: &str) -> Value {
    guatiao::Number::new(text).expect("a JSON number").into()
}

fn accepts(schema: &Value, key: &str, value: &Value) -> bool {
    validate_value(field_of(schema, key), value).is_ok()
}

#[test]
fn a_boolean_field_takes_only_a_boolean() {
    let schema = one("verify", KindBuilder::bool());
    assert!(accepts(&schema, "verify", &Value::from(true)));
    assert!(accepts(&schema, "verify", &Value::from(false)));
    for wrong in [Value::from("yes"), Value::from(""), number("1")] {
        assert!(!accepts(&schema, "verify", &wrong), "{wrong:?}");
    }
}

#[test]
fn an_integer_field_takes_only_a_number_that_is_whole() {
    let schema = one("port", KindBuilder::int_range(1, 65535));
    assert!(accepts(&schema, "port", &number("5900")));
    assert!(!accepts(&schema, "port", &Value::from("5900")), "text");
    assert!(!accepts(&schema, "port", &Value::from(true)));
    assert!(!accepts(&schema, "port", &number("5900.5")), "fractional");
    assert!(!accepts(&schema, "port", &number("70000")), "out of bounds");
}

#[test]
fn a_string_field_takes_only_text() {
    let schema = one("host", KindBuilder::string());
    assert!(accepts(&schema, "host", &Value::from("example.org")));
    assert!(!accepts(&schema, "host", &number("5900")));
    assert!(!accepts(&schema, "host", &Value::from(true)));
    assert!(!accepts(&schema, "host", &Value::from(&b"abc"[..])));
}

#[test]
fn a_number_field_takes_only_a_number() {
    let schema = one("ratio", KindBuilder::float_bounds(Some(0.0), Some(1.0)));
    assert!(accepts(&schema, "ratio", &number("0.5")));
    assert!(!accepts(&schema, "ratio", &Value::from("0.5")));
    assert!(!accepts(&schema, "ratio", &number("2")));
}

#[test]
fn the_text_form_reads_a_number_as_json_writes_one() {
    let int = one("n", KindBuilder::int());
    let real = one("x", KindBuilder::float());
    let n = field_of(&int, "n");
    let x = field_of(&real, "x");

    for good in ["0", "-7", "42"] {
        assert!(validate_text(n, good).is_ok(), "{good}");
    }
    for bad in ["007", "+1", "1.0", "1e2", " 1", "", "-", "inf"] {
        assert!(validate_text(n, bad).is_err(), "{bad:?}");
    }
    for good in ["0", "-0.5", "1e3", "2.5E-3"] {
        assert!(validate_text(x, good).is_ok(), "{good}");
    }
    for bad in [
        "inf", "-inf", "NaN", "infinity", "1e999", ".5", "5.", "+1", "01",
    ] {
        assert!(validate_text(x, bad).is_err(), "{bad:?}");
    }
}

#[test]
fn an_integer_enumeration_is_enforced_by_value() {
    let mut allowed = List::new();
    allowed.push(number("1"));
    allowed.push(number("2"));
    let mut schema = Map::new();
    schema.set("type", "integer");
    schema.set("enum", allowed);
    let mut props = Map::new();
    props.set("level", schema);
    let mut root = Map::new();
    root.set("type", "object");
    root.set("properties", props);
    let root = Value::from(root);

    let field = field_of(&root, "level");
    let Kind::Enum(_) = field.kind() else {
        panic!("an enumeration of integers is an enumeration");
    };
    assert_eq!(
        field
            .kind()
            .choices()
            .map(|c| c.value())
            .collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert!(validate_value(field, &number("2")).is_ok());
    assert!(
        validate_value(field, &number("2.0")).is_ok(),
        "the same number"
    );
    assert!(validate_value(field, &number("3")).is_err());
    assert!(
        validate_value(field, &Value::from("2")).is_err(),
        "text is not 2"
    );
    assert!(validate_text(field, "2").is_ok());
    assert!(validate_text(field, "3").is_err());
}

#[test]
fn a_fractional_bound_on_an_integer_rounds_inward() {
    let mut schema = Map::new();
    schema.set("type", "integer");
    schema.set("minimum", number("1.5"));
    schema.set("maximum", number("1e1"));
    let mut props = Map::new();
    props.set("n", schema);
    let mut root = Map::new();
    root.set("type", "object");
    root.set("properties", props);
    let root = Value::from(root);

    let Kind::Int { min, max, .. } = field_of(&root, "n").kind() else {
        panic!("an integer");
    };
    assert_eq!((min, max), (Some(2), Some(10)));
    assert!(!accepts(&root, "n", &number("1")));
    assert!(accepts(&root, "n", &number("2")));
    assert!(accepts(&root, "n", &number("10")));
    assert!(!accepts(&root, "n", &number("11")));
}

#[test]
fn a_union_tries_every_arm_with_the_value_it_was_given() {
    let schema = one(
        "target",
        KindBuilder::union(vec![
            KindBuilder::int(),
            KindBuilder::list(KindBuilder::string()),
        ]),
    );
    assert!(accepts(&schema, "target", &number("5")));
    assert!(accepts(
        &schema,
        "target",
        &list_of(vec![Value::from("x"), Value::from("y")])
    ));
    assert!(
        !accepts(&schema, "target", &Value::from("hello")),
        "text is neither arm"
    );
    assert!(
        !accepts(
            &schema,
            "target",
            &list_of(vec![Value::from("x"), number("1")])
        ),
        "an element of the wrong type"
    );
    assert!(!accepts(&schema, "target", &Value::null()));
}

#[test]
fn a_union_with_a_null_arm_takes_null() {
    let mut null_arm = Map::new();
    null_arm.set("type", "null");
    let mut string_arm = Map::new();
    string_arm.set("type", "string");
    let mut arms = List::new();
    arms.push(string_arm);
    arms.push(null_arm);
    arms.push({
        let mut m = Map::new();
        m.set("type", "boolean");
        m
    });
    let mut field = Map::new();
    field.set("anyOf", arms);
    let mut props = Map::new();
    props.set("v", field);
    let mut root = Map::new();
    root.set("type", "object");
    root.set("properties", props);
    let root = Value::from(root);

    assert!(accepts(&root, "v", &Value::null()));
    assert!(accepts(&root, "v", &Value::from("a")));
    assert!(accepts(&root, "v", &Value::from(true)));
    assert!(!accepts(&root, "v", &number("1")));
}

#[cfg(feature = "derive")]
mod derived {
    use super::*;
    use guatiao::{FromValue, Schema, ToValue};

    #[derive(Debug, PartialEq, Schema, ToValue, FromValue)]
    struct Connection {
        host: String,
        port: u16,
        verify: bool,
    }

    fn doc(host: Value, port: Value, verify: Value) -> Value {
        let mut m = Map::new();
        m.set("host", host);
        m.set("port", port);
        m.set("verify", verify);
        m.into()
    }

    #[test]
    fn what_the_type_refuses_the_schema_refuses_too() {
        let schema = <Connection as Schema>::schema().expect("the derive builds a schema");
        let schema_ref = SchemaRef::new(&schema).unwrap();

        let good = doc(Value::from("h"), number("5900"), Value::from(true));
        assert!(Connection::from_value(&good).is_ok());
        assert!(validate_map(schema_ref, &good).is_ok());

        let wrong = [
            doc(number("5900"), number("5900"), Value::from(true)),
            doc(Value::from("h"), Value::from("5900"), Value::from(true)),
            doc(Value::from("h"), number("5900"), Value::from("yes")),
            doc(Value::from("h"), number("5900"), number("1")),
            doc(Value::from("h"), number("70000"), Value::from(true)),
            doc(Value::from("h"), number("-1"), Value::from(true)),
        ];
        for value in &wrong {
            assert!(
                Connection::from_value(value).is_err(),
                "the type refuses it: {value:?}"
            );
            assert!(
                validate_map(schema_ref, value).is_err(),
                "so does its schema: {value:?}"
            );
        }
    }
}

#[test]
fn a_field_declared_twice_is_refused_when_the_schema_is_finished() {
    let twice = SchemaBuilder::new()
        .field(FieldBuilder::new("a", KindBuilder::int()).required())
        .field(FieldBuilder::new("b", KindBuilder::int()))
        .field(FieldBuilder::new("a", KindBuilder::string()).required())
        .finish();
    assert_eq!(twice, Err(guatiao::ValueError::DuplicateKey));

    // Inside an object kind, and against an arm's discriminant.
    let nested = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "tls",
            KindBuilder::map(vec![
                FieldBuilder::new("verify", KindBuilder::bool()),
                FieldBuilder::new("verify", KindBuilder::bool()),
            ]),
        ))
        .finish();
    assert_eq!(nested, Err(guatiao::ValueError::DuplicateKey));

    let arm_clash = SchemaBuilder::new()
        .field(FieldBuilder::new(
            "auth",
            KindBuilder::variant(
                "kind",
                vec![
                    guatiao::schema::build::ArmBuilder::new("pw", "Password")
                        .field(FieldBuilder::new("kind", KindBuilder::string())),
                ],
            ),
        ))
        .finish();
    assert_eq!(arm_clash, Err(guatiao::ValueError::DuplicateKey));

    // The same names in different objects are different fields.
    let fine = SchemaBuilder::new()
        .field(FieldBuilder::new("a", KindBuilder::int()).required())
        .field(FieldBuilder::new(
            "inner",
            KindBuilder::map(vec![FieldBuilder::new("a", KindBuilder::int())]),
        ))
        .finish();
    assert!(fine.is_ok());
}

#[test]
fn the_required_list_names_each_field_once() {
    use guatiao::schema::vocab;
    use guatiao::value::convert::TryAsRef;
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("a", KindBuilder::int()).required())
        .field(FieldBuilder::new("b", KindBuilder::int()).required())
        .finish()
        .unwrap();
    let required = TryAsRef::<Map>::try_as_ref(&schema)
        .and_then(|m| m.get(vocab::REQUIRED))
        .and_then(TryAsRef::<List>::try_as_ref)
        .expect("a required list");
    let names: Vec<&str> = required
        .iter()
        .filter_map(TryAsRef::<str>::try_as_ref)
        .collect();
    assert_eq!(names, ["a", "b"]);
}

#[test]
fn a_build_says_whether_it_enforces_patterns() {
    assert_eq!(guatiao::schema::PATTERNS_ENFORCED, cfg!(feature = "regex"));
    let schema = one("v", KindBuilder::string().pattern("^(first|all)$"));
    let accepted = validate_text(field_of(&schema, "v"), "bogus").is_ok();
    assert_eq!(
        accepted,
        !guatiao::schema::PATTERNS_ENFORCED,
        "a build that does not enforce patterns accepts what they refuse"
    );
}

#[cfg(feature = "regex")]
#[test]
fn a_pattern_digit_is_ascii_as_ecma_262_has_it() {
    let schema = one("n", KindBuilder::string().pattern(r"^\d+$"));
    let f = field_of(&schema, "n");
    assert!(validate_text(f, "0123456789").is_ok());
    assert!(
        validate_text(f, "\u{663}\u{664}").is_err(),
        "Arabic-Indic digits"
    );
    assert!(validate_value(f, &Value::from("\u{663}")).is_err());

    let word = one("w", KindBuilder::string().pattern(r"^\w+$"));
    let w = field_of(&word, "w");
    assert!(validate_text(w, "snake_Case9").is_ok());
    assert!(
        validate_text(w, "caf\u{e9}").is_err(),
        "word characters are ASCII"
    );

    let not_digit = one("d", KindBuilder::string().pattern(r"^\D+$"));
    assert!(validate_text(field_of(&not_digit, "d"), "abc").is_ok());
    assert!(validate_text(field_of(&not_digit, "d"), "1").is_err());
    assert!(
        validate_text(field_of(&not_digit, "d"), "a\u{663}").is_ok(),
        "an Arabic-Indic digit is not an ASCII digit, so it is a non-digit"
    );

    // A backslash escaped is a backslash, then a letter.
    let literal = one("l", KindBuilder::string().pattern(r"^\\d$"));
    assert!(validate_text(field_of(&literal, "l"), r"\d").is_ok());
    assert!(validate_text(field_of(&literal, "l"), "5").is_err());
}
