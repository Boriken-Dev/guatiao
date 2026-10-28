// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A kind's description from C: the document a kind crate commits beside
//! its header, read from its text, for a binding with no JSON reader of
//! its own; and the floor hash of a kind a language describes itself.

use crate::exports::{entry, out};
use crate::library::kind::KindDescription;
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Str, Text, Value};

/// Reads a kinds document from its JSON text: what a kind crate commits as
/// `<crate>.kinds.json` beside its header, `{"kinds": [...]}`
/// as `docs/encodings.md` states it ("A kind, described"). On
/// `GUATIAO_OK`, `out` holds the document as a value built in `alloc`, a
/// map a caller reads each kind from; the caller frees it with
/// `guatiao_value_free`. The same document comes, already read, from
/// `guatiao_registry_kind_descriptions` for a provider that carries it.
///
/// `GUATIAO_ERR_BAD_VALUE` for text that is not JSON (`why` then names the
/// byte where it stops) or not a description (`why` names the place, such
/// as `kinds[0].slots[2].args[1]`, and what is wrong there), a kind that
/// declares what `#[guatiao::kind]` refuses included: a slot named twice,
/// an object kind's `destroy`, `bytes_mut` on a kind that is not an object
/// kind. Required and optional slots come in any order. `why`, which
/// may be null, holds that text as a string built in `alloc`, else stays
/// ABSENT. `GUATIAO_ERR_ALLOC` for a null or unusable allocator.
///
/// # Safety
///
/// The view's bytes are readable for the call; `alloc` is null or valid;
/// `out` and `why` are null or address writable storage for one value
/// holding nothing the caller still owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_kind_document_read(
    text: Str,
    alloc: *const Allocator,
    out: *mut Value,
    why: *mut Value,
) -> Status {
    out!(out, why);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `why` is null or the caller's slot, holding ABSENT.
        let refused = |message: String| unsafe { refuse(why, alloc, &message) };
        let text = match core::str::from_utf8(text.into()) {
            Ok(text) => text,
            Err(e) => return refused(format!("byte {}: is not UTF-8", e.valid_up_to())),
        };
        match KindDescription::document_value(text, alloc) {
            // SAFETY: `out` is non-null, the caller's slot holding ABSENT;
            // the document records `alloc`.
            Ok(document) => unsafe { document.write_to(out, alloc.as_raw()) },
            Err(e) => refused(e.to_string()),
        }
    })
}

/// The floor hash of a kind born outside Rust, from its description as
/// JSON text: one kind as a kinds document holds it, whose `object` and
/// `slots` are read and checked as `guatiao_kind_document_read` checks
/// them, and whose other keys (`name`, `floor_hash`, `doc`) are not. The
/// hash is FNV-1a (32-bit) over `provider` or `object` and every slot in
/// table order, each spelled as `#[guatiao::kind]` spells a trait's method
/// (`docs/encodings.md`, "A kind, described"), so a declaration in any
/// language repeating a trait's slots gets the trait's hash. On
/// `GUATIAO_OK`, `*out` holds it.
///
/// `GUATIAO_ERR_BAD_VALUE` for text that is not JSON or not a kind, `why`
/// (which may be null) then holding the reason as a string built in
/// `alloc`, worded as `guatiao_kind_document_read` words it, else ABSENT.
/// `GUATIAO_ERR_ALLOC` for a null or unusable allocator, `GUATIAO_ERR_NULL`
/// for a null `out`.
///
/// # Safety
///
/// The view's bytes are readable for the call; `alloc` is null or valid;
/// `out` is null or writable; `why` is null or addresses writable storage
/// for one value holding nothing the caller still owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_kind_floor_hash(
    kind: Str,
    alloc: *const Allocator,
    out: *mut u32,
    why: *mut Value,
) -> Status {
    out!(why);
    entry!(out => {
        // SAFETY: null or valid by the caller's contract.
        let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
            return Status::GUATIAO_ERR_ALLOC;
        };
        // SAFETY: `why` is null or the caller's slot, holding ABSENT.
        let refused = |message: String| unsafe { refuse(why, alloc, &message) };
        let text = match core::str::from_utf8(kind.into()) {
            Ok(text) => text,
            Err(e) => return refused(format!("byte {}: is not UTF-8", e.valid_up_to())),
        };
        match KindDescription::floor_hash_json(text) {
            Ok(hash) => {
                // SAFETY: `out` is non-null and writable by the caller's
                // contract.
                unsafe { out.write(hash) };
                Status::GUATIAO_OK
            }
            Err(e) => refused(e.to_string()),
        }
    })
}

/// Writes `message` into `why`, when it is not null, and answers
/// `GUATIAO_ERR_BAD_VALUE`.
///
/// # Safety
///
/// `why` is null or addresses writable storage holding ABSENT.
unsafe fn refuse(why: *mut Value, alloc: Alloc, message: &str) -> Status {
    if !why.is_null()
        && let Ok(text) = Text::new_in(alloc, message)
    {
        // SAFETY: non-null, writable and holding ABSENT by the contract.
        unsafe { Value::from(text).write_to(why, alloc.as_raw()) };
    }
    Status::GUATIAO_ERR_BAD_VALUE
}
