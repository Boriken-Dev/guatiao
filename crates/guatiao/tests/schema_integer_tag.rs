// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An integer-typed schema checks the value's tag, wherever the integer
//! sits: text that spells a number is not a number.

use guatiao::schema::build::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::{validate_map, validate_text, validate_value};
use guatiao::{List, Map, Value};

fn one(key: &str, kind: KindBuilder) -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new(key, kind))
        .finish()
        .expect("the schema builds")
}

fn number(text: &str) -> Value {
    guatiao::Number::new(text).expect("a JSON number").into()
}

fn in_map(key: &str, value: Value) -> Value {
    let mut map = Map::new();
    map.set(key, value);
    map.into()
}

fn list(items: Vec<Value>) -> Value {
    let mut list = List::new();
    for item in items {
        list.push(item);
    }
    list.into()
}

fn check(kind: KindBuilder, value: Value) -> bool {
    let schema = one("n", kind);
    validate_map(SchemaRef::new(&schema).unwrap(), &in_map("n", value)).is_ok()
}

#[test]
fn text_of_digits_is_not_an_integer_wherever_the_integer_sits() {
    let digits = || Value::from("5");
    assert!(check(KindBuilder::int(), number("5")));
    assert!(!check(KindBuilder::int(), digits()));
    assert!(!check(KindBuilder::int_range(0, 10), digits()));
    assert!(!check(KindBuilder::float(), digits()));
    assert!(!check(
        KindBuilder::list(KindBuilder::int()),
        list(vec![digits()])
    ));
    assert!(check(
        KindBuilder::list(KindBuilder::int()),
        list(vec![number("5")])
    ));
    assert!(!check(
        KindBuilder::map_of(KindBuilder::int()),
        in_map("k", digits())
    ));
    assert!(!check(
        KindBuilder::tuple(vec![KindBuilder::int()]),
        list(vec![digits()])
    ));
    assert!(!check(
        KindBuilder::map(vec![FieldBuilder::new("p", KindBuilder::int())]),
        in_map("p", digits())
    ));
}

#[test]
fn text_coercion_stays_a_separate_explicit_step() {
    let schema = one("n", KindBuilder::int());
    let field = SchemaRef::new(&schema).unwrap().find("n").unwrap();
    assert!(validate_text(field, "5").is_ok());
    assert!(validate_value(field, &Value::from("5")).is_err());
}

#[test]
fn a_union_and_a_nullable_integer_check_the_tag() {
    let ints = || KindBuilder::union(vec![KindBuilder::int(), KindBuilder::bool()]);
    assert!(check(ints(), number("5")));
    assert!(!check(ints(), Value::from("5")));
    assert!(check(KindBuilder::int().nullable(), Value::null()));
    assert!(!check(KindBuilder::int().nullable(), Value::from("5")));
}
