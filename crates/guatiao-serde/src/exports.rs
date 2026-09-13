// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading and writing a document from C — or from Python through
//! `ctypes`, or anything else that can call a C function.
//!
//! **Rust is one user of this crate, not its audience.** A host in any
//! language loads a configuration file into a value and writes one back
//! out, with the same three formats and the same policy knob the Rust API
//! has.
//!
//! # Two functions per format, and one shape between them
//!
//! ```c
//! guatiao_status guatiao_json_parse(guatiao_str text, uint32_t how,
//!                                   const guatiao_alloc *alloc,
//!                                   guatiao_value *out);
//! guatiao_status guatiao_json_emit(const guatiao_value *value, uint32_t how,
//!                                  const guatiao_alloc *alloc,
//!                                  guatiao_value *out);
//! ```
//!
//! **Emitting answers a VALUE holding a string**, not a `char *`. So there
//! is no second thing to free and no second way to get it wrong: the
//! answer is released with `guatiao_value_free`, exactly like every other
//! tree this library hands over, and it carries the allocator that made
//! it.
//!
//! # The policy is one integer
//!
//! Its low byte is the byte-string spelling and the bits above are flags,
//! so a caller writes `GUATIAO_BYTES_ARRAY | GUATIAO_READ_DATA_URIS`.
//! Zero is the default in every position, which is what a caller who
//! passes nothing gets.
//!
//! # Statuses
//!
//! `GUATIAO_ERR_NULL` for a null pointer the call needs,
//! `GUATIAO_ERR_ALLOC` for an allocator that is null or cannot allocate,
//! `GUATIAO_ERR_BAD_VALUE` for text that is not UTF-8 or a document the
//! format refused, `GUATIAO_ERR_WRONG_KIND` for a value the format cannot
//! carry, and `GUATIAO_ERR_INTERNAL` if a panic was caught.
//!
//! # The shape of every function here
//!
//! **The work is the safe Rust API; these convert and call it.** `unsafe`
//! is a handful of pointer conversions with one contract each, rather
//! than a body per export to audit separately.

#![allow(non_camel_case_types)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Value};

use crate::text::Error;
use crate::{Bytes, Presentation};

// --- the policy, as one integer ----------------------------------------

/// How a byte string is spelled where the format has none. The **low
/// byte** of the `how` argument.
pub const GUATIAO_BYTES_DATA_URI: u32 = 0;
/// Bare base64 in a string, with nothing to recognise it by.
pub const GUATIAO_BYTES_BASE64: u32 = 1;
/// An array of numbers: unambiguous, at about four characters a byte.
pub const GUATIAO_BYTES_ARRAY: u32 = 2;
/// Refuse to write one at all.
pub const GUATIAO_BYTES_REFUSE: u32 = 3;

/// Turn a `data:;base64,…` string back into bytes when reading.
///
/// Off unless asked: on, a byte string survives a round trip through a
/// format with none; also on, a string somebody wrote that happens to
/// look like a data URI silently becomes bytes.
pub const GUATIAO_READ_DATA_URIS: u32 = 1 << 8;

/// Indent the output for a person to read.
///
/// Read by `guatiao_json_emit`, which is then the same call as
/// `guatiao_json_emit_pretty`. JSON only: the other formats are already
/// written for a reader and ignore it.
pub const GUATIAO_PRETTY: u32 = 1 << 9;

/// Whether the caller asked for indented output.
///
/// JSON only: the other formats are already written for a reader, and a
/// flag they cannot honour is better ignored than refused -- the same
/// integer describes every format.
fn pretty(how: u32) -> bool {
    how & GUATIAO_PRETTY != 0
}

/// The policy an integer describes.
fn presentation(how: u32) -> Presentation {
    let bytes = match how & 0xff {
        GUATIAO_BYTES_BASE64 => Bytes::Base64,
        GUATIAO_BYTES_ARRAY => Bytes::Array,
        GUATIAO_BYTES_REFUSE => Bytes::Refuse,
        // Including any spelling this build does not know. A reader that
        // guessed would put something in a document the caller did not
        // ask for; the default is the one the Rust API has.
        _ => Bytes::DataUri,
    };
    let mut out = Presentation::new().bytes(bytes);
    if how & GUATIAO_READ_DATA_URIS != 0 {
        out = out.reading_data_uris();
    }
    out
}

