// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The visitor from C, through the `extern "C"` functions with callbacks
//! written the way C writes them: every shared wire vector walks into a
//! C visitor the same from a value and from its bytes, and a C walker
//! replaying those events builds the same value and writes the same
//! bytes. A callback's status stops a walk; a malformed table, walker or
//! byte string is refused.

extern crate alloc;

use alloc::string::String;
use core::ffi::c_void;

use guatiao::exports::visit::{
    GUATIAO_LEN_UNKNOWN, VisitorTable, WalkerTable, guatiao_value_from_walker, guatiao_value_visit,
    guatiao_wire_encode_walker, guatiao_wire_visit,
};
use guatiao::value::convert::TryAsRef;
use guatiao::value::status::Status;
use guatiao::value::types::{Bytes, Str};
use guatiao::value::wire::{decode, to_bytes};
use guatiao::{Alloc, Value};
use serde_json::Value as Json;

#[path = "vectors/read.rs"]
mod vectors;
use vectors::{hex, wire_file};

/// One event, as a C visitor saw it.
#[derive(Debug, Clone, PartialEq)]
enum Event {
    Absent,
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Bytes(Vec<u8>),
    ListBegin(u64),
    ListEnd,
    MapBegin(u64),
    Key(String),
    MapEnd,
}

/// What a recording visitor's `ctx` points at: the events, and the
/// event after which to answer a status instead.
struct Record {
    events: Vec<Event>,
    stop_at: Option<(usize, Status)>,
}

/// # Safety
///
/// `ctx` is a `Record` alive for the call.
unsafe fn push(ctx: *mut c_void, event: Event) -> Status {
    // SAFETY: the caller's contract.
    let record = unsafe { &mut *ctx.cast::<Record>() };
    record.events.push(event);
    match record.stop_at {
        Some((n, status)) if record.events.len() == n => status,
        _ => Status::GUATIAO_OK,
    }
}

fn text(view: Str<'_>) -> String {
    String::from(core::str::from_utf8(view.into()).expect("UTF-8"))
}

unsafe extern "C" fn on_absent(ctx: *mut c_void) -> Status {
    // SAFETY: the table's `ctx` is a live `Record`.
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
unsafe extern "C" fn on_list_begin(ctx: *mut c_void, n: u64) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::ListBegin(n)) }
}
unsafe extern "C" fn on_list_end(ctx: *mut c_void) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::ListEnd) }
}
unsafe extern "C" fn on_map_begin(ctx: *mut c_void, n: u64) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::MapBegin(n)) }
}
unsafe extern "C" fn on_key(ctx: *mut c_void, t: Str<'_>) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::Key(text(t))) }
}
unsafe extern "C" fn on_map_end(ctx: *mut c_void) -> Status {
    // SAFETY: as above.
    unsafe { push(ctx, Event::MapEnd) }
}

