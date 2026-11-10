// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The schema made from a description is the schema guatiao's own derive writes for the same
//! declaration, key for key, wherever the two are meant to agree. A duration and an untagged
//! enum differ by design and are pinned in `schema_pins`.

mod support;

use coqui_builder::Describe;
use guatiao::{Alloc, Schema, Value};
use guatiao_builder::{kind_in, schema};
use support::*;

fn kind_of_description<T: Describe>() -> Value {
    kind_in(&T::description(), Alloc::rust())
        .finish()
        .expect("a kind")
}

fn kind_of_derive<T: Schema>() -> Value {
    T::kind(Alloc::rust()).finish().expect("a kind")
}

fn same_kind<T: Describe + Schema>(what: &str) {
    let (made, derived) = (kind_of_description::<T>(), kind_of_derive::<T>());
    assert!(
        made == derived,
        "{what} (description | derive):\n{}",
        diff(&made, &derived).join("\n")
    );
}

#[test]
fn a_struct_of_every_shared_shape() {
    same_kind::<Everything>("Everything");
}

#[test]
fn the_whole_document_is_the_derives_too() {
    let made = schema(&Everything::description()).unwrap();
    let derived = Everything::schema().unwrap();
    assert!(
        made == derived,
        "(description | derive):\n{}",
        diff(&made, &derived).join("\n")
    );
}

#[test]
fn each_part_alone() {
    same_kind::<Waiting>("Waiting");
    same_kind::<Common>("Common");
    same_kind::<Credential>("Credential");
    same_kind::<Level>("Level");
    same_kind::<Codec>("Codec");
    same_kind::<Name>("Name");
}

#[derive(Describe, guatiao::ToValue, guatiao::FromValue, Schema)]
#[map(deny_unknown_fields)]
struct Closed {
    a: u8,
}

/// guatiao's derive seals every object it describes, with or without
/// `deny_unknown_fields` (that only makes its reader refuse a stranger), so the schema made
/// from a description seals every group too and refuses no key the derive's accepts.
#[test]
fn every_group_is_sealed_as_the_derive_seals_it() {
    fn sealed(value: &Value) -> bool {
        let map = <&guatiao::Map>::try_from(value).unwrap();
        map.get("additionalProperties") == Some(&Value::from(false))
    }
    assert!(sealed(&kind_of_derive::<Waiting>()));
    assert!(sealed(&kind_of_description::<Waiting>()));
    assert!(sealed(&kind_of_derive::<Closed>()));
    assert!(sealed(&kind_of_description::<Closed>()));
    same_kind::<Closed>("Closed");
}
