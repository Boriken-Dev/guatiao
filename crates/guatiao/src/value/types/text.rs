// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! UTF-8 text: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::mem::ManuallyDrop;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::raw::{overlaps, release_buffer, reserve};

use super::{Payload, Tag, Value, or_abort};

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
    pub(crate) ptr: *mut u8,
    /// Length in bytes.
    pub(crate) len: usize,
    /// Capacity in bytes. 0 means the buffer is not owned.
    pub(crate) cap: usize,
    /// The allocator that made this buffer, and the only one that may
    /// grow or free it. Null when `cap == 0`.
    pub(crate) alloc: *const Allocator,
}

impl Drop for Text {
    fn drop(&mut self) {
        // SAFETY: a `Text` describes its own storage, and `cap == 0`
        // — a literal, or one already released — frees nothing.
        unsafe { release_buffer(self) }
    }
}

impl Text {
    /// A text over storage described by hand: `len` initialised UTF-8
    /// bytes at `ptr`, in a block of `cap` bytes from `alloc`, or a
    /// borrowed buffer when `cap` is 0.
    ///
    /// The one door for a literal or a buffer another language owns.
    /// Everything else builds through [`Text::new`].
    ///
    /// # Safety
    ///
    /// The four describe one consistent storage: the first `len` bytes are
    /// initialised and readable for as long as this lives; `cap > 0` means
    /// the block came from `alloc` with that layout and is owned by this
    /// text alone; `cap == 0` means the bytes are somebody else's and, if
    /// this text is mutated in place, writable.
    pub unsafe fn from_raw_parts(
        ptr: *mut u8,
        len: usize,
        cap: usize,
        alloc: *const Allocator,
    ) -> Text {
        Text {
            ptr,
            len,
            cap,
            alloc,
        }
    }

    /// The four fields, with ownership: this text no longer frees them.
    pub fn into_raw_parts(self) -> (*mut u8, usize, usize, *const Allocator) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.ptr, this.len, this.cap, this.alloc)
    }

    /// How many bytes the storage holds before it must grow. Zero for a
    /// buffer this text does not own.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Text, copied onto Rust's heap.
    ///
    /// Unlike [`Map::new`](super::Map::new) this OWNS a buffer the moment it exists, which
    /// is why it can fail at all; it frees that buffer on drop, like every
    /// other container here.
    pub fn new(text: &str) -> Text {
        or_abort(Text::new_in(Alloc::rust(), text))
    }

    /// The same, through an allocator you name, reporting its refusal.
    ///
    /// The buffer's storage **becomes** this text's: the four fields are
    /// copied across and the `Buffer` is forgotten, so one allocation has
    /// one owner at every instant.
    pub fn new_in(alloc: Alloc, text: &str) -> Result<Text, ValueError> {
        let b = ManuallyDrop::new(super::Buffer::new_in(alloc, text.as_bytes())?);
        let (ptr, len, cap, alloc) = (b.ptr, b.len, b.cap, b.alloc);
        Ok(Text {
            ptr,
            len,
            cap,
            alloc,
        })
    }

    /// The allocator this text grows through.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        // SAFETY: the address a container recorded is an allocator that
        // outlives it, by the contract on `Alloc`.
        Ok(unsafe { Alloc::from_raw(self.alloc) }?)
    }

    /// Appends `text`, growing through the allocator this one recorded.
    pub fn push_str(&mut self, text: &str) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        self.push_str_in(text, alloc)
    }

    /// The same, adopting `alloc` for a buffer that carries none.
    ///
    /// `text` may address this text's own bytes; an overlapping source is
    /// copied out before anything grows.
    pub fn push_str_in(&mut self, text: &str, alloc: Alloc) -> Result<(), ValueError> {
        if text.is_empty() {
            return Ok(());
        }
        // Growth frees the buffer the source may point into, and staying
        // in place would make the copy below overlap itself.
        let staged;
        let bytes = if overlaps(self.ptr, self.len, text.as_ptr(), text.len()) {
            staged = text.as_bytes().to_vec();
            staged.as_slice()
        } else {
            text.as_bytes()
        };
        // SAFETY: the container is consistent.
        unsafe { reserve(self, bytes.len(), Some(alloc))? };
        // SAFETY: `reserve` guaranteed room for `bytes.len()` more past
        // `len`, and the source is disjoint from the buffer or a staged
        // copy of it.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.add(self.len), bytes.len())
        };
        self.len += bytes.len();
        Ok(())
    }

    /// A copy through `alloc`, reporting its refusal.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Text, ValueError> {
        Text::new_in(alloc, self.as_str().ok_or(ValueError::NotUtf8)?)
    }

    /// The bytes, whether or not they are valid UTF-8.
    pub fn as_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` bytes are initialised.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
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

impl Clone for Text {
    /// A copy through the allocator this text recorded, or the crate's own
    /// when it has none. Panics as [`Value::clone`] does, and on text that
    /// is not UTF-8 — a foreign producer's, since nothing here writes one.
    fn clone(&self) -> Text {
        let alloc = Alloc::recorded_or_rust(self.alloc);
        self.clone_in(alloc)
            .expect("a text clones through a working allocator")
    }
}

impl PartialEq for Text {
    /// The BYTES, not the `&str`: text a foreign producer wrote that is
    /// not UTF-8 reads back as `None`, and two different such texts would
    /// then compare equal on the strength of both being unreadable.
    fn eq(&self, other: &Text) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Text {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`, so no two threads share it without the borrow checker
// saying so; the allocator it recorded is a table that outlives it, by the contract on `Alloc`,
// and may be called from any thread, which is the contract on
// `Allocator` — a host handing out an arena synchronises it, as Rust's
// global allocator does.
unsafe impl Send for Text {}
// SAFETY: as above.
unsafe impl Sync for Text {}

impl From<&str> for Text {
    fn from(text: &str) -> Text {
        Text::new(text)
    }
}

impl From<String> for Text {
    fn from(text: String) -> Text {
        Text::new(&text)
    }
}

impl From<&String> for Text {
    fn from(text: &String) -> Text {
        Text::new(text)
    }
}

impl From<Text> for Value {
    /// A string value. A [`Number`](super::Number) stores its digits in a
    /// [`Text`] too, so that one converts from `Number` rather than from
    /// this: the grammar has to be checked, and a conversion that cannot
    /// refuse is the wrong place to check it.
    fn from(text: Text) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_STRING);
        v.payload = Payload::text(text);
        v
    }
}
