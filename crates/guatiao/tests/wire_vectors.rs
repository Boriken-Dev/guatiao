// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `vectors/wire.json`, through the Rust API and through the C surface:
//! the same vectors every binding runs.

use guatiao::exports::channel::{
    GUATIAO_WIRE_ACCEPTED, GUATIAO_WIRE_INVALID, GUATIAO_WIRE_MALFORMED, GUATIAO_WIRE_NO_SCHEMA,
    GUATIAO_WIRE_NOT_A_SCHEMA, WireEvent, guatiao_wire_announce, guatiao_wire_close,
    guatiao_wire_receiver_accept, guatiao_wire_receiver_free, guatiao_wire_receiver_new,
    guatiao_wire_send,
};
use guatiao::exports::value::{guatiao_value_free, guatiao_wire_decode, guatiao_wire_encode};
use guatiao::value::convert::TryAsRef;
use guatiao::value::status::Status;
use guatiao::value::types::{Bytes, Tag};
use guatiao::value::wire::channel::{self, Received, Receiver};
use guatiao::value::wire::{WireError, decode, to_bytes};
use guatiao::{Alloc, Map, Value};
use serde_json::Value as Json;

#[path = "vectors/read.rs"]
mod vectors;
use vectors::{hex, tree, wire_file};

fn list<'a>(doc: &'a Json, key: &str) -> &'a [Json] {
    doc[key].as_array().expect("a list of vectors")
}

fn bytes_of(v: &Value) -> Vec<u8> {
    TryAsRef::<[u8]>::try_as_ref(v).expect("BYTES").to_vec()
}

fn offset_of(error: &WireError) -> Option<usize> {
    match error {
        WireError::Truncated { at }
        | WireError::UnknownTag { at, .. }
        | WireError::Overlong { at }
        | WireError::NotUtf8 { at }
        | WireError::NotANumber { at }
        | WireError::NotABool { at, .. }
        | WireError::DuplicateKey { at }
        | WireError::Trailing { at }
        | WireError::UnknownFrame { at, .. } => Some(*at),
        _ => None,
    }
}

#[test]
fn every_value_decodes_to_its_tree_and_encodes_to_its_bytes() {
    let doc = wire_file();
    for v in list(&doc, "values") {
        let name = v["name"].as_str().unwrap();
        let bytes = hex(v["hex"].as_str().unwrap());
        let expected = tree(&v["tree"]);
        assert_eq!(to_bytes(&expected).unwrap(), bytes, "{name}: encode");
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back, expected, "{name}: decode");
        assert_eq!(
            to_bytes(&back).unwrap(),
            bytes,
            "{name}: the same bytes again"
        );
    }
}

#[test]
fn every_malformed_input_is_refused_at_its_byte() {
    let doc = wire_file();
    for v in list(&doc, "malformed") {
        let name = v["name"].as_str().unwrap();
        let error = decode(&hex(v["hex"].as_str().unwrap())).unwrap_err();
        assert_eq!(
            offset_of(&error),
            v["at"].as_u64().map(|a| a as usize),
            "{name}: {error}"
        );
    }
}

#[test]
fn the_frame_builders_write_the_documented_bytes() {
    let doc = wire_file();
    for f in list(&doc, "frames") {
        let channel = f["channel"].as_u64().unwrap();
        let built = match f["build"].as_str().unwrap() {
            "announce" => channel::announce(channel, &tree(&f["tree"])).unwrap(),
            "send" => channel::send(channel, &tree(&f["tree"])).unwrap(),
            "close" => channel::close(channel),
            other => panic!("{other}"),
        };
        assert_eq!(built, hex(f["hex"].as_str().unwrap()), "{f}");
    }
}

/// A conversation entry's expected refusal as the C code for it.
fn refusal_code(name: &str) -> u32 {
    match name {
        "malformed" => GUATIAO_WIRE_MALFORMED,
        "no_schema" => GUATIAO_WIRE_NO_SCHEMA,
        "not_a_schema" => GUATIAO_WIRE_NOT_A_SCHEMA,
        "invalid" => GUATIAO_WIRE_INVALID,
        other => panic!("unknown refusal {other}"),
    }
}

