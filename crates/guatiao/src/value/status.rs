// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a call across the boundary reports.
//!
//! Here rather than in `exports` because a status is part of the value
//! model's vocabulary: a library's vtable slot returns one without going
//! near a wrapper, and the model must be able to name its own return type
//! without reaching into the layer above.

#![forbid(unsafe_code)]
#![allow(non_camel_case_types)]

use super::error::ValueError;

/// What an entry point reports.
///
/// The numbering is wider than the list because the space may be shared
/// with a host's own, so one `switch` covers both. The whole contract is
/// that 0 is success and everything else is failure.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The operation happened.
    GUATIAO_OK = 0,
    /// A value could not be stored: text that is not a number, or bytes
    /// that are not UTF-8. Nothing was written.
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
    /// The thing asked is gone: a freed registry, asked through the
    /// services a library kept.
    GUATIAO_ERR_GONE = 12,
    /// What was asked is not something this side offers: a library with
    /// no `unload` slot, asked to unload.
    GUATIAO_ERR_UNSUPPORTED = 13,
    /// Not now: something is still in use. A library answers this from
    /// its `unload` slot while anything it handed out is alive.
    GUATIAO_ERR_BUSY = 14,
    /// A callback unwound, or a panic was caught here. The operation did
    /// not happen; the process is still usable.
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

impl From<std::str::Utf8Error> for Status {
    fn from(e: std::str::Utf8Error) -> Status {
        Status::from(ValueError::from(e))
    }
}
