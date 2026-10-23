// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Kinds`]: which of the seven stored kinds a reader accepts.

#![forbid(unsafe_code)]

use alloc::fmt;

use crate::value::types::Tag;

/// A set of the kinds a value can be stored as: null, boolean, number,
/// string, bytes, list and map.
///
/// What [`FromValue::KINDS`](super::FromValue::KINDS) answers, so an enum
/// whose variants are told apart by kind (`#[map(untagged)]`) can check
/// at compile time that no two variants read the same kind, and pick a
/// variant by the kind it is handed. Every method is `const`.
///
/// ```
/// use guatiao::{FromValue, Kinds, Tag};
///
/// assert_eq!(<u16 as FromValue>::KINDS, Kinds::NUMBER);
/// assert!(<Option<String> as FromValue>::KINDS.contains(Tag::GUATIAO_NULL));
/// assert!(!Kinds::STRING.intersects(Kinds::MAP));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Kinds(u8);

/// The kinds in tag order, each with its bit and how a sentence names it.
const NAMED: [(Kinds, &str); 7] = [
    (Kinds::NULL, "null"),
    (Kinds::BOOL, "a boolean"),
    (Kinds::NUMBER, "a number"),
    (Kinds::STRING, "a string"),
    (Kinds::BYTES, "bytes"),
    (Kinds::LIST, "a list"),
    (Kinds::MAP, "a map"),
];

/// cbindgen:ignore
impl Kinds {
    /// No kind at all.
    pub const NONE: Kinds = Kinds(0);
    /// `GUATIAO_NULL`.
    pub const NULL: Kinds = Kinds(1 << 1);
    /// `GUATIAO_BOOL`.
    pub const BOOL: Kinds = Kinds(1 << 2);
    /// `GUATIAO_NUMBER`.
    pub const NUMBER: Kinds = Kinds(1 << 3);
    /// `GUATIAO_STRING`.
    pub const STRING: Kinds = Kinds(1 << 4);
    /// `GUATIAO_BYTES`.
    pub const BYTES: Kinds = Kinds(1 << 5);
    /// `GUATIAO_LIST`.
    pub const LIST: Kinds = Kinds(1 << 6);
    /// `GUATIAO_MAP`.
    pub const MAP: Kinds = Kinds(1 << 7);
    /// Every stored kind: what a reader that says nothing is assumed to
    /// accept.
    pub const ANY: Kinds = Kinds(0b1111_1110);

    /// The kind `tag` names; [`NONE`](Kinds::NONE) for `GUATIAO_ABSENT`,
    /// which is never stored.
    pub const fn of(tag: Tag) -> Kinds {
        match tag {
            Tag::GUATIAO_ABSENT => Kinds::NONE,
            Tag::GUATIAO_NULL => Kinds::NULL,
            Tag::GUATIAO_BOOL => Kinds::BOOL,
            Tag::GUATIAO_NUMBER => Kinds::NUMBER,
            Tag::GUATIAO_STRING => Kinds::STRING,
            Tag::GUATIAO_BYTES => Kinds::BYTES,
            Tag::GUATIAO_LIST => Kinds::LIST,
            Tag::GUATIAO_MAP => Kinds::MAP,
        }
    }

    /// Every kind in either set.
    #[must_use]
    pub const fn union(self, other: Kinds) -> Kinds {
        Kinds(self.0 | other.0)
    }

    /// Whether `tag`'s kind is in the set.
    pub const fn contains(self, tag: Tag) -> bool {
        self.intersects(Kinds::of(tag))
    }

    /// Whether the two sets share a kind.
    pub const fn intersects(self, other: Kinds) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether the set holds no kind.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The kinds by name, `Kinds(a string | a map)`.
impl fmt::Debug for Kinds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Kinds(")?;
        let mut first = true;
        for (kind, name) in NAMED {
            if self.intersects(kind) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

/// A phrase for a person: `a string, a list or a map`; `nothing` when
/// empty.
impl fmt::Display for Kinds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: alloc::vec::Vec<&str> = NAMED
            .iter()
            .filter(|(kind, _)| self.intersects(*kind))
            .map(|(_, name)| *name)
            .collect();
        match names.split_last() {
            None => f.write_str("nothing"),
            Some((last, [])) => f.write_str(last),
            Some((last, rest)) => write!(f, "{} or {last}", rest.join(", ")),
        }
    }
}