fn visitor(record: &mut Record) -> VisitorTable {
    VisitorTable {
        struct_size: size_of::<VisitorTable>() as u32,
        ctx: (record as *mut Record).cast(),
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

/// The events of `walk` into a fresh recording visitor, and its status.
fn record(walk: impl FnOnce(&VisitorTable) -> Status) -> (Status, Vec<Event>) {
    let mut record = Record {
        events: Vec::new(),
        stop_at: None,
    };
    let table = visitor(&mut record);
    let status = walk(&table);
    (status, record.events)
}

/// A C walker replaying recorded events into the visitor it is given.
///
/// # Safety
///
/// `ctx` is a `Vec<Event>` and `into` a table, both alive for the call.
unsafe extern "C" fn replay(ctx: *mut c_void, into: *const VisitorTable) -> Status {
    // SAFETY: the caller's contract.
    let (events, into) = unsafe { (&*ctx.cast::<Vec<Event>>(), &*into) };
    for event in events {
        // SAFETY: the library's own table, called with its own `ctx`; the
        // views outlive each call.
        let status = unsafe {
            match event {
                Event::Absent => into.absent.unwrap()(into.ctx),
                Event::Null => into.null.unwrap()(into.ctx),
                Event::Bool(b) => into.boolean.unwrap()(into.ctx, *b),
                Event::Number(t) => into.number.unwrap()(into.ctx, Str::new(t)),
                Event::String(t) => into.string.unwrap()(into.ctx, Str::new(t)),
                Event::Bytes(b) => into.bytes.unwrap()(into.ctx, Bytes::new(b)),
                Event::ListBegin(n) => into.list_begin.unwrap()(into.ctx, *n),
                Event::ListEnd => into.list_end.unwrap()(into.ctx),
                Event::MapBegin(n) => into.map_begin.unwrap()(into.ctx, *n),
                Event::Key(t) => into.key.unwrap()(into.ctx, Str::new(t)),
                Event::MapEnd => into.map_end.unwrap()(into.ctx),
            }
        };
        if !status.is_ok() {
            return status;
        }
    }
    Status::GUATIAO_OK
}

fn walker(events: &Vec<Event>) -> WalkerTable {
    WalkerTable {
        struct_size: size_of::<WalkerTable>() as u32,
        ctx: (events as *const Vec<Event>).cast_mut().cast(),
        walk: Some(replay),
    }
}

fn built(events: &Vec<Event>) -> (Status, Value) {
    let alloc = Alloc::rust();
    let mut out = Value::absent();
    let table = walker(events);
    // SAFETY: a live walker, allocator and out-slot.
    let status = unsafe { guatiao_value_from_walker(alloc.as_raw(), &table, &mut out) };
    (status, out)
}

fn encoded(events: &Vec<Event>) -> (Status, Vec<u8>) {
    let alloc = Alloc::rust();
    let mut out = Value::absent();
    let table = walker(events);
    // SAFETY: a live walker, allocator and out-slot.
    let status = unsafe { guatiao_wire_encode_walker(alloc.as_raw(), &table, &mut out) };
    let bytes = TryAsRef::<[u8]>::try_as_ref(&out)
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    (status, bytes)
}

fn list<'a>(doc: &'a Json, key: &str) -> &'a [Json] {
    doc[key].as_array().expect("a list of vectors")
}

#[test]
fn every_vector_crosses_every_visitor_export() {
    let doc = wire_file();
    for v in list(&doc, "values") {
        let name = v["name"].as_str().unwrap();
        let bytes = hex(v["hex"].as_str().unwrap());
        let value = decode(&bytes).unwrap();

        // SAFETY: a live value and table.
        let (status, from_value) = record(|t| unsafe { guatiao_value_visit(&value, t) });
        assert_eq!(status, Status::GUATIAO_OK, "{name}: value_visit");
        // SAFETY: readable bytes and a live table.
        let (status, from_bytes) = record(|t| unsafe { guatiao_wire_visit(Bytes::new(&bytes), t) });
        assert_eq!(status, Status::GUATIAO_OK, "{name}: wire_visit");
        assert_eq!(from_bytes, from_value, "{name}: the same events");

        assert_eq!(
            built(&from_value),
            (Status::GUATIAO_OK, value.clone()),
            "{name}: built"
        );
        assert_eq!(
            encoded(&from_value),
            (Status::GUATIAO_OK, bytes.clone()),
            "{name}: encoded"
        );

        // The same with no lengths given: the encoder counts them itself.
        let unknown: Vec<Event> = from_value
            .iter()
            .map(|e| match e {
                Event::ListBegin(_) => Event::ListBegin(GUATIAO_LEN_UNKNOWN),
                Event::MapBegin(_) => Event::MapBegin(GUATIAO_LEN_UNKNOWN),
                other => other.clone(),
            })
            .collect();
        assert_eq!(
            encoded(&unknown),
            (Status::GUATIAO_OK, bytes.clone()),
            "{name}: counted"
        );
        assert_eq!(built(&unknown).1, value, "{name}: built uncounted");
        assert_eq!(to_bytes(&value).unwrap(), bytes);
    }
}

#[test]
fn malformed_bytes_are_refused_by_the_visit() {
    let doc = wire_file();
    for v in list(&doc, "malformed") {
        let name = v["name"].as_str().unwrap();
        let bytes = hex(v["hex"].as_str().unwrap());
        // SAFETY: readable bytes and a live table.
        let (status, _) = record(|t| unsafe { guatiao_wire_visit(Bytes::new(&bytes), t) });
        assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{name}");
    }
}

