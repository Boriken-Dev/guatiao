// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `#[map(as_text)]` on a `Vec<T>` or an `Option<Vec<T>>`: a list of
//! texts, each the item's `Display` and read with its `FromStr`, and a
//! refusal naming the item's index.

#![cfg(feature = "derive")]

extern crate alloc;

use core::fmt;
use core::net::Ipv4Addr;

use guatiao::schema::read::{Kind, SchemaRef};
use guatiao::schema::validate::validate_map;
use guatiao::value::wire::{decode_as, to_bytes, to_bytes_of};
use guatiao::{FromValue, FromWalk, List, Map, Schema, Text, ToValue, Value, Walk};

#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
struct Peers {
    name: String,
    #[map(default, as_text)]
    addrs: Vec<Ipv4Addr>,
    #[map(as_text)]
    backups: Option<Vec<Ipv4Addr>>,
}

#[derive(ToValue, FromValue, Schema, Debug, PartialEq, Clone)]
#[map(tag = "kind")]
enum Route {
    Static {
        #[map(as_text)]
        via: Vec<Ipv4Addr>,
    },
    Dynamic,
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

fn text(text: &str) -> Value {
    Text::new(text).into()
}

fn list(items: &[&str]) -> Value {
    let mut list = List::new();
    for item in items {
        list.push(text(item));
    }
    list.into()
}

fn map(entries: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.set(key, value.clone());
    }
    map.into()
}

fn peers(addrs: &[&str], backups: Option<&[&str]>) -> Peers {
    let parse = |items: &[&str]| items.iter().map(|a| a.parse().unwrap()).collect();
    Peers {
        name: "p".into(),
        addrs: parse(addrs),
        backups: backups.map(parse),
    }
}

#[test]
fn a_list_is_written_as_the_texts_of_its_items() {
    let written = peers(&["10.0.0.1", "10.0.0.2"], Some(&["192.168.0.1"]))
        .to_value()
        .unwrap();
    let written = <&Map>::try_from(&written).unwrap();
    let addrs = <&List>::try_from(written.get("addrs").unwrap()).unwrap();
    let texts: Vec<&str> = addrs.iter().map(|v| <&str>::try_from(v).unwrap()).collect();
    assert_eq!(texts, ["10.0.0.1", "10.0.0.2"]);
    assert!(written.get("backups").is_some());

    let none = peers(&[], None).to_value().unwrap();
    let none = <&Map>::try_from(&none).unwrap();
    assert!(none.get("backups").is_none(), "None omits the key");
}

#[test]
fn every_value_round_trips() {
    round_trips(&peers(&[], None));
    round_trips(&peers(&["10.0.0.1"], Some(&[])));
    round_trips(&peers(
        &["10.0.0.1", "10.0.0.2"],
        Some(&["1.1.1.1", "8.8.8.8"]),
    ));
    round_trips(&Route::Static {
        via: vec!["10.1.1.1".parse().unwrap()],
    });
    round_trips(&Route::Dynamic);
}

#[test]
fn an_absent_key_is_the_default_or_none() {
    let read = Peers::from_value(&map(&[("name", text("p"))])).unwrap();
    assert_eq!(read, peers(&[], None));
    let null = Peers::from_value(&map(&[("name", text("p")), ("backups", Value::null())])).unwrap();
    assert_eq!(null.backups, None);
}

#[test]
fn a_refusal_names_the_item() {
    let error = Peers::from_value(&map(&[
        ("name", text("p")),
        ("addrs", list(&["10.0.0.1", "not an address"])),
    ]))
    .expect_err("the second item is not an address");
    let shown = error.to_string();
    assert!(shown.contains("addrs") && shown.contains('1'), "{shown}");

    let error = Peers::from_value(&map(&[("name", text("p")), ("addrs", text("10.0.0.1"))]))
        .expect_err("a text where a list is");
    assert!(error.to_string().contains("addrs"), "{error}");
}

#[test]
fn the_schema_is_a_list_of_strings() {
    let schema = Peers::schema().unwrap();
    let root = SchemaRef::new(&schema).unwrap();
    let addrs = root.find("addrs").unwrap();
    assert!(!addrs.is_required(), "the default makes it optional");
    let Kind::List { items, .. } = addrs.kind() else {
        panic!("a list: {:?}", addrs.kind());
    };
    assert!(matches!(
        guatiao::schema::read::FieldRef::new("item", items)
            .unwrap()
            .kind(),
        Kind::Str { .. }
    ));
    assert!(addrs.default().is_some());

    let backups = root.find("backups").unwrap();
    assert!(!backups.is_required());
    assert!(backups.is_nullable());
    assert!(matches!(backups.kind(), Kind::List { .. }));
}

#[test]
fn what_the_type_writes_validates_against_its_schema() {
    let schema = Peers::schema().unwrap();
    let schema = SchemaRef::new(&schema).unwrap();
    for value in [peers(&[], None), peers(&["10.0.0.1"], Some(&["10.0.0.9"]))] {
        let written = value.to_value().unwrap();
        validate_map(schema, &written).unwrap_or_else(|e| panic!("{value:?}: {e:?}"));
    }
}
