// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! UTF-8 text: the borrowed view and the owned container.

#![allow(missing_docs)]

use std::marker::PhantomData;
use std::mem::ManuallyDrop;

use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::raw::{overlaps, release_buffer, reserve};

use super::{Payload, Tag, Value, or_abort};

/// Borrowed UTF-8 text: a pointer and a length, no NUL terminator. Text
/// that is not UTF-8 is refused where it crosses into Rust.
///
/// **Check `len` before `ptr`**: an empty view may carry a dangling or
/// null pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Str<'a> {
    /// First byte. May be null or dangling when `len` is 0.
    ptr: *const u8,
    /// Length in bytes.
    len: usize,
    /// What the view borrows, for the compiler; nothing in C.
    _borrows: PhantomData<&'a str>,
}

impl<'a> Str<'a> {
    /// A view of text this program holds, for as long as it holds it.
    ///
    /// It borrows what it views, so it cannot outlive it:
    ///
    /// ```compile_fail
    /// let view = {
    ///     let owned = String::from("host");
    ///     guatiao::Str::new(&owned)
    /// };
    /// assert_eq!(view.len(), 4);
    /// ```
    pub const fn new(text: &'a str) -> Str<'a> {
        Str {
            ptr: text.as_ptr(),
            len: text.len(),
            _borrows: PhantomData,
        }
    }

    /// An empty view.
    pub const fn empty() -> Str<'a> {
        Str {
            ptr: std::ptr::null(),
            len: 0,
            _borrows: PhantomData,
        }
    }