#[test]
fn a_callbacks_status_stops_the_walk_and_is_returned() {
    // [null, true, "10"]
    let value = decode(&hex("060301020104023130")).unwrap();
    let mut record = Record {
        events: Vec::new(),
        stop_at: Some((2, Status::GUATIAO_ERR_NOT_FOUND)),
    };
    let table = visitor(&mut record);
    // SAFETY: a live value and table.
    let status = unsafe { guatiao_value_visit(&value, &table) };
    assert_eq!(status, Status::GUATIAO_ERR_NOT_FOUND);
    assert_eq!(record.events.len(), 2, "nothing after the stop");
}

#[test]
fn a_null_slot_refuses_its_event() {
    let mut record = Record {
        events: Vec::new(),
        stop_at: None,
    };
    let mut table = visitor(&mut record);
    table.null = None;
    let value = Value::null();
    // SAFETY: a live value and table.
    let status = unsafe { guatiao_value_visit(&value, &table) };
    assert_eq!(status, Status::GUATIAO_ERR_WRONG_KIND);
}

#[test]
fn a_table_smaller_than_this_builds_is_refused() {
    let mut record = Record {
        events: Vec::new(),
        stop_at: None,
    };
    let mut table = visitor(&mut record);
    table.struct_size = 8;
    let value = Value::null();
    // SAFETY: a live value; the table is whole, only its size says less.
    let status = unsafe { guatiao_value_visit(&value, &table) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(record.events.is_empty());
    // SAFETY: null is the case under test.
    let status = unsafe { guatiao_value_visit(&value, core::ptr::null()) };
    assert_eq!(status, Status::GUATIAO_ERR_NULL);
}

#[test]
fn a_walker_that_breaks_the_protocol_is_refused() {
    let cases: Vec<(&str, Vec<Event>, Status)> = vec![
        ("nothing", vec![], Status::GUATIAO_ERR_WRONG_KIND),
        (
            "an unclosed list",
            vec![Event::ListBegin(1), Event::Null],
            Status::GUATIAO_ERR_WRONG_KIND,
        ),
        (
            "a key outside a map",
            vec![Event::Key("k".into())],
            Status::GUATIAO_ERR_WRONG_KIND,
        ),
        (
            "a value where a key is due",
            vec![Event::MapBegin(1), Event::Null],
            Status::GUATIAO_ERR_WRONG_KIND,
        ),
        (
            "two values",
            vec![Event::Null, Event::Null],
            Status::GUATIAO_ERR_WRONG_KIND,
        ),
        (
            "not a number",
            vec![Event::Number("1,5".into())],
            Status::GUATIAO_ERR_BAD_VALUE,
        ),
        (
            "a length not kept",
            vec![Event::ListBegin(2), Event::Null, Event::ListEnd],
            Status::GUATIAO_ERR_WRONG_KIND,
        ),
    ];
    for (name, events, status) in cases {
        let (got, bytes) = encoded(&events);
        assert_eq!(got, status, "{name}: encoded");
        assert!(bytes.is_empty(), "{name}: out stays absent");
        let (got, value) = built(&events);
        // A tree takes a length as room to reserve, never a promise.
        if name != "a length not kept" {
            assert_eq!(got, status, "{name}: built");
            assert_eq!(
                value.tag(),
                Ok(guatiao::Tag::GUATIAO_ABSENT),
                "{name}: out stays absent"
            );
        }
    }

    // A repeated key: the wire encoding has none, a tree replaces.
    let repeated = vec![
        Event::MapBegin(GUATIAO_LEN_UNKNOWN),
        Event::Key("a".into()),
        Event::Null,
        Event::Key("a".into()),
        Event::Bool(true),
        Event::MapEnd,
    ];
    assert_eq!(encoded(&repeated).0, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(built(&repeated).0, Status::GUATIAO_OK);
}

/// A walker whose own status is not OK stops the build with it.
#[test]
fn a_walkers_own_status_is_returned() {
    unsafe extern "C" fn refuse(_: *mut c_void, _: *const VisitorTable) -> Status {
        Status::GUATIAO_ERR_GONE
    }
    let table = WalkerTable {
        struct_size: size_of::<WalkerTable>() as u32,
        ctx: core::ptr::null_mut(),
        walk: Some(refuse),
    };
    let alloc = Alloc::rust();
    let mut out = Value::absent();
    // SAFETY: a live walker, allocator and out-slot.
    let status = unsafe { guatiao_value_from_walker(alloc.as_raw(), &table, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_GONE);
}
