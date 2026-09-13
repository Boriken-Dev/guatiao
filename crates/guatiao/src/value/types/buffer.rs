// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bytes: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::mem::ManuallyDrop;

use crate::value::alloc::Allocator;
// The raw layer this crate keeps to itself: the free walk, the
// allocator-taking mutators and the private helpers. Imported whole
// because the impl below calls into it at almost every line.
use crate::value::alloc::Alloc;
use crate::value::mutate::*;
use crate::value::raw::release_buffer;

use super::{Payload, Tag, Value};

/// Borrowed bytes: any content at all, NULs included.
///
/// Same shape and the same check-the-length rule as `guatiao_str`; a
/// separate type because "text" and "arbitrary bytes" are different
/// promises and collapsing them loses the distinction at every call site.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Bytes {
    /// First byte. May be null or dangling when `len` is 0.
    pub ptr: *const u8,
    /// Length in bytes.
    pub len: usize,
}

impl Bytes {
    /// A view of bytes this program already holds.
    ///
    /// `'static` where [`Str::borrowed`](super::Str::borrowed) takes any
    /// lifetime, because the view keeps no lifetime of its own: it is a
    /// C struct, and the only borrow that is free of a guarantee somebody
    /// has to make by hand is one that outlives the program.
    pub const fn borrowed(bytes: &'static [u8]) -> Bytes {
        Bytes {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }

    /// An empty view.
    pub const fn empty() -> Bytes {
        Bytes {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}

/// Owned, growable bytes. See `guatiao_string` for the `cap` rule.
#[repr(C)]
#[derive(Debug)]
pub struct Buffer {
    /// First byte.
    pub ptr: *mut u8,
    /// Length in bytes.
    pub len: usize,
    /// Capacity in bytes. 0 means the buffer is not owned.
    pub cap: usize,
    /// The allocator that made this buffer. Null when `cap == 0`.
    pub alloc: *const Allocator,
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: as for `Text`.
        unsafe { release_buffer(self) }
    }
}

impl Buffer {
    /// Bytes, copied onto Rust's heap. See [`Text::new`](super::Text::new).
    pub fn new(bytes: &[u8]) -> Buffer {
        or_abort(Buffer::new_in(Alloc::rust(), bytes))
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc, bytes: &[u8]) -> Result<Buffer, ValueError> {
        Ok(owned_bytes(alloc, bytes)?)
    }

    /// The bytes themselves.
    pub fn as_slice(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` bytes are initialised.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

/// Empty, growing through Rust's allocator. See [`Text::default`](super::Text).
impl Default for Buffer {
    fn default() -> Buffer {
        Buffer::new(&[])
    }
}

impl From<Buffer> for Value {
    fn from(bytes: Buffer) -> Value {
        let mut v = blank(Tag::GUATIAO_BYTES);
        v.payload = Payload {
            bytes: ManuallyDrop::new(bytes),
        };
        v
    }
}
