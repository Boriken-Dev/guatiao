// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The documents-as-events functions from C, over every shared wire
//! vector and in every format: wire bytes written as a document give the
//! text the tree path gives; that text read back as wire bytes gives the
//! bytes the tree path gives; a document's events into a C visitor are
//! the events of those bytes; and a C walker replaying a value's events
//! writes that same text.

#![cfg(all(feature = "json", feature = "toml", feature = "yaml"))]

extern crate alloc;

use alloc::string::String;
use core::ffi::c_void;

use guatiao::exports::value::{guatiao_wire_decode, guatiao_wire_encode};
use guatiao::exports::visit::{VisitorTable, WalkerTable, guatiao_value_visit, guatiao_wire_visit};
use guatiao::value::convert::TryAsRef;
use guatiao::value::status::Status;
use guatiao::value::types::{Bytes, Str};
use guatiao::{Alloc, Value};
use guatiao_serde::exports::*;

type Parse =
    unsafe extern "C" fn(Str, u32, *const guatiao::value::alloc::Allocator, *mut Value) -> Status;
type Emit = unsafe extern "C" fn(
    *const Value,
    u32,
    *const guatiao::value::alloc::Allocator,
    *mut Value,
) -> Status;
type ToWire = unsafe extern "C" fn(
    Str,
    u32,
    *const guatiao::value::alloc::Allocator,
    *mut Value,
    *mut ParseError,
) -> Status;
type FromWire =
    unsafe extern "C" fn(Bytes, u32, *const guatiao::value::alloc::Allocator, *mut Value) -> Status;
type Visit = unsafe extern "C" fn(
    Str,
    u32,
    *const guatiao::value::alloc::Allocator,
    *const VisitorTable,
    *mut ParseError,
) -> Status;
type EmitWalker = unsafe extern "C" fn(
    *const WalkerTable,
    u32,
    *const guatiao::value::alloc::Allocator,
    *mut Value,
) -> Status;

struct Format {
    name: &'static str,
    parse: Parse,
    emit: Emit,
    to_wire: ToWire,
    from_wire: FromWire,
    visit: Visit,
    emit_walker: EmitWalker,
}

const FORMATS: [Format; 3] = [
    Format {
        name: "json",
        parse: guatiao_json_parse,
        emit: guatiao_json_emit,
        to_wire: guatiao_json_to_wire,
        from_wire: guatiao_json_from_wire,
        visit: guatiao_json_visit,
        emit_walker: guatiao_json_emit_walker,
    },
    Format {
        name: "toml",
        parse: guatiao_toml_parse,
        emit: guatiao_toml_emit,
        to_wire: guatiao_toml_to_wire,
        from_wire: guatiao_toml_from_wire,
        visit: guatiao_toml_visit,
        emit_walker: guatiao_toml_emit_walker,
    },
    Format {
        name: "yaml",
        parse: guatiao_yaml_parse,
        emit: guatiao_yaml_emit,
        to_wire: guatiao_yaml_to_wire,
        from_wire: guatiao_yaml_from_wire,
        visit: guatiao_yaml_visit,
        emit_walker: guatiao_yaml_emit_walker,
    },
];

// --- a recording visitor and a replaying walker, as C writes them ---------------

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Absent,
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Bytes(Vec<u8>),
    ListBegin,
    ListEnd,
    MapBegin,
    Key(String),
    MapEnd,
}

/// # Safety
///
/// `ctx` is a `Vec<Event>` alive for the call.
unsafe fn push(ctx: *mut c_void, e: Event) -> Status {
    // SAFETY: the caller's contract.
    unsafe { &mut *ctx.cast::<Vec<Event>>() }.push(e);
    Status::GUATIAO_OK
}

fn text(view: Str<'_>) -> String {
    String::from(core::str::from_utf8(view.into()).expect("UTF-8"))
}

unsafe extern "C" fn on_absent(ctx: *mut c_void) -> Status {
    // SAFETY: the table's `ctx` is a live `Vec<Event>`.
    unsafe { push(ctx, Event::Absent) }
}
unsafe extern "C" fn on_null(ctx: *mut c_void) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::Null) }
}
unsafe extern "C" fn on_bool(ctx: *mut c_void, v: bool) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::Bool(v)) }
}
unsafe extern "C" fn on_number(ctx: *mut c_void, t: Str<'_>) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::Number(text(t))) }
}
unsafe extern "C" fn on_string(ctx: *mut c_void, t: Str<'_>) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::String(text(t))) }
}
unsafe extern "C" fn on_bytes(ctx: *mut c_void, b: Bytes<'_>) -> Status {
    let b: &[u8] = b.into();
    // SAFETY: as above.
    unsafe { push(ctx, Event::Bytes(b.to_vec())) }
}
// Lengths are left out of what is compared: a format's reader knows none.
unsafe extern "C" fn on_list_begin(ctx: *mut c_void, _: u64) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::ListBegin) }
}
unsafe extern "C" fn on_list_end(ctx: *mut c_void) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::ListEnd) }
}
unsafe extern "C" fn on_map_begin(ctx: *mut c_void, _: u64) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::MapBegin) }
}
unsafe extern "C" fn on_key(ctx: *mut c_void, t: Str<'_>) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::Key(text(t))) }
}
unsafe extern "C" fn on_map_end(ctx: *mut c_void) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::MapEnd) }
}

