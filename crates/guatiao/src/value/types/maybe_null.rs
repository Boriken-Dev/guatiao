// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A pointer whose null is meaningful.

#![allow(missing_docs)]

use std::fmt;

/// A `*const T` where **null is a value, not a mistake**.
///
/// `repr(transparent)`, so it crosses as a plain `const T *` and costs a
/// C caller nothing. What it buys is on the Rust side: a bare `*const T`
/// says nothing about whether null is expected, so every reader
/// re-decides and one eventually decides wrong.
///
/// **Non-null means a well-formed `T` that outlives the read**, and
/// nothing checks it — the same class of promise as a vtable pointer.
#[repr(transparent)]
pub struct MaybeNull<T>(*const T);

impl<T> MaybeNull<T> {
    /// Nothing here.
    pub const fn null() -> MaybeNull<T> {
        MaybeNull(std::ptr::null())
    }

    /// A pointer to something that outlives the reader.
    pub const fn of(value: &'static T) -> MaybeNull<T> {
        MaybeNull(value as *const T)
    }

    /// Whether it points at anything: the only question answerable
    /// without the contract above.
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    /// The raw pointer, for a caller that has its own rules.
    pub fn as_ptr(self) -> *const T {
        self.0
    }

    /// What it points at, or `None`.
    ///
    /// # Safety
    ///
    /// If non-null, it addresses a live, well-formed `T` valid for `'a`.
    /// For a descriptor read out of a loaded library that holds because
    /// the library is never unloaded.
    pub unsafe fn get<'a>(self) -> Option<&'a T> {
        // SAFETY: the caller states a non-null pointer is a live `T`.
        unsafe { self.0.as_ref() }
    }
}

// Written out: `derive` would demand `T: Copy` and `T: Debug`, and a
// pointer needs neither of its pointee.
impl<T> Clone for MaybeNull<T> {
    fn clone(&self) -> MaybeNull<T> {
        *self
    }
}

impl<T> Copy for MaybeNull<T> {}

impl<T> Default for MaybeNull<T> {
    fn default() -> MaybeNull<T> {
        MaybeNull::null()
    }
}

impl<T> fmt::Debug for MaybeNull<T> {
    /// The address, and nothing followed: following it would fault on
    /// exactly the malformed pointer a person is looking at.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_null() {
            f.write_str("null")
        } else {
            write!(f, "{:p}", self.0)
        }
    }
}
