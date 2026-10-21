// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The derive shapes beside a struct of named fields: each writes what
//! serde would, reads it back, and its schema accepts it.

#![cfg(feature = "derive")]
#![allow(dead_code)]

use core::fmt::Debug;
use core::time::Duration;

use guatiao::schema::read::SchemaRef;
use guatiao::schema::validate::validate_map;
use guatiao::schema::{FieldBuilder, SchemaBuilder};
use guatiao::{Alloc, FromValue, List, Map, MapError, Schema, ToValue, Value, ValueError};

fn check<T: ToValue + FromValue + Schema + PartialEq + Debug>(v: T) -> Value {
    let value = v.to_value().expect("writes");
    assert_eq!(T::from_value(&value).expect("reads back"), v);
    let schema = SchemaBuilder::new()
        .field(FieldBuilder::new("f", T::kind(Alloc::rust())).required())
        .finish()
        .expect("a schema");
    let mut doc = Map::new();
    doc.set("f", value.clone());
    validate_map(SchemaRef::new(&schema).unwrap(), &Value::from(doc))
        .unwrap_or_else(|e| panic!("{v:?}'s schema refuses what it wrote: {e}"));
    value
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Port(u16);

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Pair(String, u8);

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Marker;

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
#[map(transparent)]
struct Host {
    name: String,
    #[map(skip)]
    resolved: bool,
}

#[test]
fn a_newtype_is_its_field() {
    assert_eq!(check(Port(5900)), 5900u16.to_value().unwrap());
    assert_eq!(
        Port::kind(Alloc::rust()).finish(),
        u16::kind(Alloc::rust()).finish()
    );
    assert_eq!(
        check(Host {
            name: "a".into(),
            resolved: false
        }),
        "a".to_value().unwrap()
    );
}

#[test]
fn a_tuple_struct_is_a_list_by_position() {
    let v = check(Pair("x".into(), 7));
    assert_eq!(v, ("x".to_string(), 7u8).to_value().unwrap());
    assert_eq!(
        Pair::kind(Alloc::rust()).finish(),
        <(String, u8)>::kind(Alloc::rust()).finish()
    );
    let short = vec!["x".to_string()].to_value().unwrap();
    assert!(matches!(
        Pair::from_value(&short),
        Err(MapError::BadValue { .. })
    ));
    let wrong = ("x".to_string(), "y".to_string()).to_value().unwrap();
    assert_eq!(Pair::from_value(&wrong).unwrap_err().key(), "[1]");
}

#[test]
fn a_unit_struct_is_null() {
    assert_eq!(check(Marker), Value::null());
    assert!(Marker::from_value(&Value::from(1i64)).is_err());
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Labelled<T> {
    label: String,
    value: T,
    extra: Option<T>,
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Grid<const N: usize> {
    cells: [u8; N],
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
#[map(tag = "kind")]
enum Either<A, B> {
    Left { a: A },
    Right { b: B },
}

#[derive(Debug, PartialEq, ToValue, Schema)]
struct Borrowed<'a> {
    name: &'a str,
}

#[test]
fn a_generic_type_is_bounded_by_the_trait_for_each_parameter() {
    check(Labelled {
        label: "p".into(),
        value: Port(1),
        extra: None,
    });
    check(Labelled {
        label: "n".into(),
        value: 3u64,
        extra: Some(4),
    });
    check(Grid { cells: [1, 2, 3] });
    check(Either::<u8, String>::Left { a: 1 });
    check(Either::<u8, String>::Right { b: "r".into() });
    // A borrowed field writes; nothing a reader returns can borrow.
    let v = Borrowed { name: "x" }.to_value().unwrap();
    let schema = Borrowed::schema().unwrap();
    validate_map(SchemaRef::new(&schema).unwrap(), &v).unwrap();
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Inner {
    a: u8,
    b: Option<String>,
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
#[map(tag = "type")]
enum Shape {
    Circle(Inner),
    Square { side: u8 },
    Nothing,
}

#[test]
fn a_newtype_arm_writes_its_map_beside_the_tag() {
    let v = check(Shape::Circle(Inner {
        a: 1,
        b: Some("x".into()),
    }));
    let keys: Vec<&str> = <&Map>::try_from(&v).unwrap().keys().collect();
    assert_eq!(keys, ["type", "a", "b"]);
    check(Shape::Square { side: 2 });
    check(Shape::Nothing);
    let schema = Shape::schema().unwrap();
    let mut extra = Map::new();
    extra.set("type", "Circle");
    extra.set("a", 1i64);
    extra.set("side", 1i64);
    assert!(
        validate_map(SchemaRef::new(&schema).unwrap(), &extra.into()).is_err(),
        "the arm declares its type's fields and no others"
    );
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
#[map(tag = "type")]
enum Scalar {
    Number(u8),
}

#[test]
fn a_newtype_arm_of_a_type_that_writes_no_map_is_refused_at_run_time() {
    // As serde's internally tagged newtype: the shape compiles, and a
    // scalar has no keys to write beside the tag.
    assert_eq!(Scalar::Number(1).to_value(), Err(ValueError::WrongKind));
    assert_eq!(Scalar::schema(), Err(ValueError::WrongKind));
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Outer {
    name: String,
    #[map(flatten)]
    inner: Inner,
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
struct Clashing {
    a: u8,
    #[map(flatten)]
    inner: Inner,
}

#[test]
fn a_flattened_member_writes_its_keys_beside_the_others() {
    let o = Outer {
        name: "n".into(),
        inner: Inner { a: 1, b: None },
    };
    let v = o.to_value().unwrap();
    let keys: Vec<&str> = <&Map>::try_from(&v).unwrap().keys().collect();
    assert_eq!(keys, ["name", "a"]);
    assert_eq!(Outer::from_value(&v).unwrap(), o);
    let schema = Outer::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    validate_map(s, &v).unwrap();
    assert!(s.find("a").unwrap().is_required());
    assert!(!s.find("b").unwrap().is_required());
    // Two fields on one key: refused when written, and by the schema.
    let c = Clashing {
        a: 1,
        inner: Inner { a: 2, b: None },
    };
    assert_eq!(c.to_value(), Err(ValueError::DuplicateKey));
    assert_eq!(Clashing::schema(), Err(ValueError::DuplicateKey));
}

mod secs {
    use core::time::Duration;
    use guatiao::schema::KindBuilder;
    use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError};

    pub fn to_value_in(d: &Duration, alloc: Alloc) -> Result<Value, ValueError> {
        d.as_secs().to_value_in(alloc)
    }

    pub fn from_value(v: &Value) -> Result<Duration, MapError> {
        u64::from_value(v).map(Duration::from_secs)
    }

    pub fn kind(alloc: Alloc) -> KindBuilder {
        u64::kind(alloc)
    }
}

#[derive(Debug, PartialEq, ToValue, FromValue, Schema)]
#[map(deny_unknown_fields)]
struct Tuned {
    #[map(with = "secs")]
    timeout: Duration,
    #[map(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
}

#[test]
fn with_skip_serializing_if_and_deny_unknown_fields() {
    let t = Tuned {
        timeout: Duration::from_secs(30),
        tags: vec![],
    };
    let v = t.to_value().unwrap();
    let map = <&Map>::try_from(&v).unwrap();
    assert_eq!(map.len(), 1, "an empty list is left out");
    assert_eq!(map.get("timeout"), Some(&Value::from(30i64)));
    assert_eq!(Tuned::from_value(&v).unwrap(), t);
    let schema = Tuned::schema().unwrap();
    validate_map(SchemaRef::new(&schema).unwrap(), &v).unwrap();

    let mut more = Map::new();
    more.set("timeout", 1i64);
    more.set("colour", "red");
    let refused = Tuned::from_value(&more.into()).unwrap_err();
    assert_eq!(refused.key(), "colour");

    let t = Tuned {
        timeout: Duration::from_secs(1),
        tags: vec!["a".into()],
    };
    check(t);
}

#[test]
fn shapes_nest() {
    let list: List = [Pair("a".into(), 1), Pair("b".into(), 2)]
        .iter()
        .map(|p| p.to_value().unwrap())
        .collect();
    assert_eq!(Vec::<Pair>::from_value(&list.into()).unwrap().len(), 2);
    check(vec![Marker, Marker]);
    check(Some(Port(1)));
}
