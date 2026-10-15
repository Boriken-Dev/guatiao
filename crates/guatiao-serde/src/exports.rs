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
//! **`out` is written ABSENT on entry**, before anything can fail, so a
//! caller reading it after an error reads `GUATIAO_ABSENT` rather than
//! whatever it held — the same convention `guatiao` and `guatiao-intake`
//! follow. It must not hold a tree the caller still owns.
//!
//! # Where a parse stopped
//!
//! Each `_parse` has a twin, `_parse_ex`, taking one more pointer, a
//! `guatiao_parse_error *` that may be null. Its `format` is the call's on
//! every return. On a document that does not parse, or text that is not
//! UTF-8, it receives the line and the column (from 1, the column in
//! characters, 0 when the parser did not say) and a `message` string
//! value, freed with `guatiao_value_free`. The message is the parser's
//! wording and never quotes the document, which may hold a secret.
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

use core::panic::AssertUnwindSafe;
use std::panic::catch_unwind;

use guatiao::value::alloc::{Alloc, Allocator};
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Text, Value};

use crate::text::{Error, Position, position_at};
use crate::{Bytes, Presentation};

/// Which format refused a document: `guatiao_parse_error.format`.
pub const GUATIAO_FORMAT_JSON: u32 = 0;
/// See [`GUATIAO_FORMAT_JSON`].
pub const GUATIAO_FORMAT_TOML: u32 = 1;
/// See [`GUATIAO_FORMAT_JSON`].
pub const GUATIAO_FORMAT_YAML: u32 = 2;

/// Why a document did not parse, for a caller that can show where.
///
/// Filled by the `_parse_ex` functions. `format` is the call's on every
/// return, whatever failed. `line` and `column` count from 1, the column
/// in characters, and are 0 when the parser did not say where; for text
/// that is not UTF-8 they name its first byte that is not. `message` is a
/// string value built through the caller's allocator, ABSENT when the
/// call did not fail on the text, and released with `guatiao_value_free`.
#[repr(C)]
pub struct ParseError {
    /// A `GUATIAO_FORMAT_*` constant: the format the call reads.
    pub format: u32,
    /// The line, from 1; 0 when unknown.
    pub line: u32,
    /// The column, from 1, in characters; 0 when unknown.
    pub column: u32,
    /// What the parser expected, without an excerpt of the document.
    pub message: Value,
}

impl ParseError {
    /// Nothing known yet but the format: no position and no message.
    fn none(format: u32) -> ParseError {
        ParseError {
            format,
            line: 0,
            column: 0,
            message: Value::absent(),
        }
    }

    /// The refusal, with its message built through `alloc`.
    fn of(error: &Error, format: u32, alloc: Alloc) -> Option<ParseError> {
        let detail = match error {
            Error::Format { detail, .. }
            | Error::Syntax { detail, .. }
            | Error::Unsupported { detail, .. } => detail,
        };
        ParseError::at(format, error.position(), detail, alloc)
    }

