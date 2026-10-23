// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(untagged)]`: variants told apart by the kind of value each
//! writes. Each writes, reads back, and its type's own schema accepts what
//! it wrote; reading picks the variant by the value's kind, and what no
//! variant reads is refused naming the kinds, never the value.

#![cfg(feature = "derive")]
extern crate alloc;

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::{FromValue, Kinds, Map, MapError, Schema, ToValue, Value};

/// The certificate a server presents.
#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
#[map(untagged, rename_all = "lowercase")]
enum Tls {
    /// The machine's own identity.
    Identity,
    Off,
    /// A certificate and its key, from files.
    Files {
        certificate: PathBuf,
        key: PathBuf,
    },
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
#[map(untagged)]
enum Port {
    Number(u16),
    Service(String),
    Range((u16, u16)),
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
#[map(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
struct Server {
    tls: Tls,
    fallback: Option<Tls>,
    ports: Vec<Port>,
    names: OneOrMany<String>,
    by_site: BTreeMap<String, Tls>,
}

fn files() -> Tls {
    Tls::Files {
        certificate: "/etc/ssl/c.pem".into(),
        key: "/etc/ssl/k.pem".into(),
    }
}

/// Writes `v`, checks its type's schema accepts it and that it reads back
/// equal.
fn check<T: ToValue + FromValue + Schema + PartialEq + core::fmt::Debug>(v: &T) -> Value {
    let value = v.to_value().expect("writes");
    let schema = T::schema().expect("a schema");
    validate_map(SchemaRef::new(&schema).expect("a map"), &value)
        .expect("its own schema accepts it");
    assert_eq!(&T::from_value(&value).expect("reads back"), v);
    value
}

fn server() -> Server {
    Server {
        tls: Tls::Identity,
        fallback: Some(files()),
        ports: vec![
            Port::Number(443),
            Port::Service("https".into()),
            Port::Range((8000, 8100)),
        ],
        names: OneOrMany::Many(vec!["a".into(), "b".into()]),
        by_site: BTreeMap::from([
            ("north".to_string(), Tls::Off),
            ("south".to_string(), files()),
        ]),
    }
}

#[test]
fn every_arm_round_trips_and_validates() {
    let value = check(&server());
    let map = <&Map>::try_from(&value).unwrap();
    assert_eq!(
        <&str>::try_from(map.get("tls").unwrap()).unwrap(),
        "identity"
    );
    assert!(<&Map>::try_from(map.get("fallback").unwrap()).is_ok());
    check(&Server {
        tls: files(),
        fallback: None,
        ports: Vec::new(),
        names: OneOrMany::One("only".into()),
        by_site: BTreeMap::new(),
    });
}

#[test]
fn a_unit_variant_is_its_name_and_a_record_its_map() {
    assert_eq!(
        <&str>::try_from(&Tls::Off.to_value().unwrap()).unwrap(),
        "off"
    );
    let written = files().to_value().unwrap();
    let map = <&Map>::try_from(&written).unwrap();
    assert_eq!(map.len(), 2, "no tag is stored");
    assert_eq!(
        <&str>::try_from(map.get("key").unwrap()).unwrap(),
        "/etc/ssl/k.pem"
    );
}

#[test]
fn the_kinds_are_the_arms_kinds() {
    assert_eq!(<Tls as FromValue>::KINDS, Kinds::STRING.union(Kinds::MAP));
    assert_eq!(
        <Port as FromValue>::KINDS,
        Kinds::NUMBER.union(Kinds::STRING).union(Kinds::LIST)
    );
    assert_eq!(<Server as FromValue>::KINDS, Kinds::MAP);
    assert_eq!(
        <Option<Tls> as FromValue>::KINDS,
        Kinds::STRING.union(Kinds::MAP).union(Kinds::NULL)
    );
}

#[test]
fn a_name_no_unit_variant_has_is_refused_naming_the_names() {
    let unknown = "maybe".to_value().unwrap();
    assert_eq!(
        Tls::from_value(&unknown),
        Err(MapError::bad_value("one of identity, off"))
    );
}

#[test]
fn a_kind_no_variant_reads_is_refused_naming_the_kinds() {
    let number = 7u8.to_value().unwrap();
    assert_eq!(
        Tls::from_value(&number),
        Err(MapError::bad_value("a string or a map"))
    );
    assert_eq!(
        Port::from_value(&Value::null()),
        Err(MapError::bad_value("a number, a string or a list"))
    );
}

#[test]
fn an_error_inside_an_arm_names_its_key() {
    let mut map = Map::new();
    map.set("certificate", "c.pem");
    match Tls::from_value(&map.into()) {
        Err(MapError::MissingKey { key }) => assert_eq!(key, "key"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_schema_is_a_union_of_the_arms() {
    let kind = Tls::kind(guatiao::Alloc::rust()).finish().unwrap();
    let mut doc = Map::new();
    doc.set("tls", kind);
    let mut props = Map::new();
    props.set("properties", doc);
    let root = Value::from(props);
    let schema = SchemaRef::new(&root).unwrap();
    let field = schema.find("tls").unwrap();
    let Kind::Union(_) = field.kind() else {
        panic!("{:?}", field.kind());
    };
    let arms: Vec<Kind<'_>> = field.kind().alternatives().collect();
    assert_eq!(arms.len(), 2, "the unit variants are one arm");
    let names: Vec<&str> = arms[0].choices().map(|c| c.value()).collect();
    assert_eq!(names, ["identity", "off"]);
    let labels: Vec<&str> = arms[0].choices().map(|c| c.label()).collect();
    assert_eq!(labels[0], "The machine's own identity.");
    assert!(matches!(arms[1], Kind::Map(_)));
}

#[test]
fn validation_refuses_what_no_arm_accepts() {
    let schema = Server::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let mut good = <&Map>::try_from(&server().to_value().unwrap())
        .unwrap()
        .clone();
    good.set("tls", 5u8);
    assert!(validate_map(s, &good.clone().into()).is_err());
    good.set("tls", "maybe");
    assert!(validate_map(s, &good.clone().into()).is_err());
    let mut stray = Map::new();
    stray.set("certificate", "c");
    stray.set("key", "k");
    stray.set("password", "p");
    good.set("tls", stray);
    assert!(
        validate_map(s, &good.into()).is_err(),
        "the record arm declares its keys"
    );
}
