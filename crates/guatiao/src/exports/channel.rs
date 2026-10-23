// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Channels, from C: the frames `guatiao::value::wire::channel` writes,
//! and a receiver that keeps each channel's schema and checks every value
//! against it.
//!
//! ```text
//! frame := kind:u8 channel:LEB128 payload
//! kind  := GUATIAO_FRAME_SCHEMA (0) | GUATIAO_FRAME_VALUE (1) | GUATIAO_FRAME_CLOSE (2)
//! ```
//!
//! A builder answers the frame as a BYTES value; the receiver answers a
//! [`WireEvent`]. A refusal never quotes what was received: it names the
//! byte offset, the channel, and for a value its schema refuses, the
//! field and what it would have accepted.

#![allow(non_camel_case_types)]

use core::ptr;

use super::schema::describe_validation;
use super::{entry, guard, guard_with, out};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Buffer, Bytes, Text, Value};
use crate::value::wire::WireError;
use crate::value::wire::channel::{self, Received, Receiver};

/// A `guatiao_wire_event`'s `refusal` for a frame that was read.
pub const GUATIAO_WIRE_ACCEPTED: u32 = 0;
/// A `guatiao_wire_event`'s `refusal` for bytes that are not a frame: truncated, an
/// unknown frame kind or tag, a length not in its shortest form, text that
/// is not UTF-8, a number outside the grammar, a repeated key, bytes after
/// the value. `at` names the byte. The status is `GUATIAO_ERR_BAD_VALUE`.
pub const GUATIAO_WIRE_MALFORMED: u32 = 1;
/// A `guatiao_wire_event`'s `refusal` for a value on a channel no schema was announced
/// on. The status is `GUATIAO_ERR_NOT_FOUND`.
pub const GUATIAO_WIRE_NO_SCHEMA: u32 = 2;
/// A `guatiao_wire_event`'s `refusal` for a schema frame whose value is not a schema.
/// The status is `GUATIAO_ERR_WRONG_KIND`.
pub const GUATIAO_WIRE_NOT_A_SCHEMA: u32 = 3;
/// A `guatiao_wire_event`'s `refusal` for a value its channel's schema refuses;
/// `detail` says which field and why. The status is
/// `GUATIAO_ERR_BAD_VALUE`.
pub const GUATIAO_WIRE_INVALID: u32 = 4;

/// What one frame said, or why it was refused.
///
/// Written empty (zeros, every value ABSENT) on entry to
/// `guatiao_wire_receiver_accept`. Its three values are the caller's, each
/// freed with `guatiao_value_free`.
#[repr(C)]
#[derive(Debug)]
pub struct WireEvent {
    /// The channel the frame named; 0 when the frame ended before it.
    pub channel: u64,
    /// The frame's kind byte (`GUATIAO_FRAME_SCHEMA`, `_VALUE`, `_CLOSE`,
    /// or the unknown byte a refusal names); 0 for an empty frame.
    pub kind: u32,
    /// `GUATIAO_WIRE_ACCEPTED`, or which refusal: `GUATIAO_WIRE_*`.
    pub refusal: u32,
    /// For `GUATIAO_WIRE_MALFORMED`, the offset of the byte in the frame
    /// it went wrong at; 0 otherwise.
    pub at: usize,
    /// For a schema frame, a copy of the schema now the channel's; for a
    /// value frame, the value, checked against it. ABSENT otherwise.
    pub value: Value,
    /// The refusal in words, a STRING; ABSENT when the frame was read.
    pub message: Value,
    /// For `GUATIAO_WIRE_INVALID`, the map `guatiao_schema_validate` writes
    /// (`key`, then `expected` or `known`, and `message`); ABSENT otherwise.
    pub detail: Value,
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(offset_of!(WireEvent, kind) == 8 && offset_of!(WireEvent, refusal) == 12);
    assert!(offset_of!(WireEvent, at) == 16);
    assert!(offset_of!(WireEvent, value) == 16 + size_of::<usize>());
    assert!(offset_of!(WireEvent, message) == offset_of!(WireEvent, value) + size_of::<Value>());
    assert!(offset_of!(WireEvent, detail) == offset_of!(WireEvent, message) + size_of::<Value>());
};

impl WireEvent {
    fn empty() -> WireEvent {
        WireEvent {
            channel: 0,
            kind: 0,
            refusal: GUATIAO_WIRE_ACCEPTED,
            at: 0,
            value: Value::absent(),
            message: Value::absent(),
            detail: Value::absent(),
        }
    }
}

/// A receiver: each channel's schema, and the allocator everything it
/// answers is built in. Opaque to C.
#[derive(Debug)]
pub struct WireReceiver {
    inner: Receiver,
    alloc: Alloc,
}