#[test]
fn the_conversation_runs_through_one_rust_receiver() {
    let doc = wire_file();
    let mut receiver = Receiver::new();
    for step in list(&doc, "conversation") {
        let frame = hex(step["frame"].as_str().unwrap());
        let channel = step["channel"].as_u64().unwrap_or(0);
        match (receiver.accept(&frame), step["event"].as_str()) {
            (Ok(Received::Schema(c)), Some("schema")) => {
                assert_eq!(c, channel);
                assert_eq!(receiver.schema(c), Some(&tree(&step["tree"])));
            }
            (Ok(Received::Value { channel: c, value }), Some("value")) => {
                assert_eq!((c, value), (channel, tree(&step["tree"])));
            }
            (Ok(Received::Closed(c)), Some("close")) => assert_eq!(c, channel),
            (Err(error), None) => {
                let (code, at) = match &error {
                    WireError::NoSchema { channel: c } => (GUATIAO_WIRE_NO_SCHEMA, *c),
                    WireError::NotASchema { channel: c } => (GUATIAO_WIRE_NOT_A_SCHEMA, *c),
                    WireError::Invalid { channel: c, .. } => (GUATIAO_WIRE_INVALID, *c),
                    other => (GUATIAO_WIRE_MALFORMED, offset_of(other).unwrap() as u64),
                };
                assert_eq!(
                    code,
                    refusal_code(step["refusal"].as_str().unwrap()),
                    "{step}"
                );
                let want = if code == GUATIAO_WIRE_MALFORMED {
                    step["at"].as_u64().unwrap()
                } else {
                    channel
                };
                assert_eq!(at, want, "{step}: {error}");
            }
            (got, _) => panic!("{step}: {got:?}"),
        }
    }
}

/// The C receiver's status for each refusal, from the vector file's table.
fn status_of(doc: &Json, refusal: &str) -> Status {
    match doc["statuses"][refusal].as_str().unwrap() {
        "GUATIAO_ERR_BAD_VALUE" => Status::GUATIAO_ERR_BAD_VALUE,
        "GUATIAO_ERR_NOT_FOUND" => Status::GUATIAO_ERR_NOT_FOUND,
        "GUATIAO_ERR_WRONG_KIND" => Status::GUATIAO_ERR_WRONG_KIND,
        other => panic!("{other}"),
    }
}

fn text_of(v: &Value) -> &str {
    TryAsRef::<str>::try_as_ref(v).expect("a STRING")
}

fn free_event(event: &mut WireEvent) {
    for v in [&mut event.value, &mut event.message, &mut event.detail] {
        // SAFETY: each is a value the event handed over.
        assert_eq!(unsafe { guatiao_value_free(v) }, Status::GUATIAO_OK);
    }
}

