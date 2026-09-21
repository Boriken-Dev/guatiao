// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bytes: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::marker::PhantomData;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::raw::{dangling, overlaps, release_buffer, reserve};

use super::{Payload, Tag, Value, or_abort};

/// Borrowed bytes: any content at all, NULs included.
///
/// Same shape and check-the-length rule as `guatiao_str`, and a separate
/// type because "text" and "arbitrary bytes" are different promises.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Bytes<'a> {
    /// First byte. May be null or dangling when `len` is 0.
    ptr: *const u8,
    /// Length in bytes.
    len: usize,
    /// What the view borrows, for the compiler; nothing in C.
    _borrows: PhantomData<&'a [u8]>,
}

impl<'a> Bytes<'a> {
    /// A view of bytes this program holds, for as long as it holds it.
    pub const fn new(items: &'a [u8]) -> Bytes<'a> {
        Bytes {
            ptr: items.as_ptr(),
            len: items.len(),
            _borrows: PhantomData,
        }
    }

    /// An empty view.
    pub const fn empty() -> Bytes<'a> {
        Bytes {
            ptr: std::ptr::null(),
            len: 0,
            _borrows: PhantomData,
        }
    }

    /// A view described by hand.
    ///
    /// # Safety
    ///
    /// `len` is 0, or `ptr` addresses `len` initialised elements that stay
    /// readable and unchanged for `'a`.
    pub const unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Bytes<'a> {
        Bytes {
            ptr,
            len,
            _borrows: PhantomData,
        }
    }

    /// The first element. May be null or dangling when `len` is 0.
    pub const fn ptr(&self) -> *const u8 {
        self.ptr
    }

    /// How many elements.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether it views nothing.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The elements, whatever they hold. A null pointer views nothing.
    fn items(self) -> &'a [u8] {
        if self.len == 0 || self.ptr.is_null() {
            return &[];
        }
        // SAFETY: the view was made from a borrow of `'a`, or by
        // `from_raw_parts` or a foreign caller promising the same.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<'a> From<&'a [u8]> for Bytes<'a> {
    fn from(items: &'a [u8]) -> Bytes<'a> {
        Bytes::new(items)
    }
}

/// The bytes: any content is bytes.
impl<'a> From<Bytes<'a>> for &'a [u8] {
    fn from(view: Bytes<'a>) -> &'a [u8] {
        view.items()
    }
}

/// Owned, growable bytes. See `guatiao_string` for the `cap` rule.
#[repr(C)]
#[derive(Debug)]
pub struct Buffer {
    /// First byte.
    pub(crate) ptr: *mut u8,
    /// Length in bytes.
    pub(crate) len: usize,
    /// Capacity in bytes. 0 means the buffer is not owned.
    pub(crate) cap: usize,
    /// The allocator that made this buffer. Null when `cap == 0`.
    pub(crate) alloc: *const Allocator,
}

impl Clone for Buffer {
    /// A copy through the allocator this buffer recorded, or the crate's
    /// own when it has none. Panics as [`Value::clone`] does.
    fn clone(&self) -> Buffer {
        let alloc = Alloc::recorded_or_rust(self.alloc);
        self.clone_in(alloc)
            .expect("a buffer clones through a working allocator")
    }
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Buffer) -> bool {
        **self == **other
    }
}