fn recorder(events: &mut Vec<Event>) -> VisitorTable {
    VisitorTable {
        struct_size: size_of::<VisitorTable>() as u32,
        ctx: (events as *mut Vec<Event>).cast(),
        absent: Some(on_absent),
        null: Some(on_null),
        boolean: Some(on_bool),
        number: Some(on_number),
        string: Some(on_string),
        bytes: Some(on_bytes),
        list_begin: Some(on_list_begin),
        list_end: Some(on_list_end),
        map_begin: Some(on_map_begin),
        key: Some(on_key),
        map_end: Some(on_map_end),
    }
}

/// Replays the events behind `ctx` into `into`, every length unknown.
///
/// # Safety
///
/// `ctx` is a `Vec<Event>` and `into` a table, both alive for the call.
unsafe extern "C" fn replay(ctx: *mut c_void, into: *const VisitorTable) -> Status {
    // SAFETY: the caller's contract.
    let (events, into) = unsafe { (&*ctx.cast::<Vec<Event>>(), &*into) };
    let unknown = guatiao::exports::visit::GUATIAO_LEN_UNKNOWN;
    for e in events {
        // SAFETY: the library's own table and `ctx`; views outlive calls.
        let s = unsafe {
            match e {
                Event::Absent => into.absent.unwrap()(into.ctx),
                Event::Null => into.null.unwrap()(into.ctx),
                Event::Bool(b) => into.boolean.unwrap()(into.ctx, *b),
                Event::Number(t) => into.number.unwrap()(into.ctx, Str::new(t)),
                Event::String(t) => into.string.unwrap()(into.ctx, Str::new(t)),
                Event::Bytes(b) => into.bytes.unwrap()(into.ctx, Bytes::new(b)),
                Event::ListBegin => into.list_begin.unwrap()(into.ctx, unknown),
                Event::ListEnd => into.list_end.unwrap()(into.ctx),
                Event::MapBegin => into.map_begin.unwrap()(into.ctx, unknown),
                Event::Key(t) => into.key.unwrap()(into.ctx, Str::new(t)),
                Event::MapEnd => into.map_end.unwrap()(into.ctx),
            }
        };
        if !s.is_ok() {
            return s;
        }
    }
    Status::GUATIAO_OK
}

// --- helpers --------------------------------------------------------------------

fn alloc() -> Alloc {
    Alloc::rust()
}

/// A STRING value's text, or the status.
fn string_of(status: Status, out: &Value) -> Result<String, Status> {
    if !status.is_ok() {
        return Err(status);
    }
    Ok(String::from(
        TryAsRef::<str>::try_as_ref(out).expect("a STRING answer"),
    ))
}

fn bytes_of(status: Status, out: &Value) -> Result<Vec<u8>, Status> {
    if !status.is_ok() {
        return Err(status);
    }
    Ok(TryAsRef::<[u8]>::try_as_ref(out)
        .expect("a BYTES answer")
        .to_vec())
}

fn wire_vectors() -> Vec<(String, Vec<u8>)> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../guatiao/tests/vectors/wire.json"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("readable")).expect("JSON");
    doc["values"]
        .as_array()
        .expect("values")
        .iter()
        .map(|v| {
            let hex = v["hex"].as_str().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            (v["name"].as_str().unwrap().to_string(), bytes)
        })
        .collect()
}