    /// A refusal at `position`, its message built through `alloc`.
    fn at(
        format: u32,
        position: Option<Position>,
        detail: &str,
        alloc: Alloc,
    ) -> Option<ParseError> {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Some(ParseError {
            format,
            line: position.map_or(0, |p| count(p.line)),
            column: position.map_or(0, |p| count(p.column)),
            message: Text::new_in(alloc, detail).map(Value::from).ok()?,
        })
    }
}

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
        Error::Format { .. } | Error::Syntax { .. } => Status::GUATIAO_ERR_BAD_VALUE,
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
            core::mem::forget(payload);
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
    text: Str<'a>,
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
    let text = core::str::from_utf8(text.into()).map_err(Status::from)?;
    Ok((text, alloc))
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
/// `out_error` is null for the plain `_parse` functions; the `_parse_ex`
/// ones pass the caller's. Both pointers are written on entry (`out`
/// ABSENT, `out_error` empty but for its format), so a failure leaves
/// neither holding anything.
///
/// # Safety
///
/// As the functions that call it.
unsafe fn parse_with(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    out_error: *mut ParseError,
    format: u32,
    read: Read,
) -> Status {
    // Both out-pointers first, so a failure leaves neither holding
    // anything. A null `out` is refused below, by `inputs`.
    //
    // SAFETY: the caller's contract, for both pointers.
    unsafe {
        let _ = Value::absent().write_to(out, core::ptr::null());
        if !out_error.is_null() {
            core::ptr::write(out_error, ParseError::none(format));
        }
    }
    // Writes a refusal through `out_error` when there is one, or answers
    // that its message could not be built: the allocator is what to fix
    // first.
    let report = |detail: Option<ParseError>, status: Status| {
        if out_error.is_null() {
            return status;
        }
        let Some(detail) = detail else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: non-null, and writable by the caller's contract.
        unsafe { out_error.write(detail) };
        status
    };
    guard(|| {
        let bytes: &[u8] = text.into();
        // SAFETY: the caller's contract; an empty view never dereferences.
        let (_, alloc) = match unsafe { inputs(Str::empty(), alloc, out) } {
            Ok(pair) => pair,
            Err(status) => return status,
        };
        let text = match core::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => {
                // Where the text stops being UTF-8, counted over the part
                // that is.
                let valid = core::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or("");
                let at = position_at(valid, valid.len());
                let detail = ParseError::at(format, Some(at), "the text is not UTF-8", alloc);
                return report(detail, Status::from(e));
            }
        };
        match read(text, alloc, presentation(how)) {
            // SAFETY: `out` is writable by contract and was written ABSENT
            // on entry; the tree, built through `alloc`, moves into it.
            Ok(value) => unsafe { value.write_to(out, alloc.as_raw()) },
            Err(e) => report(ParseError::of(&e, format, alloc), status_of(&e)),
        }
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
    // `out` first, so a failure leaves it holding nothing; a null one is
    // refused below, by `inputs`.
    //
    // SAFETY: the caller's contract.
    let _ = unsafe { Value::absent().write_to(out, core::ptr::null()) };
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
        // The document crosses as a STRING value built through `alloc`.
        match write(value, presentation(how)).map(|text| Text::new_in(alloc, &text)) {
            // SAFETY: `out` is writable by contract and was written ABSENT
            // on entry.
            Ok(Ok(text)) => unsafe { Value::from(text).write_to(out, alloc.as_raw()) },
            Ok(Err(e)) => Status::from(e),
            Err(e) => status_of(&e),
        }
    })
}

/// Reads a JSON document into a value, built through `alloc`.
///
/// `out` is written ABSENT on entry, so a failed call leaves it ABSENT.
/// A document that does not parse is `GUATIAO_ERR_BAD_VALUE`; the `_ex`
/// twin says where.
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
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            core::ptr::null_mut(),
            GUATIAO_FORMAT_JSON,
            crate::text::json::from_str,
        )
    }
}

/// [`guatiao_json_parse`], and where it stopped when the document is not
/// JSON.
///
/// `out_error` may be null, which is the same call. Otherwise it is
/// written empty on entry and, on a document that does not parse, receives
/// the format, the line and column and the parser's message (see
/// [`ParseError`]); free its `message` with `guatiao_value_free`.
///
/// # Safety
///
/// As [`guatiao_json_parse`]; `out_error` is null or addresses writable
/// storage for one `guatiao_parse_error` that holds no message the caller
/// still owns.
#[cfg(feature = "json")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_json_parse_ex(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    out_error: *mut ParseError,
) -> Status {
    // SAFETY: forwarded.
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            out_error,
            GUATIAO_FORMAT_JSON,
            crate::text::json::from_str,
        )
    }
}

/// Writes a value as a JSON document.
///
/// **`out` receives a value holding the text**, released with
/// `guatiao_value_free` like any other tree — so there is no second thing
/// to free and no second way to get it wrong. It is written ABSENT on
/// entry, so a failed call leaves it ABSENT.
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
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            core::ptr::null_mut(),
            GUATIAO_FORMAT_TOML,
            crate::text::toml::from_str,
        )
    }
}

/// [`guatiao_toml_parse`], and where it stopped when the document is not
/// TOML. See [`guatiao_json_parse_ex`].
///
/// # Safety
///
/// As [`guatiao_json_parse_ex`].
#[cfg(feature = "toml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_toml_parse_ex(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    out_error: *mut ParseError,
) -> Status {
    // SAFETY: forwarded.
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            out_error,
            GUATIAO_FORMAT_TOML,
            crate::text::toml::from_str,
        )
    }
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
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            core::ptr::null_mut(),
            GUATIAO_FORMAT_YAML,
            crate::text::yaml::from_str,
        )
    }
}

/// [`guatiao_yaml_parse`], and where it stopped when the document is not
/// YAML. See [`guatiao_json_parse_ex`].
///
/// # Safety
///
/// As [`guatiao_json_parse_ex`].
#[cfg(feature = "yaml")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_yaml_parse_ex(
    text: Str,
    how: u32,
    alloc: *const Allocator,
    out: *mut Value,
    out_error: *mut ParseError,
) -> Status {
    // SAFETY: forwarded.
    unsafe {
        parse_with(
            text,
            how,
            alloc,
            out,
            out_error,
            GUATIAO_FORMAT_YAML,
            crate::text::yaml::from_str,
        )
    }
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
