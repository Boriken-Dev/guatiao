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

use core::ptr;
use guatiao::value::convert::TryAsRef;

use guatiao::value::alloc::{Allocator, rust_alloc};
use guatiao::value::status::Status;
use guatiao::value::types::{Map, Str, Tag, Value};
use guatiao_serde::exports::{
    GUATIAO_FORMAT_JSON, GUATIAO_PRETTY, ParseError, guatiao_json_emit, guatiao_json_parse,
    guatiao_json_parse_ex,
};

/// A value worth indenting: flat output has no newline in it.
fn a_map() -> Value {
    let mut map = Map::new();
    map.set("port", 5900);
    map.set("host", "example.test");
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

/// **A failed call leaves `out` ABSENT**, as the other two crates' exports
/// do, whatever it held: the first statement of every export writes it.
#[test]
fn a_failed_call_leaves_out_absent() {
    let vt = rust_alloc();

    let mut out = Value::null();
    // SAFETY: `out` is writable and holds a null, which owns nothing.
    let status = unsafe { guatiao_json_parse(Str::new("{ nope"), 0, &vt, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT), "a refused document");

    // The allocator is refused before the document is read.
    let mut out = Value::null();
    // SAFETY: as above.
    let status = unsafe { guatiao_json_parse(Str::new("{}"), 0, ptr::null(), &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_ALLOC);
    assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT), "a bad allocator");

    let mut out = Value::null();
    // SAFETY: `out` writable; the null value is what this asserts about.
    let status = unsafe { guatiao_json_emit(ptr::null(), 0, &vt, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
    assert_eq!(out.tag(), Ok(Tag::GUATIAO_ABSENT), "a null value");
}

/// `_parse_ex` says where a document stopped, and leaves the error empty
/// when nothing went wrong.
#[test]
fn parse_ex_reports_where_a_document_stopped() {
    let vt = rust_alloc();
    let mut out = Value::null();
    let mut error = ParseError {
        format: 9,
        line: 9,
        column: 9,
        message: Value::null(),
    };

    // SAFETY: both pointers address writable storage owning nothing.
    let status =
        unsafe { guatiao_json_parse_ex(Str::new("{\"a\": 1 x}"), 0, &vt, &mut out, &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(error.format, GUATIAO_FORMAT_JSON);
    assert_eq!((error.line, error.column), (1, 9));
    let message = TryAsRef::<str>::try_as_ref(&error.message).expect("a message");
    assert!(!message.is_empty());
    assert!(!message.contains("1 x"), "no excerpt: {message}");

    // A secret on the line the parser objects to is not in the message.
    let mut secret = ParseError {
        format: 9,
        line: 9,
        column: 9,
        message: Value::null(),
    };
    // SAFETY: as above.
    let status = unsafe {
        guatiao_json_parse_ex(
            Str::new("{\"password\": \"hunter2-SECRET\" x}"),
            0,
            &vt,
            &mut out,
            &mut secret,
        )
    };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    let message = TryAsRef::<str>::try_as_ref(&secret.message).expect("a message");
    assert!(!message.contains("hunter2"), "{message}");

    // A document that parses reports nothing.
    let mut error = ParseError {
        format: 9,
        line: 9,
        column: 9,
        message: Value::null(),
    };
    // SAFETY: as above.
    let status = unsafe { guatiao_json_parse_ex(Str::new("{}"), 0, &vt, &mut out, &mut error) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!((error.format, error.line, error.column), (0, 0, 0));
    assert_eq!(error.message.tag(), Ok(Tag::GUATIAO_ABSENT));

    // And a null `out_error` is the plain call.
    let mut again = Value::null();
    // SAFETY: as above.
    let status =
        unsafe { guatiao_json_parse_ex(Str::new("{ nope"), 0, &vt, &mut again, ptr::null_mut()) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
}

/// `_parse_ex` fills what it can for every failure: the format of the
/// call whatever went wrong, and for text that is not UTF-8, where it
/// stops being UTF-8 and a message saying so.
#[test]
fn parse_ex_names_the_format_and_the_place_for_every_failure() {
    let vt = rust_alloc();
    let blank = || ParseError {
        format: 9,
        line: 9,
        column: 9,
        message: Value::null(),
    };
    // `"é": ` and then a byte no UTF-8 text holds, on the second line.
    let bytes = b"{\n\"\xc3\xa9\": \xff}";
    // SAFETY: the bytes outlive every call below.
    let text = || unsafe { Str::from_raw_parts(bytes.as_ptr(), bytes.len()) };

    let mut out = Value::null();
    let mut error = blank();
    // SAFETY: both pointers address writable storage owning nothing.
    let status = unsafe { guatiao_json_parse_ex(text(), 0, &vt, &mut out, &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(
        (error.format, error.line, error.column),
        (GUATIAO_FORMAT_JSON, 2, 6),
        "the column counts characters up to the first byte that is not UTF-8"
    );
    let message = TryAsRef::<str>::try_as_ref(&error.message).expect("a message");
    assert!(message.contains("UTF-8"), "{message}");

    #[cfg(feature = "toml")]
    {
        use guatiao_serde::exports::{GUATIAO_FORMAT_TOML, guatiao_toml_parse_ex};
        let mut error = blank();
        // SAFETY: a null `out` is refused before anything is written there.
        let status = unsafe {
            guatiao_toml_parse_ex(Str::new("a = 1"), 0, &vt, ptr::null_mut(), &mut error)
        };
        assert_eq!(status, Status::GUATIAO_ERR_NULL);
        assert_eq!(
            (error.format, error.line, error.column),
            (GUATIAO_FORMAT_TOML, 0, 0),
            "a TOML call does not read as JSON when nothing was parsed"
        );
        assert_eq!(error.message.tag(), Ok(Tag::GUATIAO_ABSENT));
    }
    #[cfg(feature = "yaml")]
    {
        use guatiao_serde::exports::{GUATIAO_FORMAT_YAML, guatiao_yaml_parse_ex};
        let mut error = blank();
        // SAFETY: as above.
        let status = unsafe { guatiao_yaml_parse_ex(text(), 0, &vt, &mut out, &mut error) };
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
        assert_eq!(
            (error.format, error.line, error.column),
            (GUATIAO_FORMAT_YAML, 2, 6)
        );
    }
}
