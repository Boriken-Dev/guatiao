// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading the bytes of a typed array or an array buffer: the one place
//! this crate needs `unsafe`, since QuickJS lends the memory.

use rquickjs::{ArrayBuffer, TypedArray};

/// The bytes a `Uint8Array` holds, copied. Empty when it is detached,
/// which QuickJS reports by leaving a `TypeError` pending: cleared here, or
/// the next import in any context would take it for its own.
pub(crate) fn of_array(array: &TypedArray<'_, u8>) -> Vec<u8> {
    // SAFETY: the slice is copied on this line and no JavaScript runs
    // while it is alive.
    let held = unsafe { array.as_bytes() }.map(<[u8]>::to_vec);
    held.unwrap_or_else(|| {
        drop(array.ctx().catch());
        Vec::new()
    })
}

/// The bytes an `ArrayBuffer` holds, copied. Empty when it is detached.
pub(crate) fn of_buffer(buffer: &ArrayBuffer<'_>) -> Vec<u8> {
    // SAFETY: as in `of_array`.
    let held = unsafe { buffer.as_bytes() }.map(<[u8]>::to_vec);
    held.unwrap_or_else(|| {
        drop(buffer.ctx().catch());
        Vec::new()
    })
}
