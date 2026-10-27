// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `guatiao_kind_floor_hash`, the floor hash a binding asks for when it
//! describes a kind born in its own language: the shared vectors the
//! derive expands as traits, and the committed documents, hash as they
//! say; what is not a kind is refused where it goes wrong.

use guatiao::exports::kind::guatiao_kind_floor_hash;
use guatiao::value::status::Status;
use guatiao::value::types::Str;
use guatiao::{Alloc, TryAsRef, Value};

const VECTORS: &str = include_str!("../../guatiao-derive/tests/vectors/floor_hash.json");

const DOCUMENTS: [&str; 2] = [
    include_str!("../../../examples/greeter_kind/include/greeter_kind.kinds.json"),
    include_str!("../../../examples/tally_kind/include/tally_kind.kinds.json"),
];

fn hash(text: &str) -> (Status, u32, Value) {
    let (mut out, mut why) = (0u32, Value::absent());
    // SAFETY: the text is readable, the allocator ours, the slots writable.
    let status = unsafe {
        guatiao_kind_floor_hash(Str::new(text), Alloc::rust().as_raw(), &mut out, &mut why)
    };
    (status, out, why)
}

/// Each kind of `{"vectors"|"kinds": [...]}` with its floor hash.
fn kinds(text: &str, key: &str) -> Vec<(String, serde_json::Value)> {
    let doc: serde_json::Value = serde_json::from_str(text).expect("JSON");
    doc[key]
        .as_array()
        .expect("a list")
        .iter()
        .map(|k| (k["name"].as_str().unwrap().to_string(), k.clone()))
        .collect()
}

#[test]
fn every_vector_hashes_as_the_trait_it_pairs_with() {
    let vectors = kinds(VECTORS, "vectors");
    assert!(vectors.len() >= 6);
    for (name, v) in vectors {
        let (status, got, why) = hash(&v.to_string());
        assert_eq!(status, Status::GUATIAO_OK, "{name}: {why:?}");
        assert_eq!(u64::from(got), v["floor_hash"].as_u64().unwrap(), "{name}");
    }
}

#[test]
fn a_committed_document_hashes_as_it_says() {
    for text in DOCUMENTS {
        for (name, kind) in kinds(text, "kinds") {
            let (status, got, why) = hash(&kind.to_string());
            assert_eq!(status, Status::GUATIAO_OK, "{name}: {why:?}");
            assert_eq!(
                u64::from(got),
                kind["floor_hash"].as_u64().unwrap(),
                "{name}"
            );
        }
    }
}

#[test]
fn an_optional_slot_is_hashed_with_the_rest() {
    let kind = |shout: &str| {
        format!(
            r#"{{"object": false, "slots": [
                {{"name": "greet", "required": true, "fallible": true,
                  "args": [{{"name": "name", "type": "str"}}], "answer": {{"type": "map"}}}},
                {{"name": "shout", "required": false, "fallible": false,
                  "args": [{{"name": "name", "type": "{shout}"}}], "answer": {{"type": "text"}}}}]}}"#
        )
    };
    let (_, str_arg, _) = hash(&kind("str"));
    let (_, bytes_arg, _) = hash(&kind("bytes"));
    assert_ne!(str_arg, bytes_arg);
    // `floor_hash` and `name` are not read.
    let (status, again, _) =
        hash(&kind("str").replacen('{', r#"{"floor_hash": 7, "name": "x", "#, 1));
    assert_eq!((status, again), (Status::GUATIAO_OK, str_arg));
}

#[test]
fn what_is_not_a_kind_is_refused_where_it_goes_wrong() {
    for (text, says) in [
        ("{", "`byte 1`"),
        (r#"{"slots": []}"#, "`object` is missing"),
        (
            r#"{"object": false, "slots": [{"name": "x", "required": true, "fallible": false, "args": [{"name": "a", "type": "u256"}]}]}"#,
            "`slots[0].args[0].type`",
        ),
        (
            r#"{"object": true, "slots": [{"name": "destroy", "required": true, "fallible": false, "args": []}]}"#,
            "`slots[0]`",
        ),
    ] {
        let (status, out, why) = hash(text);
        assert_eq!((status, out), (Status::GUATIAO_ERR_BAD_VALUE, 0), "{text}");
        let why: &str = TryAsRef::try_as_ref(&why).expect("why is a string");
        assert!(why.contains(says), "{text}: {why}");
    }
    // `why` may be null; `out` may not.
    let mut out = 0u32;
    // SAFETY: as above, with no slot for why.
    let status = unsafe {
        guatiao_kind_floor_hash(
            Str::new("nope"),
            Alloc::rust().as_raw(),
            &mut out,
            core::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    // SAFETY: a null `out` is refused before anything is read.
    let status = unsafe {
        guatiao_kind_floor_hash(
            Str::new("{}"),
            Alloc::rust().as_raw(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}
