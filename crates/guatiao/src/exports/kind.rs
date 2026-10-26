// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A kind's description from C: the document a kind crate commits beside
//! its header, read from its text, for a binding with no JSON reader of
//! its own.

use crate::exports::{entry, out};
use crate::library::kind::KindDescription;
use crate::value::alloc::{Alloc, Allocator};
use crate::value::status::Status;
use crate::value::types::{Str, Text, Value};

/// Reads a kinds document from its JSON text: what a kind crate commits as
/// `<crate>.kinds.json` beside its header, `{"format": 1, "kinds": [...]}`
/// as `docs/encodings.md` states it ("A kind, described"). On
/// `GUATIAO_OK`, `out` holds the document as a value built in `alloc`, a
/// map a caller reads each kind from; the caller frees it with
/// `guatiao_value_free`. The same document comes, already read, from
/// `guatiao_registry_kind_descriptions` for a provider that carries it.
///
/// `GUATIAO_ERR_BAD_VALUE` for text that is not JSON (`why` then names the
/// byte where it stops) or not a description (`why` names the place, such
/// as `kinds[0].slots[2].args[1]`, and what is wrong there); `why`, which
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
        let refused = |message: String| {
            if !why.is_null()
                && let Ok(text) = Text::new_in(alloc, &message)
            {
                // SAFETY: `why` is non-null and writable by the caller's
                // contract, and holds the ABSENT written on entry.
                unsafe { Value::from(text).write_to(why, alloc.as_raw()) };
            }
            Status::GUATIAO_ERR_BAD_VALUE
        };
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