/// A format's reader: text, an allocator and a policy in, a value out.
type Read = fn(&str, Alloc, Presentation) -> Result<Value, Error>;

/// A format's writer: a value and a policy in, a document out.
type Write = fn(&Value, Presentation) -> Result<String, Error>;

/// A status for a format's own refusal.
///
/// The format's message is lost, which is what a status costs. The Rust
/// API carries it; a caller that needs it has that.
fn status_of(e: &Error) -> Status {
    match e {
        Error::Format { .. } => Status::GUATIAO_ERR_BAD_VALUE,
        Error::Unsupported { .. } => Status::GUATIAO_ERR_WRONG_KIND,
        // No wildcard arm. `Error` is `#[non_exhaustive]` for a CONSUMER;
        // in here every variant is known, so appending one is a compile
        // error at this match rather than a refusal nobody chose.
    }
}

/// Runs a boundary body, turning a panic into a status rather than an
/// abort, and forgetting the payload rather than dropping it — dropping
/// one can panic, and a second panic in an `extern "C"` body is the abort
/// this exists to avoid.
fn guard(body: impl FnOnce() -> Status) -> Status {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(s) => s,
        Err(payload) => {
            std::mem::forget(payload);
            Status::GUATIAO_ERR_INTERNAL
        }
    }
}

/// The three pointers every function here converts, or the status saying
/// which was wrong.
///
/// # Safety
///
/// `text` is a readable view for the call, `alloc` is null or addresses
/// an allocator, and `out` addresses writable storage for one value.
unsafe fn inputs<'a>(
    text: Str,
    alloc: *const Allocator,
    out: *mut Value,
) -> Result<(&'a str, Alloc), Status> {
    if out.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // A null or incomplete allocator is `GUATIAO_ERR_ALLOC`, not
    // `GUATIAO_ERR_NULL`: the caller passed something, and what it passed
    // cannot allocate. Every crate in this workspace answers it the same
    // way, so one C caller's error handling covers all of them.
    // SAFETY: the caller's contract.
    let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
        return Err(Status::GUATIAO_ERR_ALLOC);
    };
    if text.len > 0 && text.ptr.is_null() {
        return Err(Status::GUATIAO_ERR_NULL);
    }
    // SAFETY: as above; a zero length never dereferences the pointer.
    let bytes = if text.len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(text.ptr, text.len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(Status::GUATIAO_ERR_BAD_VALUE);
    };
    Ok((text, alloc))
}

/// Writes a parsed value out, or the status saying why not.
///
/// # Safety
///
/// `out` addresses writable storage for one value, whose previous
/// contents are the caller's to have freed.
unsafe fn deliver(out: *mut Value, built: Result<Value, Error>) -> Status {
    match built {
        Ok(value) => {
            // SAFETY: the caller's contract.
            unsafe { out.write(value) };
            Status::GUATIAO_OK
        }
        Err(e) => status_of(&e),
    }
}

/// Writes an emitted document out as a STRING VALUE.
///
/// # Safety
///
/// As [`deliver`].
unsafe fn deliver_text(out: *mut Value, built: Result<String, Error>, alloc: Alloc) -> Status {
    match built {
        Ok(text) => match Value::string_in(alloc, &text) {
            Ok(value) => {
                // SAFETY: the caller's contract.
                unsafe { out.write(value) };
                Status::GUATIAO_OK
            }
            Err(e) => Status::from(e),
        },
        Err(e) => status_of(&e),
    }
}

/// One parse, whichever format's reader is handed in.
///
/// The six `extern "C"` functions below are written out rather than
/// generated, because **cbindgen parses syntax and cannot expand a
/// macro** — a macro-generated pair compiles perfectly and then does not
/// appear in the committed header at all, which is the one place a C
/// caller looks. Measured, by writing the macro first.
///
/// The body still exists once. Only the declarations are repeated.
///
/// # Safety
///
/// As the functions that call it.
unsafe fn parse_with(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    read: Read,
) -> Status {
    guard(|| {
        // SAFETY: the caller's contract.
        let (text, alloc) = match unsafe { inputs(text, alloc, out) } {
            Ok(pair) => pair,
            Err(status) => return status,
        };
        // SAFETY: as above.
        unsafe { deliver(out, read(text, alloc, presentation(how))) }
    })
}

