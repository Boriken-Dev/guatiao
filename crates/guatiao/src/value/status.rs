// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a call across the boundary reports.
//!
//! # Why this is not inside `exports`
//!
//! The `extern "C"` wrappers are behind a feature, because whichever
//! artifact wants to export them turns it on and nobody else pays. A
//! **status code** is not like that: a library's vtable slot returns one,
//! and a library that had to enable `c-exports` merely to name its own
//! return type would export this crate's whole mutation surface from its
//! own library as a side effect — which is a different artifact's job and
//! would put two copies of those symbols in one process for no reason.
//!
//! So the type lives here, ungated, and the wrappers that return it stay
//! behind the feature.

#![forbid(unsafe_code)]
#![allow(non_camel_case_types)]

use super::mutate::ValueError;

/// What an entry point reports.
///
/// The numbering is wider than the list because these values come from a
/// status space a host application may share with the crates it embeds, so
/// one `switch` in a consumer can cover both. A consumer with no such
/// space reads 0 as success and treats every other value as failure, which
/// is the whole contract.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The operation happened.
    GUATIAO_OK = 0,
    /// A value could not be stored: text offered as a number that is not
    /// one, or bytes offered as text that are not UTF-8. Nothing was
    /// written.
    GUATIAO_ERR_BAD_VALUE = 7,
    /// The allocator could not satisfy the request, or was unusable.
    /// Nothing was written.
    GUATIAO_ERR_ALLOC = 8,
    /// The operation does not apply to this value's kind.
    GUATIAO_ERR_WRONG_KIND = 9,
    /// A key or index was not there.
    GUATIAO_ERR_NOT_FOUND = 10,
    /// A required pointer was null.
    GUATIAO_ERR_NULL = 11,
    /// A callback unwound, or a panic was caught at this boundary. The
    /// operation did not happen; the process is still usable.
    GUATIAO_ERR_INTERNAL = 100,
}

impl From<ValueError> for Status {
    fn from(e: ValueError) -> Status {
        match e {
            ValueError::Alloc(_) => Status::GUATIAO_ERR_ALLOC,
            ValueError::NotANumber | ValueError::NotUtf8 => Status::GUATIAO_ERR_BAD_VALUE,
            ValueError::WrongKind | ValueError::UnknownTag(_) => Status::GUATIAO_ERR_WRONG_KIND,
            ValueError::OutOfRange => Status::GUATIAO_ERR_NOT_FOUND,
            ValueError::TooDeep => Status::GUATIAO_ERR_BAD_VALUE,
        }
    }
}