#[test]
fn the_conversation_runs_through_the_c_receiver() {
    let doc = wire_file();
    let alloc = Alloc::rust();
    // SAFETY: a valid allocator that outlives the receiver.
    let recv = unsafe { guatiao_wire_receiver_new(alloc.as_raw()) };
    assert!(!recv.is_null());
    for step in list(&doc, "conversation") {
        let frame = hex(step["frame"].as_str().unwrap());
        let channel = step["channel"].as_u64().unwrap_or(0);
        let mut event = core::mem::MaybeUninit::<WireEvent>::uninit();
        // SAFETY: a live receiver, readable bytes, writable storage.
        let status =
            unsafe { guatiao_wire_receiver_accept(recv, Bytes::new(&frame), event.as_mut_ptr()) };
        // SAFETY: written on entry, whatever the status.
        let mut event = unsafe { event.assume_init() };
        assert_eq!(event.channel, channel, "{step}");
        if let Some(kind) = step["kind"].as_u64() {
            assert_eq!(u64::from(event.kind), kind, "{step}");
        }
        match step["event"].as_str() {
            Some(name) => {
                assert_eq!(status, Status::GUATIAO_OK, "{step}");
                assert_eq!(event.refusal, GUATIAO_WIRE_ACCEPTED);
                let kind = match name {
                    "schema" => channel::SCHEMA,
                    "value" => channel::VALUE,
                    _ => channel::CLOSE,
                };
                assert_eq!(event.kind, u32::from(kind), "{step}");
                if step.get("tree").is_some() {
                    assert_eq!(event.value, tree(&step["tree"]), "{step}");
                } else {
                    assert_eq!(event.value.tag(), Ok(Tag::GUATIAO_ABSENT));
                }
                assert_eq!(event.message.tag(), Ok(Tag::GUATIAO_ABSENT));
            }
            None => {
                let refusal = step["refusal"].as_str().unwrap();
                assert_eq!(status, status_of(&doc, refusal), "{step}");
                assert_eq!(event.refusal, refusal_code(refusal), "{step}");
                assert_eq!(event.value.tag(), Ok(Tag::GUATIAO_ABSENT), "{step}");
                assert_eq!(event.at as u64, step["at"].as_u64().unwrap_or(0), "{step}");
                let message = text_of(&event.message);
                assert!(!message.is_empty(), "{step}");
                assert!(
                    !message.contains("hunter2"),
                    "a refusal quotes no value: {message}"
                );
                if let Some(key) = step["key"].as_str() {
                    let detail = TryAsRef::<Map>::try_as_ref(&event.detail).expect("a map");
                    assert_eq!(detail.get("key").map(text_of), Some(key), "{step}");
                    assert!(detail.get("message").is_some());
                } else {
                    assert_eq!(event.detail.tag(), Ok(Tag::GUATIAO_ABSENT), "{step}");
                }
            }
        }
        free_event(&mut event);
    }
    // SAFETY: a live receiver, freed once.
    unsafe { guatiao_wire_receiver_free(recv) };
}

#[test]
fn the_c_frame_builders_write_the_documented_bytes() {
    let doc = wire_file();
    let alloc = Alloc::rust();
    for f in list(&doc, "frames") {
        let channel = f["channel"].as_u64().unwrap();
        let mut out = Value::null();
        let status = match f["build"].as_str().unwrap() {
            // SAFETY: valid pointers; `out` holds nothing owned.
            "announce" => unsafe {
                guatiao_wire_announce(alloc.as_raw(), channel, &tree(&f["tree"]), &mut out)
            },
            // SAFETY: as above.
            "send" => unsafe {
                guatiao_wire_send(alloc.as_raw(), channel, &tree(&f["tree"]), &mut out)
            },
            // SAFETY: as above.
            _ => unsafe { guatiao_wire_close(alloc.as_raw(), channel, &mut out) },
        };
        assert_eq!(status, Status::GUATIAO_OK, "{f}");
        assert_eq!(bytes_of(&out), hex(f["hex"].as_str().unwrap()), "{f}");
    }
}

#[test]
fn every_value_crosses_the_c_wire_functions() {
    let doc = wire_file();
    let alloc = Alloc::rust();
    for v in list(&doc, "values") {
        let bytes = hex(v["hex"].as_str().unwrap());
        let mut out = Value::null();
        // SAFETY: valid pointers; `out` holds nothing owned.
        let status = unsafe { guatiao_wire_encode(alloc.as_raw(), &tree(&v["tree"]), &mut out) };
        assert_eq!(
            (status, bytes_of(&out)),
            (Status::GUATIAO_OK, bytes.clone()),
            "{v}"
        );
        let mut back = Value::null();
        // SAFETY: the view borrows `bytes` for the call.
        let status = unsafe { guatiao_wire_decode(alloc.as_raw(), Bytes::new(&bytes), &mut back) };
        assert_eq!(
            (status, back),
            (Status::GUATIAO_OK, tree(&v["tree"])),
            "{v}"
        );
    }
    for v in list(&doc, "malformed") {
        let bytes = hex(v["hex"].as_str().unwrap());
        let mut out = Value::null();
        // SAFETY: as above.
        let status = unsafe { guatiao_wire_decode(alloc.as_raw(), Bytes::new(&bytes), &mut out) };
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{v}");
        assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT));
    }
}
