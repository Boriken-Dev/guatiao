// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! UTF-8 text: the borrowed view and the owned container.

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

/// Borrowed UTF-8 text: a pointer and a length, no NUL terminator.
///
/// **Check `len` before `ptr`.** An empty view may carry a dangling or
/// null pointer, and the pointer must not be touched when the length is
/// zero.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Str {
    /// First byte. May be null or dangling when `len` is 0.
    pub ptr: *const u8,
    /// Length in bytes.
    pub len: usize,
}

impl Str {
    /// A view of text this program already holds.
    ///
    /// Borrowed, so whatever owns the text must outlive the view — which
    /// is free for a literal, and is the usual case in a library
    /// descriptor, where every string is a constant in the library image.
    pub const fn borrowed(text: &str) -> Str {
        Str {
            ptr: text.as_ptr(),
            len: text.len(),
        }
    }

    /// An empty view.
    pub const fn empty() -> Str {
        Str {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}

/// Owned, growable UTF-8 text.
///
/// `cap == 0` means the buffer is **not owned**: a literal or a borrow,
/// never freed, copied out of on the first growth.
#[repr(C)]
#[derive(Debug)]
pub struct Text {
    /// First byte. Never null for an owned buffer; dangling-but-aligned
    /// when the container is empty.
    pub ptr: *mut u8,
    /// Length in bytes.
    pub len: usize,
    /// Capacity in bytes. 0 means the buffer is not owned.
    pub cap: usize,
    /// The allocator that made this buffer, and the only one that may
    /// grow or free it. Null when `cap == 0`.
    pub alloc: *const Allocator,
}

impl Drop for Text {
    fn drop(&mut self) {
        // SAFETY: a `Text` describes its own storage, and `cap == 0`
        // — a literal, or one already released — frees nothing.
        unsafe { release_buffer(self) }
    }
}

impl Text {
    /// Text, copied onto Rust's heap.
    ///
    /// Unlike [`Map::new`](super::Map::new) this OWNS a buffer the moment it exists, which
    /// is why it can fail at all; it frees that buffer on drop, like every
    /// other container here.
    pub fn new(text: &str) -> Text {
        or_abort(Text::new_in(Alloc::rust(), text))
    }

    /// The same, through an allocator you name, reporting its refusal.
    pub fn new_in(alloc: Alloc, text: &str) -> Result<Text, ValueError> {
        Ok(owned_text(alloc, text)?)
    }

    /// The text itself, or `None` if it is not valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        if self.len == 0 {
            return Some("");
        }
        // SAFETY: the first `len` bytes are initialised.
        let bytes = unsafe { std::slice::from_raw_parts(self.ptr, self.len) };
        std::str::from_utf8(bytes).ok()
    }
}

/// Empty, growing through Rust's allocator: [`Text::new`] of `""`.
///
/// An empty container owns nothing, so this allocates nothing and cannot
/// fail.
impl Default for Text {
    fn default() -> Text {
        Text::new("")
    }
}

impl From<Text> for Value {
    /// A string value. A NUMBER also stores its digits in a [`Text`], so
    /// that one is spelled [`Value::number`] rather than reached by
    /// conversion — the grammar has to be checked, and a conversion that
    /// cannot refuse is the wrong place to check it.
    fn from(text: Text) -> Value {
        let mut v = blank(Tag::GUATIAO_STRING);
        v.payload = Payload {
            text: ManuallyDrop::new(text),
        };
        v
    }
}
