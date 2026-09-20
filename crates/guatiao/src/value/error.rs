// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Why a write was refused, and how deep any walk follows.
//!
//! A write fails because the allocator refused or the argument was not a
//! value this model can hold. Reading fails for a different question
//! entirely and reports [`MapError`](super::convert::MapError).

#![forbid(unsafe_code)]

use super::alloc::AllocError;

/// How deep a tree any walk here follows: cloning one, merging two,
/// comparing two for equality, and checking one against a schema.
///
/// Configuration trees are a handful of levels deep; this is far above any
/// real one and far below what would exhaust a stack. It exists so a
/// hostile or corrupt tree is an error rather than a dead process: a stack
/// overflow on Windows is not catchable and takes the host with it.
pub const MAX_DEPTH: u32 = 128;

/// Why a mutation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueError {
    /// The allocator could not satisfy the request, or was itself unusable.
    Alloc(AllocError),
    /// Text offered as a number did not match the JSON number grammar
    /// (RFC 8259 section 6). See [`Number::new`](crate::Number::new).
    NotANumber,
    /// A key, or the text of a string value, was not valid UTF-8.
    ///
    /// Refused at the point it is offered rather than accepted and fixed
    /// up later: a lossy conversion does not fail, it **renames the key**,
    /// producing a map that is quietly not the one it came from.
    NotUtf8,
    /// The operation does not apply to this value's kind — pushing to
    /// something that is not a list, say.
    WrongKind,
    /// The tag is not one this build knows. Skip this value; do not stop.
    UnknownTag(u32),
    /// An index was past the end.
    OutOfRange,
    /// The tree is nested deeper than [`MAX_DEPTH`].
    TooDeep,
}

impl From<AllocError> for ValueError {
    fn from(e: AllocError) -> ValueError {
        ValueError::Alloc(e)
    }
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueError::Alloc(e) => write!(f, "{e}"),
            ValueError::NotANumber => f.write_str("not a JSON number (RFC 8259 section 6)"),
            ValueError::NotUtf8 => f.write_str("not valid UTF-8"),
            ValueError::WrongKind => f.write_str("the value is not of the kind this needs"),
            ValueError::UnknownTag(t) => write!(f, "tag {t} is not one this build knows"),
            ValueError::OutOfRange => f.write_str("the index is past the end"),
            ValueError::TooDeep => write!(f, "nested deeper than {MAX_DEPTH}"),
        }
    }
}

impl std::error::Error for ValueError {}