impl Eq for Buffer {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`; the allocator it recorded outlives it and may be
// called from any thread, which is the contract on `Allocator`.
unsafe impl Send for Buffer {}
// SAFETY: as above.
unsafe impl Sync for Buffer {}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: as for `Text`.
        unsafe { release_buffer(self) }
    }
}

impl Buffer {
    /// A buffer over storage described by hand. See
    /// [`Text::from_raw_parts`](super::Text::from_raw_parts), which this
    /// is for bytes.
    ///
    /// # Safety
    ///
    /// As `Text::from_raw_parts`, without the UTF-8 requirement.
    pub unsafe fn from_raw_parts(
        ptr: *mut u8,
        len: usize,
        cap: usize,
        alloc: *const Allocator,
    ) -> Buffer {
        Buffer {
            ptr,
            len,
            cap,
            alloc,
        }
    }

    /// The four fields, with ownership: this buffer no longer frees them.
    pub fn into_raw_parts(self) -> (*mut u8, usize, usize, *const Allocator) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.ptr, this.len, this.cap, this.alloc)
    }

    /// How many bytes the storage holds before it must grow. Zero for a
    /// buffer this does not own.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Bytes, copied onto Rust's heap. See [`Text::new`](super::Text::new).
    pub fn new(bytes: &[u8]) -> Buffer {
        or_abort(Buffer::new_in(Alloc::rust(), bytes))
    }

    /// The same, through an allocator you name.
    pub fn new_in(alloc: Alloc, bytes: &[u8]) -> Result<Buffer, ValueError> {
        let mut b = Buffer {
            ptr: dangling::<u8>(),
            len: 0,
            cap: 0,
            alloc: alloc.as_raw(),
        };
        if !bytes.is_empty() {
            // SAFETY: the container is consistent -- empty, owning nothing.
            unsafe { reserve(&mut b, bytes.len(), Some(alloc))? };
            // SAFETY: `reserve` guaranteed room for `bytes.len()` elements
            // at `b.ptr`, and the two regions cannot overlap since one was
            // just allocated.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), b.ptr, bytes.len()) };
            b.len = bytes.len();
        }
        Ok(b)
    }

    /// Frees the storage and leaves this empty. Idempotent: `cap == 0`
    /// afterwards, which frees nothing.
    pub(crate) fn release(&mut self) {
        // SAFETY: this container describes its own storage.
        unsafe { release_buffer(self) }
    }

    /// The allocator this buffer grows through.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        // SAFETY: the address a container recorded is an allocator that
        // outlives it, by the contract on `Alloc`.
        Ok(unsafe { Alloc::from_raw(self.alloc) }?)
    }

    /// Appends `bytes`, growing through the allocator this one recorded.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), ValueError> {
        let alloc = self.alloc()?;
        self.push_in(bytes, alloc)
    }

    /// The same, adopting `alloc` for a buffer that carries none.
    /// `bytes` may address this buffer's own storage, and an overlapping
    /// source is copied out before anything grows.
    pub fn push_in(&mut self, bytes: &[u8], alloc: Alloc) -> Result<(), ValueError> {
        if bytes.is_empty() {
            return Ok(());
        }
        let staged;
        let bytes = if overlaps(self.ptr, self.len, bytes.as_ptr(), bytes.len()) {
            staged = bytes.to_vec();
            staged.as_slice()
        } else {
            bytes
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
    pub fn clone_in(&self, alloc: Alloc) -> Result<Buffer, ValueError> {
        Buffer::new_in(alloc, self)
    }
}

/// The bytes, as `Vec<u8>` gives them.
impl std::ops::Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` bytes are initialised, and the borrow of
        // `self` keeps them alive.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

/// The bytes, mutably. A slice cannot change its length.
impl std::ops::DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        if self.len == 0 {
            return &mut [];
        }
        // SAFETY: as for `deref`, and `&mut self` makes the borrow unique.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

/// A `HashMap<Buffer, _>` is searched with a `&[u8]`: hash and equality
/// are the slice's.
impl std::borrow::Borrow<[u8]> for Buffer {
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for Buffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

/// By bytes, as its equality is.
impl std::hash::Hash for Buffer {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

/// Appends, as `Vec<u8>`'s `Write` does. An allocator's refusal is the
/// error.
impl std::io::Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.push(bytes).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Empty, growing through Rust's allocator. See [`Text::default`](super::Text).
impl Default for Buffer {
    fn default() -> Buffer {
        Buffer::new(&[])
    }
}

/// A copy onto Rust's heap: the buffer's storage belongs to its allocator.
impl From<Buffer> for Vec<u8> {
    fn from(bytes: Buffer) -> Vec<u8> {
        bytes.to_vec()
    }
}

impl From<&[u8]> for Buffer {
    fn from(bytes: &[u8]) -> Buffer {
        Buffer::new(bytes)
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(bytes: Vec<u8>) -> Buffer {
        Buffer::new(&bytes)
    }
}

impl From<Buffer> for Value {
    fn from(bytes: Buffer) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_BYTES);
        v.payload = Payload::bytes(bytes);
        v
    }
}
