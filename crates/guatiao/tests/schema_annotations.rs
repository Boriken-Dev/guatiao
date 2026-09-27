// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JSON Schema's meta-data annotations: what is said ABOUT a value rather
//! than about its shape. `readOnly`, `writeOnly`, `deprecated` and
//! `examples` are carried and read, and none of them makes validation
//! refuse anything -- which is the contract, so it is tested as much as
//! the round trip is.

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::{validate_text, validate_value};
use guatiao::schema::vocab;
use guatiao::value::convert::TryAsRef;
use guatiao::value::read::bool_or;
use guatiao::{Map, Text, Value};

fn one(field: FieldBuilder) -> Value {
    SchemaBuilder::new()
        .field(field)
        .finish()
        .expect("it builds")
}

/// The field's own schema, raw, for asserting what the document says.
fn raw<'a>(schema: &'a Value, key: &str) -> &'a Map {
    TryAsRef::<Map>::try_as_ref(schema)
        .and_then(|m| m.get(vocab::PROPERTIES))
        .and_then(TryAsRef::<Map>::try_as_ref)
        .and_then(|m| m.get(key))
        .and_then(TryAsRef::<Map>::try_as_ref)
        .expect("the field is declared")
}

#[test]
fn a_plain_field_says_none_of_them() {
    let schema = one(FieldBuilder::new("host", KindBuilder::string()));
    let f = SchemaRef::new(&schema).unwrap().find("host").unwrap();
    assert!(!f.is_read_only());
    assert!(!f.is_deprecated());
    assert!(!f.is_sensitive());
    assert!(f.examples().is_empty());
}

#[test]
fn read_only_round_trips_and_refuses_nothing() {
    let schema = one(FieldBuilder::new("serial", KindBuilder::string()).read_only());
    let f = SchemaRef::new(&schema).unwrap().find("serial").unwrap();
    assert!(f.is_read_only());
    assert!(bool_or(raw(&schema, "serial").get(vocab::READ_ONLY), false));
    // "A person may not change this" is about who writes, not about which
    // values are acceptable.
    assert!(validate_text(f, "anything").is_ok());
}

#[test]
fn deprecated_round_trips() {
    let schema = one(FieldBuilder::new("legacy", KindBuilder::bool()).deprecated());
    let f = SchemaRef::new(&schema).unwrap().find("legacy").unwrap();
    assert!(f.is_deprecated());
    assert!(validate_text(f, "yes").is_ok());
}

#[test]
fn examples_keep_their_order_and_are_not_a_whitelist() {
    let schema = one(FieldBuilder::new("host", KindBuilder::string())
        .examples([Text::new("db.internal"), Text::new("10.0.0.1")]));
    let f = SchemaRef::new(&schema).unwrap().find("host").unwrap();
    let shown: Vec<&str> = f
        .examples()
        .iter()
        .filter_map(TryAsRef::<str>::try_as_ref)
        .collect();
    assert_eq!(shown, ["db.internal", "10.0.0.1"]);
    assert!(
        validate_value(f, &Value::from(Text::new("anything.else"))).is_ok(),
        "an example is offered, never required"
    );
}

#[test]
fn a_secret_is_also_write_only_for_tools_that_know_no_x_key() {
    let schema = one(FieldBuilder::new("password", KindBuilder::string()).sensitive());
    let f = SchemaRef::new(&schema).unwrap().find("password").unwrap();
    assert!(f.is_sensitive());
    let doc = raw(&schema, "password");
    assert!(bool_or(doc.get(vocab::X_SENSITIVE), false));
    assert!(
        bool_or(doc.get(vocab::WRITE_ONLY), false),
        "`writeOnly` is JSON Schema's spelling of a secret"
    );
}

#[test]
fn write_only_alone_is_not_sensitive() {
    // `x-sensitive` stays the key this crate reads: "never log" is a
    // stronger promise than "never echoed back", and a producer that only
    // wrote the weaker one did not make the stronger.
    let schema =
        one(FieldBuilder::new("token", KindBuilder::string()).option(vocab::WRITE_ONLY, true));
    let f = SchemaRef::new(&schema).unwrap().find("token").unwrap();
    assert!(!f.is_sensitive());
}

#[test]
fn the_annotations_are_keywords_not_extras() {
    let schema = one(FieldBuilder::new("x", KindBuilder::string())
        .read_only()
        .deprecated()
        .examples([Text::new("a")])
        .sensitive());
    let f = SchemaRef::new(&schema).unwrap().find("x").unwrap();
    for k in [
        vocab::READ_ONLY,
        vocab::WRITE_ONLY,
        vocab::DEPRECATED,
        vocab::EXAMPLES,
    ] {
        assert!(vocab::known(k), "{k} is JSON Schema's own");
        assert!(
            f.extra(k).is_none(),
            "{k} is read by name, not as an annotation"
        );
    }
}