#[test]
fn every_vector_crosses_every_document_export_in_every_format() {
    let a = alloc();
    let mut compared = 0;
    for (name, bytes) in wire_vectors() {
        let mut value = Value::absent();
        // SAFETY: readable bytes and a writable slot.
        let s = unsafe { guatiao_wire_decode(a.as_raw(), Bytes::new(&bytes), &mut value) };
        assert!(s.is_ok(), "{name}: decode");

        for f in &FORMATS {
            let what = format!("{name} as {}", f.name);

            // Wire bytes to text: what the tree path writes, or its refusal.
            let mut tree_text = Value::absent();
            // SAFETY: a live value, allocator and slot.
            let s = unsafe { (f.emit)(&value, 0, a.as_raw(), &mut tree_text) };
            let expected = string_of(s, &tree_text);
            let mut streamed = Value::absent();
            // SAFETY: readable bytes, a live allocator and slot.
            let s = unsafe { (f.from_wire)(Bytes::new(&bytes), 0, a.as_raw(), &mut streamed) };
            assert_eq!(string_of(s, &streamed), expected, "{what}: from_wire");

            // A C walker replaying the value's events writes the same text.
            let mut events = Vec::new();
            let table = recorder(&mut events);
            // SAFETY: a live value and table.
            assert!(unsafe { guatiao_value_visit(&value, &table) }.is_ok());
            let walker = WalkerTable {
                struct_size: size_of::<WalkerTable>() as u32,
                ctx: (&mut events as *mut Vec<Event>).cast(),
                walk: Some(replay),
            };
            let mut walked = Value::absent();
            // SAFETY: a live walker, allocator and slot.
            let s = unsafe { (f.emit_walker)(&walker, 0, a.as_raw(), &mut walked) };
            assert_eq!(string_of(s, &walked), expected, "{what}: emit_walker");

            let Ok(text) = expected else { continue };
            compared += 1;

            // That text read as wire bytes: what the tree path encodes.
            let mut parsed = Value::absent();
            // SAFETY: a readable view, a live allocator and slot.
            let s = unsafe { (f.parse)(Str::new(&text), 0, a.as_raw(), &mut parsed) };
            assert!(s.is_ok(), "{what}: parse");
            let mut tree_bytes = Value::absent();
            // SAFETY: a live value, allocator and slot.
            let s = unsafe { guatiao_wire_encode(a.as_raw(), &parsed, &mut tree_bytes) };
            let tree_bytes = bytes_of(s, &tree_bytes).expect("encodes");
            let mut tape = Value::absent();
            // SAFETY: a readable view, a live allocator and slot.
            let s = unsafe {
                (f.to_wire)(
                    Str::new(&text),
                    0,
                    a.as_raw(),
                    &mut tape,
                    core::ptr::null_mut(),
                )
            };
            assert_eq!(
                bytes_of(s, &tape),
                Ok(tree_bytes.clone()),
                "{what}: to_wire"
            );

            // The document's events into a C visitor are those bytes' events.
            let mut from_text = Vec::new();
            let table = recorder(&mut from_text);
            // SAFETY: a readable view, a live allocator and table.
            let s = unsafe {
                (f.visit)(
                    Str::new(&text),
                    0,
                    a.as_raw(),
                    &table,
                    core::ptr::null_mut(),
                )
            };
            assert!(s.is_ok(), "{what}: visit");
            let mut from_bytes = Vec::new();
            let table = recorder(&mut from_bytes);
            // SAFETY: readable bytes and a live table.
            let s = unsafe { guatiao_wire_visit(Bytes::new(&tree_bytes), &table) };
            assert!(s.is_ok(), "{what}: wire_visit");
            assert_eq!(from_text, from_bytes, "{what}: the same events");
        }
    }
    assert!(compared > 20, "only {compared} vectors had a spelling");
}

#[test]
fn a_document_that_does_not_parse_says_where_through_every_new_export() {
    let a = alloc();
    let mut events = Vec::new();
    let table = recorder(&mut events);
    let mut error = core::mem::MaybeUninit::<ParseError>::uninit();
    // SAFETY: a readable view, a live allocator, table and error slot.
    let s = unsafe {
        guatiao_json_visit(
            Str::new("{\n  \"a\": }"),
            0,
            a.as_raw(),
            &table,
            error.as_mut_ptr(),
        )
    };
    assert_eq!(s, Status::GUATIAO_ERR_BAD_VALUE);
    // SAFETY: written by the call.
    let mut error = unsafe { error.assume_init() };
    assert_eq!((error.format, error.line), (GUATIAO_FORMAT_JSON, 2));
    // SAFETY: the message the call built.
    unsafe { guatiao::exports::value::guatiao_value_free(&mut error.message) };

    let mut out = Value::absent();
    let mut error = core::mem::MaybeUninit::<ParseError>::uninit();
    // SAFETY: as above.
    let s = unsafe {
        guatiao_toml_to_wire(
            Str::new("a = = 1"),
            0,
            a.as_raw(),
            &mut out,
            error.as_mut_ptr(),
        )
    };
    assert_eq!(s, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(out.tag(), Ok(guatiao::Tag::GUATIAO_ABSENT));
    // SAFETY: written by the call.
    let mut error = unsafe { error.assume_init() };
    assert_eq!(error.format, GUATIAO_FORMAT_TOML);
    // SAFETY: the message the call built.
    unsafe { guatiao::exports::value::guatiao_value_free(&mut error.message) };

    // Bytes that are not a value are refused, `out` left absent.
    let mut out = Value::absent();
    // SAFETY: readable bytes, a live allocator and slot.
    let s = unsafe { guatiao_yaml_from_wire(Bytes::new(b"\x07\x05"), 0, a.as_raw(), &mut out) };
    assert_eq!(s, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(out.tag(), Ok(guatiao::Tag::GUATIAO_ABSENT));
}
