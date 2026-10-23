// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What intake does with an enum told apart by kind (`#[map(untagged)]`):
//! the field is a union. A scalar arm is its text under the field's key;
//! a map or a list arm has no flat spelling, so `flatten` answers `false`
//! and the field is entered as a whole value.

#![cfg(feature = "derive")]
extern crate alloc;

use alloc::collections::BTreeMap;

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_intake::{Form, Screen, check, flatten, unflatten};

#[derive(ToValue, FromValue, Schema, Debug, PartialEq)]
#[map(untagged, rename_all = "lowercase")]
enum Tls {
    Identity,
    Off,
    Files { certificate: String, key: String },
}

#[derive(ToValue, FromValue, Schema, Form, Debug, PartialEq)]
struct Server {
    host: String,
    tls: Tls,
}

#[test]
fn the_field_is_a_union_and_a_form_naming_it_fits() {
    let schema = Server::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    assert!(matches!(s.find("tls").unwrap().kind(), Kind::Union(_)));
    let form = Server::form(guatiao::Alloc::rust()).unwrap();
    check(s, guatiao_intake::FormRef::new(&form).unwrap()).expect("the form fits");
}

#[test]
fn a_string_arm_is_its_text_and_reads_back() {
    let schema = Server::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let field = s.find("tls").unwrap();
    let mut store = BTreeMap::new();
    let value = Tls::Identity.to_value().unwrap();
    assert!(flatten(field, &value, &mut store));
    assert_eq!(store.get("tls").map(String::as_str), Some("identity"));
    let back = unflatten(guatiao::Alloc::rust(), field, &store).unwrap();
    assert_eq!(Tls::from_value(&back).unwrap(), Tls::Identity);
}

#[test]
fn a_map_arm_has_no_flat_spelling() {
    let schema = Server::schema().unwrap();
    let s = SchemaRef::new(&schema).unwrap();
    let field = s.find("tls").unwrap();
    let mut store = BTreeMap::from([("tls".to_string(), "off".to_string())]);
    let files = Tls::Files {
        certificate: "c.pem".into(),
        key: "k.pem".into(),
    };
    assert!(!flatten(field, &files.to_value().unwrap(), &mut store));
    assert!(store.is_empty(), "what the field held before is cleared");
}