    /// The view `ptr` points at, its text checked: how a view C hands over
    /// by pointer is read, as `CStr::from_ptr` reads a C string. Null is
    /// an empty view.
    ///
    /// # Safety
    ///
    /// `ptr` is null, or points at a view whose bytes stay readable for
    /// `'a`. Only the memory is promised: the text is checked.
    ///
    /// ```
    /// # use guatiao::{Str, ValueError};
    /// let from_c = Str::new("host");
    /// // SAFETY: a live view, over a literal.
    /// let view = unsafe { Str::from_ptr(&from_c) }?;
    /// assert_eq!(<&str>::from(view), "host");
    /// assert_eq!(unsafe { Str::from_ptr(std::ptr::null()) }?.len(), 0);
    /// # Ok::<(), ValueError>(())
    /// ```
    pub const unsafe fn from_ptr(ptr: *const Str<'a>) -> Result<Str<'a>, ValueError> {
        if ptr.is_null() {
            return Ok(Str::empty());
        }
        // SAFETY: the caller's contract.
        let view = unsafe { ptr.read() };
        match std::str::from_utf8(view.items()) {
            Ok(_) => Ok(view),
            Err(_) => Err(ValueError::NotUtf8),
        }
    }

    /// A view described by hand, trusted as `String::from_raw_parts`
    /// trusts: nothing is checked.
    ///
    /// # Safety
    ///
    /// `len` is 0, or `ptr` addresses `len` bytes of UTF-8 that stay
    /// readable and unchanged for `'a`.
    pub const unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Str<'a> {
        Str {
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
    const fn items(self) -> &'a [u8] {
        if self.len == 0 || self.ptr.is_null() {
            return &[];
        }
        // SAFETY: the view was made from a borrow of `'a`, or by
        // `from_raw_parts` or a foreign caller promising the same.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<'a> From<&'a str> for Str<'a> {
    fn from(text: &'a str) -> Str<'a> {
        Str::new(text)
    }
}

/// The bytes, claiming nothing about them: what `str::from_utf8` checks
/// when a view came from C, whose memory the caller vouched for and whose
/// text nobody has.
///
/// ```
/// # use guatiao::Str;
/// let from_c = Str::new("host"); // as an export receives it
/// let checked = std::str::from_utf8(from_c.into())?;
/// assert_eq!(checked, "host");
/// # Ok::<(), std::str::Utf8Error>(())
/// ```
impl<'a> From<Str<'a>> for &'a [u8] {
    fn from(view: Str<'a>) -> &'a [u8] {
        view.items()
    }
}

/// The text: a `Str` is UTF-8 from the moment it is made.
impl<'a> From<Str<'a>> for &'a str {
    fn from(view: Str<'a>) -> &'a str {
        // SAFETY: `new` took a `str`, and the caller of `from_raw_parts`
        // promised.
        unsafe { std::str::from_utf8_unchecked(view.items()) }
    }
}

/// Owned, growable UTF-8 text. `cap == 0` means the buffer is **not
/// owned**: never freed, copied out of on the first growth.
///
/// UTF-8 is the contract, not a hope: a string or a map key whose bytes
/// are not UTF-8 is refused where it is read, and the node holding it
/// reads as no string, or no map.
#[repr(C)]
#[derive(Debug)]
pub struct Text {
    /// First byte. Dangling-but-aligned when the container is empty.
    pub(crate) ptr: *mut u8,
    /// Length in bytes.
    pub(crate) len: usize,
    /// Capacity in bytes. 0 means the buffer is not owned.
    pub(crate) cap: usize,
    /// The allocator that made this buffer and the only one that may
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
    /// A text over storage described by hand: the one door for a literal
    /// or a buffer another language owns.
    ///
    /// # Safety
    ///
    /// The four describe one consistent storage: the first `len` bytes
    /// are initialised and readable for as long as this lives; `cap > 0`
    /// means the block came from `alloc` with that layout and is this
    /// text's alone; `cap == 0` means the bytes are somebody else's and,
    /// if this text is mutated in place, writable. The bytes are UTF-8:
    /// a `Text` is read as a `str` without checking again.
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

    /// The four fields, with ownership: this no longer frees them.
    pub fn into_raw_parts(self) -> (*mut u8, usize, usize, *const Allocator) {
        let this = std::mem::ManuallyDrop::new(self);
        (this.ptr, this.len, this.cap, this.alloc)
    }

    /// How many bytes the storage holds before it must grow.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Text, copied onto Rust's heap. Unlike [`Map::new`](super::Map::new)
    /// this owns a buffer the moment it exists, which is why it can fail
    /// at all.
    pub fn new(text: &str) -> Text {
        or_abort(Text::new_in(Alloc::rust(), text))
    }

    /// The same, through an allocator you name. The buffer's storage
    /// **becomes** this text's: the fields are copied across and the
    /// `Buffer` is forgotten, so one allocation has one owner at every
    /// instant.
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

    /// Frees the storage and leaves this empty. Idempotent: `cap == 0`
    /// afterwards, which frees nothing.
    pub(crate) fn release(&mut self) {
        // SAFETY: this container describes its own storage.
        unsafe { release_buffer(self) }
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

    /// The same, adopting `alloc` for a buffer that carries none. `text`
    /// may address this text's own bytes, and an overlapping source is
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
        Text::new_in(alloc, self)
    }

    /// The text `ptr` points at, checked: how a text C hands over by
    /// pointer is read, as `CStr::from_ptr` reads a C string.
    ///
    /// # Safety
    ///
    /// `ptr` points at a text whose storage is consistent and outlives
    /// `'a`. Only the memory is promised: the text is checked.
    pub const unsafe fn from_ptr<'a>(ptr: *const Text) -> Result<&'a Text, ValueError> {
        // SAFETY: the caller's contract.
        let text = unsafe { &*ptr };
        match std::str::from_utf8(text.bytes()) {
            Ok(_) => Ok(text),
            Err(_) => Err(ValueError::NotUtf8),
        }
    }

    /// The stored bytes, before anything is known of them: what the doors
    /// check, and what freeing and comparing a foreign node read.
    pub(crate) const fn bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        // SAFETY: the first `len` bytes are initialised.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

/// The text, as a `String` gives its `str`.
impl std::ops::Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: a `Text` is checked once, when it is made: its
        // constructors take a `str`, and a foreign one is checked by the
        // door that reads it out of a value.
        unsafe { std::str::from_utf8_unchecked(self.bytes()) }
    }
}

/// The text to change in place, as `String` gives it: a `&mut str` can
/// only stay UTF-8. Bytes a text does not own (`cap == 0`) are written
/// where they are, which `from_raw_parts` asks of them, as `Buffer` does.
impl std::ops::DerefMut for Text {
    fn deref_mut(&mut self) -> &mut str {
        if self.len == 0 {
            return <&mut str>::default();
        }
        // SAFETY: the first `len` bytes are initialised and UTF-8, and
        // `&mut self` makes the borrow unique.
        unsafe {
            std::str::from_utf8_unchecked_mut(std::slice::from_raw_parts_mut(self.ptr, self.len))
        }
    }
}

/// For a bound, which does not deref, as `String` has it.
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsMut<str> for Text {
    fn as_mut(&mut self) -> &mut str {
        self
    }
}

/// A `HashMap<Text, _>` is searched with a `&str`: hash and equality are
/// the `str`'s.
impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
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
    /// A copy through the allocator this text recorded, or the crate's
    /// own. Panics as [`Value::clone`] does.
    fn clone(&self) -> Text {
        let alloc = Alloc::recorded_or_rust(self.alloc);
        self.clone_in(alloc)
            .expect("a text clones through a working allocator")
    }
}

impl AsRef<[u8]> for Text {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// As its `str`, as its equality is.
impl std::hash::Hash for Text {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

/// Appends, as `String`'s `Write` does, so `write!(text, "{x}")` works.
/// An allocator's refusal is `fmt::Error`, which carries no detail.
impl std::fmt::Write for Text {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.push_str(text).map_err(|_| std::fmt::Error)
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Text) -> bool {
        **self == **other
    }
}

impl Eq for Text {}

// SAFETY: the buffer is owned outright and reached only through `&self`
// or `&mut self`; the allocator it recorded outlives it and may be
// called from any thread, which is the contract on `Allocator`.
unsafe impl Send for Text {}
// SAFETY: as above.
unsafe impl Sync for Text {}

/// A copy onto Rust's heap: the text's storage belongs to its allocator.
impl From<Text> for String {
    fn from(text: Text) -> String {
        String::from(&*text)
    }
}

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
    /// A string value. A [`Number`](super::Number) stores its digits in
    /// a [`Text`] too and converts from `Number` instead: the grammar has
    /// to be checked, and a conversion that cannot refuse is the wrong
    /// place for it.
    fn from(text: Text) -> Value {
        let mut v = Value::blank(Tag::GUATIAO_STRING);
        v.payload = Payload::text(text);
        v
    }
}