/// One emit, likewise.
///
/// `indented` is the same format's writer for a person to read, for the
/// one format that has two spellings; a format with only one passes
/// `None` and `GUATIAO_PRETTY` means nothing to it.
///
/// # Safety
///
/// As the functions that call it.
unsafe fn emit_with(
    value: *const Value,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    write: Write,
    indented: Option<Write>,
) -> Status {
    guard(|| {
        if value.is_null() {
            return Status::GUATIAO_ERR_NULL;
        }
        // SAFETY: the caller's contract. An empty view never dereferences.
        let (_, alloc) = match unsafe { inputs(Str::empty(), alloc, out) } {
            Ok(pair) => pair,
            Err(status) => return status,
        };
        // SAFETY: as above.
        let value = unsafe { &*value };
        let write = match indented {
            Some(indented) if pretty(how) => indented,
            _ => write,
        };
        // SAFETY: as above.
        unsafe { deliver_text(out, write(value, presentation(how)), alloc) }
    })
}

/// Reads a JSON document into a value, built through `alloc`.
///
/// # Safety
///
/// `text` is a readable view for this call, `alloc` addresses a complete
/// allocator outliving every tree built through it, and `out` addresses
/// writable storage for one value whose previous contents are the
/// caller's to have freed.
#[cfg(feature = "json")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_json_parse(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded, with this crate's JSON reader.
    unsafe { parse_with(text, how, alloc, out, crate::text::json::from_str) }
}

/// Writes a value as a JSON document.
///
/// **`out` receives a value holding the text**, released with
/// `guatiao_value_free` like any other tree — so there is no second thing
/// to free and no second way to get it wrong.
///
/// `GUATIAO_PRETTY` in `how` indents the output, which is the same thing
/// `guatiao_json_emit_pretty` does.
///
/// # Safety
///
/// `value` addresses a well-formed value, `alloc` addresses a complete
/// allocator, and `out` addresses writable storage for one value whose
/// previous contents are the caller's to have freed.
#[cfg(feature = "json")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_json_emit(
    value: *const Value,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe {
        emit_with(
            value,
            how,
            alloc,
            out,
            crate::text::json::to_string,
            Some(crate::text::json::to_string_pretty),
        )
    }
}

/// The same, indented for a person to read.
///
/// A function as well as the `GUATIAO_PRETTY` flag, because a caller that
/// writes one document for a person has no policy integer to put a flag
/// in and should not have to invent one.
///
/// # Safety
///
/// As [`guatiao_json_emit`].
#[cfg(feature = "json")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_json_emit_pretty(
    value: *const Value,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe {
        emit_with(
            value,
            how,
            alloc,
            out,
            crate::text::json::to_string_pretty,
            None,
        )
    }
}

/// Reads a TOML document into a value, built through `alloc`.
///
/// # Safety
///
/// As [`guatiao_json_parse`].
#[cfg(feature = "toml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_toml_parse(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe { parse_with(text, how, alloc, out, crate::text::toml::from_str) }
}

/// Writes a value as a TOML document.
///
/// **A TOML document is a table**, so a value that is not a map is
/// refused with `GUATIAO_ERR_WRONG_KIND` rather than wrapped in a key it
/// never had.
///
/// # Safety
///
/// As [`guatiao_json_emit`].
#[cfg(feature = "toml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_toml_emit(
    value: *const Value,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe { emit_with(value, how, alloc, out, crate::text::toml::to_string, None) }
}

/// Reads a YAML document into a value, built through `alloc`.
///
/// # Safety
///
/// As [`guatiao_json_parse`].
#[cfg(feature = "yaml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_yaml_parse(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe { parse_with(text, how, alloc, out, crate::text::yaml::from_str) }
}

/// Writes a value as a YAML document.
///
/// # Safety
///
/// As [`guatiao_json_emit`].
#[cfg(feature = "yaml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_yaml_emit(
    value: *const Value,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded.
    unsafe { emit_with(value, how, alloc, out, crate::text::yaml::to_string, None) }
}
