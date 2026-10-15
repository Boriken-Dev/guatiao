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

use core::fmt;

use super::error::ValueError;

/// What an entry point reports: 0 is success and everything else is
/// failure.
///
/// An integer, never an enum: the space may be shared with a host's own
/// codes, so one `switch` covers both, and a code nobody here defines is
/// still a status. The codes this crate defines are `GUATIAO_OK` and the
/// `GUATIAO_ERR_*` constants.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status(u32);

/// The codes this crate defines for a `guatiao_status`, as named
/// constants. No slot, field or return value has this type: each is a
/// `guatiao_status`, so a code outside this list is still a value of it.
#[repr(u32)]
enum StatusCode {
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

/// cbindgen:ignore
impl Status {
    /// The operation happened.
    pub const GUATIAO_OK: Status = Status(StatusCode::GUATIAO_OK as u32);
    /// A value could not be stored: text that is not a number, or bytes
    /// that are not UTF-8. Nothing was written.
    pub const GUATIAO_ERR_BAD_VALUE: Status = Status(StatusCode::GUATIAO_ERR_BAD_VALUE as u32);
    /// The allocator could not satisfy the request, or was unusable.
    /// Nothing was written.
    pub const GUATIAO_ERR_ALLOC: Status = Status(StatusCode::GUATIAO_ERR_ALLOC as u32);
    /// The operation does not apply to this value's kind.
    pub const GUATIAO_ERR_WRONG_KIND: Status = Status(StatusCode::GUATIAO_ERR_WRONG_KIND as u32);
    /// A key or index was not there.
    pub const GUATIAO_ERR_NOT_FOUND: Status = Status(StatusCode::GUATIAO_ERR_NOT_FOUND as u32);
    /// A required pointer was null.
    pub const GUATIAO_ERR_NULL: Status = Status(StatusCode::GUATIAO_ERR_NULL as u32);
    /// The thing asked is gone: a freed registry, asked through the
    /// services a library kept.
    pub const GUATIAO_ERR_GONE: Status = Status(StatusCode::GUATIAO_ERR_GONE as u32);
    /// What was asked is not something this side offers: a library with
    /// no `unload` slot, asked to unload.
    pub const GUATIAO_ERR_UNSUPPORTED: Status = Status(StatusCode::GUATIAO_ERR_UNSUPPORTED as u32);
    /// Not now: something is still in use. A library answers this from
    /// its `unload` slot while anything it handed out is alive.
    pub const GUATIAO_ERR_BUSY: Status = Status(StatusCode::GUATIAO_ERR_BUSY as u32);
    /// A callback unwound, or a panic was caught here. The operation did
    /// not happen; the process is still usable.
    pub const GUATIAO_ERR_INTERNAL: Status = Status(StatusCode::GUATIAO_ERR_INTERNAL as u32);

    /// Whether this is [`Status::GUATIAO_OK`].
    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }

    /// The name of a code this crate defines, or `None` for any other.
    fn name(self) -> Option<&'static str> {
        Some(match self {
            Status::GUATIAO_OK => "GUATIAO_OK",
            Status::GUATIAO_ERR_BAD_VALUE => "GUATIAO_ERR_BAD_VALUE",
            Status::GUATIAO_ERR_ALLOC => "GUATIAO_ERR_ALLOC",
            Status::GUATIAO_ERR_WRONG_KIND => "GUATIAO_ERR_WRONG_KIND",
            Status::GUATIAO_ERR_NOT_FOUND => "GUATIAO_ERR_NOT_FOUND",
            Status::GUATIAO_ERR_NULL => "GUATIAO_ERR_NULL",
            Status::GUATIAO_ERR_GONE => "GUATIAO_ERR_GONE",
            Status::GUATIAO_ERR_UNSUPPORTED => "GUATIAO_ERR_UNSUPPORTED",
            Status::GUATIAO_ERR_BUSY => "GUATIAO_ERR_BUSY",
            Status::GUATIAO_ERR_INTERNAL => "GUATIAO_ERR_INTERNAL",
            _ => return None,
        })
    }
}

impl From<u32> for Status {
    fn from(code: u32) -> Status {
        Status(code)
    }
}

impl From<Status> for u32 {
    fn from(status: Status) -> u32 {
        status.0
    }
}

/// The constant's name for a code this crate defines, `Status(n)` for
/// any other.
impl fmt::Debug for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "Status({})", self.0),
        }
    }
}

/// What happened, as one lowercase clause; a code this crate does not
/// define displays as its number.
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Status::GUATIAO_OK => "ok",
            Status::GUATIAO_ERR_BAD_VALUE => "a value could not be stored",
            Status::GUATIAO_ERR_ALLOC => "the allocator refused",
            Status::GUATIAO_ERR_WRONG_KIND => "the operation does not apply to this kind",
            Status::GUATIAO_ERR_NOT_FOUND => "not found",
            Status::GUATIAO_ERR_NULL => "a required pointer was null",
            Status::GUATIAO_ERR_GONE => "gone",
            Status::GUATIAO_ERR_UNSUPPORTED => "unsupported",
            Status::GUATIAO_ERR_BUSY => "busy",
            Status::GUATIAO_ERR_INTERNAL => "internal error",
            Status(code) => return write!(f, "status {code}"),
        })
    }
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

impl From<crate::value::alloc::AllocError> for Status {
    fn from(e: crate::value::alloc::AllocError) -> Status {
        Status::from(ValueError::Alloc(e))
    }
}

impl From<core::str::Utf8Error> for Status {
    fn from(e: core::str::Utf8Error) -> Status {
        Status::from(ValueError::from(e))
    }
}

#[cfg(test)]
mod tests {
    use super::Status;

    #[test]
    fn any_code_is_a_status() {
        let foreign = Status::from(1);
        assert!(!foreign.is_ok());
        assert_ne!(foreign, Status::GUATIAO_OK);
        assert_eq!(u32::from(foreign), 1);
        assert_eq!(alloc::format!("{foreign}"), "status 1");
        assert_eq!(alloc::format!("{foreign:?}"), "Status(1)");
        assert!(Status::GUATIAO_OK.is_ok());
        assert_eq!(
            alloc::format!("{:?}", Status::GUATIAO_ERR_NULL),
            "GUATIAO_ERR_NULL"
        );
    }
}
