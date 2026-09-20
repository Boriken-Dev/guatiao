// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The C surface, called from Rust.
//!
//! `tests/c_consumer.rs` compiles a real C program against the committed
//! header, which is the proof that the surface is usable. It also skips
//! where there is no C compiler. These never skip: they call the exported
//! functions directly, so a flag that is declared and read nowhere — or a
//! status that disagrees with the header — is red on every machine.

#![cfg(feature = "json")]

use guatiao::value::convert::TryAsRef;
use std::ptr;

use guatiao::value::alloc::{Allocator, rust_alloc};
use guatiao::value::status::Status;
use guatiao::value::types::{Map, Value};
use guatiao_serde::exports::{GUATIAO_PRETTY, guatiao_json_emit};

/// A value worth indenting: flat output has no newline in it.
fn a_map() -> Value {
    let mut map = Map::new();
    map.set("port", 5900).unwrap();
    map.set("host", "example.test").unwrap();
    map.into()
}

fn emit(value: &Value, how: u32, alloc: *const Allocator) -> (Status, Option<String>) {
    let mut out = Value::null();
    // SAFETY: `value` is a live tree, `alloc` is what the caller passed,
    // and `out` is a writable value whose previous contents are a null —
    // which owns nothing, so overwriting it frees nothing.
    let status = unsafe { guatiao_json_emit(value, how, alloc, &mut out) };
    let text = TryAsRef::<str>::try_as_ref(&out).map(str::to_string);
    (status, text)
}

/// **`GUATIAO_PRETTY` is read.** It was declared in the header and in the
/// constants for a while without anything acting on it, which a C caller
/// could only find out by looking at the output.
#[test]
fn the_pretty_flag_indents_the_document() {
    let value = a_map();
    let vt = rust_alloc();

    let (status, flat) = emit(&value, 0, &vt);
    assert_eq!(status, Status::GUATIAO_OK);
    let flat = flat.expect("the answer is a string value");
    assert!(!flat.contains('\n'), "the default is compact: {flat}");

    let (status, pretty) = emit(&value, GUATIAO_PRETTY, &vt);
    assert_eq!(status, Status::GUATIAO_OK);
    let pretty = pretty.expect("the answer is a string value");
    assert!(pretty.contains('\n'), "the flag indents: {pretty}");

    // The same document either way, whitespace aside.
    assert_eq!(
        flat.replace(['\n', ' '], ""),
        pretty.replace(['\n', ' '], "")
    );
}

/// An allocator that cannot allocate is `GUATIAO_ERR_ALLOC`, not
/// `GUATIAO_ERR_NULL`: the caller passed something, and what it passed
/// cannot build the answer. The same status in every crate here, so one
/// C caller's error handling covers all of them.
#[test]
fn an_allocator_that_cannot_allocate_says_so() {
    let value = a_map();

    let (status, text) = emit(&value, 0, ptr::null());
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert_eq!(text, None);

    // Present, and missing the functions an allocator is.
    let empty = Allocator {
        struct_size: size_of::<Allocator>() as u32,
        ctx: ptr::null_mut(),
        alloc: None,
        free: None,
        release: None,
    };
    let (status, text) = emit(&value, 0, &empty);
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert_eq!(text, None);
}

/// A null `out` is the caller's mistake, and the one status that is still
/// about a pointer.
#[test]
fn a_null_out_is_refused_before_anything_is_built() {
    let value = a_map();
    let vt = rust_alloc();
    // SAFETY: `value` is a live tree and `vt` a complete allocator; the
    // null `out` is what this asserts about.
    let status = unsafe { guatiao_json_emit(&value, 0, &vt, ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);

    // And a null value, likewise.
    let mut out = Value::null();
    // SAFETY: as above, with `out` writable and holding a null.
    let status = unsafe { guatiao_json_emit(ptr::null(), 0, &vt, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}
