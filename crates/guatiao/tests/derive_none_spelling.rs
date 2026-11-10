// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(default = <expr>, none = "text")]` on an `Option<T>`: the text
//! is `None`, the inner forms are `Some`, an absent key is the default, and
//! the schema is `anyOf` the one text or the inner kind, with the default
//! the inner value.

#![cfg(feature = "derive")]

extern crate alloc;

use core::fmt;
use core::time::Duration;

use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::value::wire::{decode_as, to_bytes, to_bytes_of};
use guatiao::{FromValue, FromWalk, Map, Schema, Text, ToValue, Value, Walk};

#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
struct Limits {
    host: String,
    #[map(default = Some(Duration::from_secs(30)), none = "never")]
    read: Option<Duration>,
    #[map(default = None, none = "off", rename = "write-cap")]
    write: Option<u16>,
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
#[map(tag = "kind")]
enum Link {
    Quick {
        #[map(default = Some(5u8), none = "unbounded")]
        retries: Option<u8>,
    },
    Slow,
}

fn round_trips<T>(value: &T)
where
    T: ToValue + FromValue + Walk + FromWalk + PartialEq + fmt::Debug,
{
    let tree = value.to_value().expect("the tree is built");
    let bytes = to_bytes(&tree).expect("the tree encodes");
    assert_eq!(to_bytes_of(value).unwrap(), bytes, "walked: {value:?}");
    assert_eq!(&decode_as::<T>(&bytes).unwrap(), value, "read from bytes");
    assert_eq!(&T::from_value(&tree).unwrap(), value, "read from a tree");
}

fn map(entries: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.set(key, value.clone());
    }
    map.into()
}

fn text(text: &str) -> Value {
    Text::new(text).into()
}

fn limits(read: Option<Duration>, write: Option<u16>) -> Limits {
    Limits {
        host: "h".into(),
        read,
        write,
    }
}

#[test]
fn none_is_the_text_and_some_is_the_inner_value() {
    let none = limits(None, None).to_value().unwrap();
    let none = <&Map>::try_from(&none).unwrap();
    assert_eq!(
        <&str>::try_from(none.get("read").unwrap()).unwrap(),
        "never"
    );
    assert_eq!(
        <&str>::try_from(none.get("write-cap").unwrap()).unwrap(),
        "off"
    );

    let some = limits(Some(Duration::from_millis(1500)), Some(9))
        .to_value()
        .unwrap();
    let some = <&Map>::try_from(&some).unwrap();
    assert_eq!(
        Duration::from_value(some.get("read").unwrap()).unwrap(),
        Duration::from_millis(1500)
    );
    assert_eq!(u16::from_value(some.get("write-cap").unwrap()).unwrap(), 9);
}

#[test]
fn every_value_round_trips_through_the_tree_and_the_bytes() {
    round_trips(&limits(None, None));
    round_trips(&limits(Some(Duration::from_secs(30)), Some(0)));
    round_trips(&limits(Some(Duration::from_nanos(7)), None));
    round_trips(&Link::Quick { retries: None });
    round_trips(&Link::Quick { retries: Some(3) });
    round_trips(&Link::Slow);
}

#[test]
fn an_absent_key_is_the_default_and_a_null_is_none() {
    let read = Limits::from_value(&map(&[("host", text("h"))])).unwrap();
    assert_eq!(read, limits(Some(Duration::from_secs(30)), None));

    let null = Limits::from_value(&map(&[
        ("host", text("h")),
        ("read", Value::null()),
        ("write-cap", text("off")),
    ]))
    .unwrap();
    assert_eq!(null, limits(None, None));

    let arm = Link::from_value(&map(&[("kind", text("Quick"))])).unwrap();
    assert_eq!(arm, Link::Quick { retries: Some(5) });
    let arm = Link::from_value(&map(&[
        ("kind", text("Quick")),
        ("retries", text("unbounded")),
    ]))
    .unwrap();
    assert_eq!(arm, Link::Quick { retries: None });
}

#[test]
fn another_text_is_refused_under_the_key() {
    let error = Limits::from_value(&map(&[("host", text("h")), ("read", text("soon"))]))
        .expect_err("not the none text, not a duration");
    assert!(error.to_string().contains("read"), "{error}");
}

fn field<'a>(schema: &'a Value, key: &str) -> FieldRef<'a> {
    SchemaRef::new(schema)
        .unwrap()
        .find(key)
        .unwrap_or_else(|| panic!("no field {key}"))
}

#[test]
fn the_schema_is_any_of_the_text_and_the_inner_kind() {
    let schema = Limits::schema().unwrap();
    let read = field(&schema, "read");
    assert!(!read.is_required(), "a default makes it optional");
    let Kind::Union(_) = read.kind() else {
        panic!("a union: {:?}", read.kind());
    };
    let arms: Vec<Kind<'_>> = read.kind().alternatives().collect();
    assert_eq!(arms.len(), 2);
    let names: Vec<&str> = arms[0].choices().map(|c| c.value()).collect();
    assert_eq!(names, ["never"]);

    // The default is the inner value, written as the inner type writes it.
    let default = read.default().expect("a default");
    assert_eq!(
        Duration::from_value(default).unwrap(),
        Duration::from_secs(30)
    );

    let write = field(&schema, "write-cap");
    let names: Vec<&str> = write
        .kind()
        .alternatives()
        .next()
        .unwrap()
        .choices()
        .map(|c| c.value())
        .collect();
    assert_eq!(names, ["off"]);
    // A default of `None` is written as the text.
    assert_eq!(<&str>::try_from(write.default().unwrap()).unwrap(), "off");
}

#[test]
fn what_the_type_writes_validates_against_its_schema() {
    let schema = Limits::schema().unwrap();
    let schema = SchemaRef::new(&schema).unwrap();
    for value in [
        limits(None, None),
        limits(Some(Duration::from_secs(1)), Some(65535)),
    ] {
        let written = value.to_value().unwrap();
        validate_map(schema, &written).unwrap_or_else(|e| panic!("{value:?}: {e:?}"));
    }
    let wrong = map(&[("host", text("h")), ("read", text("soon"))]);
    assert!(validate_map(schema, &wrong).is_err());
}

#[test]
fn a_tagged_arm_carries_the_same_field() {
    let kind = Link::kind(guatiao::Alloc::rust()).finish().unwrap();
    let link = FieldRef::new("link", &kind).unwrap();
    let arm = link.kind().arms().next().expect("an arm");
    let retries = arm.fields().find(|f| f.key() == "retries").unwrap();
    assert!(!retries.is_required());
    assert_eq!(u8::from_value(retries.default().unwrap()).unwrap(), 5);
}