// --- frames -------------------------------------------------------------

/// Writes `frame` through `out` as a BYTES value built in `alloc`.
///
/// # Safety
///
/// `alloc` is null or valid; `out` is non-null, writable, and holds no
/// value the caller still owns.
unsafe fn answer_frame(
    alloc: *const Allocator,
    frame: Result<Vec<u8>, crate::value::error::ValueError>,
    out: *mut Value,
) -> Status {
    // SAFETY: null or valid by the caller's contract.
    let Ok(a) = (unsafe { Alloc::from_raw(alloc) }) else {
        return Status::GUATIAO_ERR_ALLOC;
    };
    let frame = match frame {
        Ok(frame) => frame,
        Err(e) => return e.into(),
    };
    match Buffer::new_in(a, &frame).map(Value::from) {
        Ok(v) => {
            // SAFETY: `out` is writable and holds no value the caller owns.
            unsafe { ptr::write(out, v) };
            Status::GUATIAO_OK
        }
        Err(e) => e.into(),
    }
}

/// The frame that announces `schema` on `channel`, written through `out`
/// as a BYTES value built in `alloc`. A later announcement on the same
/// channel replaces it.
///
/// `out` is written the absent marker on entry. The schema is not judged
/// here: a receiver refuses one that is not a schema. A node `schema`
/// holds that its own reader refuses (text that is not UTF-8, a number
/// outside the grammar, an unknown tag) is `GUATIAO_ERR_BAD_VALUE`; a null
/// or unusable allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// The pointers are null or valid, and `out` addresses writable storage
/// for one value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_announce(
    alloc: *const Allocator,
    channel: u64,
    schema: *const Value,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(schema, out => {
        // SAFETY: checked non-null and well-formed by contract.
        let frame = channel::announce(channel, unsafe { &*schema });
        // SAFETY: forwarded; `out` checked non-null.
        unsafe { answer_frame(alloc, frame, out) }
    })
}

/// The frame that carries `value` on `channel`, written through `out` as
/// a BYTES value built in `alloc`.
///
/// `out` is written the absent marker on entry; the refusals are
/// `guatiao_wire_announce`'s.
///
/// # Safety
///
/// As `guatiao_wire_announce`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_send(
    alloc: *const Allocator,
    channel: u64,
    value: *const Value,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(value, out => {
        // SAFETY: checked non-null and well-formed by contract.
        let frame = channel::send(channel, unsafe { &*value });
        // SAFETY: forwarded; `out` checked non-null.
        unsafe { answer_frame(alloc, frame, out) }
    })
}

/// The frame that ends `channel`, written through `out` as a BYTES value
/// built in `alloc`. A receiver forgets the channel's schema.
///
/// `out` is written the absent marker on entry. A null or unusable
/// allocator is `GUATIAO_ERR_ALLOC`.
///
/// # Safety
///
/// `alloc` is null or valid, and `out` addresses writable storage for one
/// value that does not already hold one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_close(
    alloc: *const Allocator,
    channel: u64,
    out: *mut Value,
) -> Status {
    out!(out);
    entry!(out => {
        // SAFETY: forwarded; `out` checked non-null.
        unsafe { answer_frame(alloc, Ok(channel::close(channel)), out) }
    })
}

// --- the receiver ---------------------------------------------------------

/// A receiver that builds everything it answers, and the schemas it
/// keeps, in `alloc`. Null when `alloc` is null or unusable.
///
/// Free it with `guatiao_wire_receiver_free`. One caller at a time: it
/// is not safe to use from two threads at once.
///
/// # Safety
///
/// `alloc` is null or valid, and outlives the receiver and every value it
/// answers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_receiver_new(alloc: *const Allocator) -> *mut WireReceiver {
    guard_with(ptr::null_mut(), || {
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return ptr::null_mut();
        };
        Box::into_raw(Box::new(WireReceiver {
            inner: Receiver::new_in(alloc),
            alloc,
        }))
    })
}

/// Frees a receiver and the schemas it keeps. Null is accepted and does
/// nothing.
///
/// # Safety
///
/// `recv` is null, or a live handle from `guatiao_wire_receiver_new` that
/// has not been freed and that nothing else is using.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_receiver_free(recv: *mut WireReceiver) {
    if recv.is_null() {
        return;
    }
    guard_with((), || {
        // SAFETY: the caller's contract.
        drop(unsafe { Box::from_raw(recv) });
    });
}

