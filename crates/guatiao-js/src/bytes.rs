// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading the bytes of a typed array or an array buffer: the one place
//! this crate needs `unsafe`, since QuickJS lends the memory.

use rquickjs::{ArrayBuffer, TypedArray};

/// The bytes a `Uint8Array` holds, copied. Empty when it is detached.
pub(crate) fn of_array(array: &TypedArray<'_, u8>) -> Vec<u8> {
    // SAFETY: the slice is copied on this line and no JavaScript runs
    // while it is alive.
    unsafe { array.as_bytes() }
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}

/// The bytes an `ArrayBuffer` holds, copied. Empty when it is detached.
pub(crate) fn of_buffer(buffer: &ArrayBuffer<'_>) -> Vec<u8> {
    // SAFETY: as in `of_array`.
    unsafe { buffer.as_bytes() }
        .map(<[u8]>::to_vec)
        .unwrap_or_default()
}
