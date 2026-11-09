// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A value as one line of text: the shared vectors, what stands in for what
//! JSON cannot say, the bound on depth, and the C function.

use guatiao::exports::value::guatiao_value_display;
use guatiao::value::MAX_DEPTH;
use guatiao::value::convert::TryAsRef;
use guatiao::value::types::Payload;
use guatiao::{Alloc, List, Map, Tag, Text, Value};

#[path = "vectors/read.rs"]
mod vectors;

fn doc() -> serde_json::Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/display.json");
    let text = std::fs::read_to_string(path).expect("the vector file is readable");
    serde_json::from_str(&text).expect("the vector file is JSON")
}

#[test]
fn every_vector_prints_exactly_its_line() {
    let doc = doc();
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 20);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let value = vectors::tree(&case["tree"]);
        assert_eq!(value.to_string(), case["text"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn the_c_function_writes_the_same_line_as_a_string_value() {
    for case in doc()["cases"].as_array().unwrap() {
        let value = vectors::tree(&case["tree"]);
        let alloc = Alloc::rust();
        let mut out = Value::null();
        // SAFETY: a well-formed value, an allocator and writable storage.
        let status = unsafe { guatiao_value_display(&value, alloc.as_raw(), &mut out) };
        assert_eq!(status, guatiao::value::Status::GUATIAO_OK);
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&out),
            case["text"].as_str(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn the_c_function_refuses_what_it_cannot_use() {
    let value = Value::null();
    let mut out = Value::null();
    // SAFETY: null pointers are what is being tried; `out` is writable.
    unsafe {
        assert_eq!(
            guatiao_value_display(core::ptr::null(), Alloc::rust().as_raw(), &mut out),
            guatiao::value::Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_value_display(&value, core::ptr::null(), &mut out),
            guatiao::value::Status::GUATIAO_ERR_ALLOC
        );
        assert_eq!(
            guatiao_value_display(&value, Alloc::rust().as_raw(), core::ptr::null_mut()),
            guatiao::value::Status::GUATIAO_ERR_NULL
        );
    }
    assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT));
}

#[test]
fn a_node_in_the_wiping_allocator_is_not_shown() {
    let w = Alloc::wiping();
    let leaf = Value::from(Text::new_in(w, "hunter2").unwrap());
    assert_eq!(leaf.to_string(), "[sensitive]");

    let mut map = Map::new_in(w);
    map.set_in("password", Text::new_in(w, "hunter2").unwrap(), w)
        .unwrap();
    assert_eq!(Value::from(map).to_string(), "[sensitive]");

    let mut outer = Map::new();
    outer.set("user", Text::new("alice"));
    outer.set("password", Value::from(Text::new_in(w, "hunter2").unwrap()));
    let shown = Value::from(outer).to_string();
    assert_eq!(shown, r#"{"user":"alice","password":[sensitive]}"#);

    // A key of its own in the wiping allocator, in a map that is not.
    let mut keyed = Map::new();
    keyed.set_in("hunter2", Text::new("x"), w).unwrap();
    let shown = Value::from(keyed).to_string();
    assert_eq!(shown, r#"{[sensitive]:"x"}"#);
}

#[test]
fn an_unknown_tag_and_text_that_is_not_utf8_never_fault() {
    // SAFETY: an unknown tag over a payload that owns nothing.
    let unknown = unsafe { Value::from_raw_parts(99, Payload::bool(false)) };
    assert_eq!(unknown.to_string(), "<unknown tag 99>");

    static BAD: [u8; 2] = [0xff, 0xfe];
    // SAFETY: the shape is right (a borrowed, readable buffer); only the
    // content is wrong.
    let bad = unsafe {
        Value::from_raw_parts(
            u32::from(Tag::GUATIAO_STRING),
            Payload::text(Text::from_raw_parts(
                BAD.as_ptr().cast_mut(),
                2,
                0,
                core::ptr::null(),
            )),
        )
    };
    let mut list = List::new();
    list.push(unknown);
    list.push(bad);
    assert_eq!(
        Value::from(list).to_string(),
        r#"[<unknown tag 99>,<not utf-8>]"#
    );
}

#[test]
fn a_tree_deeper_than_the_bound_costs_a_bounded_line() {
    let mut v = Value::null();
    for _ in 0..MAX_DEPTH * 4 {
        let mut list = List::new();
        list.push(v);
        v = list.into();
    }
    let shown = v.to_string();
    let expected = format!(
        "{}...{}",
        "[".repeat(MAX_DEPTH as usize),
        "]".repeat(MAX_DEPTH as usize)
    );
    assert_eq!(shown, expected);
}

#[test]
fn a_tree_of_maps_deeper_than_the_bound_costs_a_bounded_line() {
    let mut v = Value::null();
    for _ in 0..MAX_DEPTH * 4 {
        let mut map = Map::new();
        map.set("k", v);
        v = map.into();
    }
    let expected = format!(
        "{}...{}",
        "{\"k\":".repeat(MAX_DEPTH as usize),
        "}".repeat(MAX_DEPTH as usize)
    );
    assert_eq!(v.to_string(), expected);
}