/// Reads one `frame`, writing what it said through `out_event`.
///
/// A schema frame makes its schema the channel's; a value frame answers
/// its value only once its channel has a schema, and only if that schema
/// accepts it; a close frame forgets the channel's schema. The event's
/// `value` is the caller's: the schema's copy or the checked value, built
/// in the receiver's allocator.
///
/// A refusal is a status, with `out_event` saying which and why
/// (`refusal`, `at`, `message`, `detail`) and changing nothing the
/// receiver keeps:
///
/// - bytes that are not a frame: `GUATIAO_ERR_BAD_VALUE`,
///   `GUATIAO_WIRE_MALFORMED`, `at` the byte;
/// - a value on a channel with no schema: `GUATIAO_ERR_NOT_FOUND`,
///   `GUATIAO_WIRE_NO_SCHEMA`;
/// - a schema frame that is not a schema: `GUATIAO_ERR_WRONG_KIND`,
///   `GUATIAO_WIRE_NOT_A_SCHEMA`;
/// - a value its schema refuses: `GUATIAO_ERR_BAD_VALUE`,
///   `GUATIAO_WIRE_INVALID`, `detail` the field and what it accepts.
///
/// The allocator refusing is `GUATIAO_ERR_ALLOC` with `refusal` left
/// `GUATIAO_WIRE_ACCEPTED`; a schema frame that got that far is already
/// the channel's. `out_event` is written empty on entry.
///
/// # Safety
///
/// `recv` is null or a live handle nothing else is using; the frame's
/// bytes are readable for the call; `out_event` is null or addresses
/// writable storage for one event holding no value the caller still owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_wire_receiver_accept(
    recv: *mut WireReceiver,
    frame: Bytes,
    out_event: *mut WireEvent,
) -> Status {
    if !out_event.is_null() {
        // SAFETY: checked non-null; writable and holding nothing the
        // caller owns, by contract.
        unsafe { ptr::write(out_event, WireEvent::empty()) };
    }
    if recv.is_null() || out_event.is_null() {
        return Status::GUATIAO_ERR_NULL;
    }
    guard(|| {
        // SAFETY: checked non-null; live and unshared by contract.
        let recv = unsafe { &mut *recv };
        // SAFETY: the caller guarantees the view's bytes.
        let bytes: &[u8] = frame.into();
        // SAFETY: checked non-null, and written empty above.
        let event = unsafe { &mut *out_event };
        let (kind, channel) = channel::head(bytes);
        event.kind = kind.map_or(0, u32::from);
        event.channel = channel.unwrap_or(0);
        match recv.inner.accept(bytes) {
            Ok(Received::Schema(channel)) => {
                let schema = recv
                    .inner
                    .schema(channel)
                    .expect("the frame just announced it");
                match schema.clone_in(recv.alloc) {
                    Ok(copy) => event.value = copy,
                    Err(e) => return e.into(),
                }
                Status::GUATIAO_OK
            }
            Ok(Received::Value { value, .. }) => {
                event.value = value;
                Status::GUATIAO_OK
            }
            Ok(Received::Closed(_)) => Status::GUATIAO_OK,
            Err(error) => refuse(event, &error, recv.alloc),
        }
    })
}

/// Fills `event` with `error`, and answers its status.
fn refuse(event: &mut WireEvent, error: &WireError, alloc: Alloc) -> Status {
    let (refusal, status) = match error {
        WireError::NoSchema { channel } => {
            event.channel = *channel;
            (GUATIAO_WIRE_NO_SCHEMA, Status::GUATIAO_ERR_NOT_FOUND)
        }
        WireError::NotASchema { channel } => {
            event.channel = *channel;
            (GUATIAO_WIRE_NOT_A_SCHEMA, Status::GUATIAO_ERR_WRONG_KIND)
        }
        WireError::Invalid { channel, error } => {
            event.channel = *channel;
            event.detail = describe_validation(error, alloc).unwrap_or_else(Value::absent);
            (GUATIAO_WIRE_INVALID, Status::GUATIAO_ERR_BAD_VALUE)
        }
        WireError::Alloc { at, error } => {
            event.at = *at;
            (GUATIAO_WIRE_ACCEPTED, (*error).into())
        }
        WireError::Truncated { at }
        | WireError::UnknownTag { at, .. }
        | WireError::Overlong { at }
        | WireError::NotUtf8 { at }
        | WireError::NotANumber { at }
        | WireError::NotABool { at, .. }
        | WireError::DuplicateKey { at }
        | WireError::Trailing { at }
        | WireError::UnknownFrame { at, .. } => {
            event.at = *at;
            (GUATIAO_WIRE_MALFORMED, Status::GUATIAO_ERR_BAD_VALUE)
        }
    };
    event.refusal = refusal;
    event.message =
        Text::new_in(alloc, &error.to_string()).map_or_else(|_| Value::absent(), Value::from);
    status
}
