// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Converting a Rust type to and from a value.
//!
//! [`ToValue`] and [`FromValue`], both written by the derive. Writing
//! takes an allocator because building a value allocates one and the
//! allocator travels with the tree; reading copies into ordinary Rust
//! types and needs nothing.
//!
//! ```
//! use guatiao::Alloc;
//! use guatiao::value::convert::{FromValue, ToValue};
//!
//! let value = "10.0.0.1".to_value(Alloc::rust())?;
//! assert_eq!(String::from_value(&value)?, "10.0.0.1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! **Why a named trait beside `TryFrom`.** The orphan rule refuses
//! `impl<T: FromValue> TryFrom<&Value> for T`, and refuses `Vec<T>` and
//! `Option<T>` too, so a generic reader needs a trait of this crate's
//! own. Writing has no `From` to be, since it needs an allocator. And
//! `TryFrom` for a user's own type is theirs to write.
//!
//! **The two directions fail for different questions**: a refused write
//! is a [`ValueError`], a read that found no key or the wrong kind is a
//! [`MapError`]. [`MapError`] keeps its three cases apart because a
//! typo'd key and a type mismatch have opposite fixes, and builds its
//! path on the way **out**, because a leaf does not know what it was
//! stored under.
//!
//! **`Option<T>` collapses absent and null.** Writing omits the key;
//! reading takes either. The asymmetry is what makes the round trip
//! total. A reader that must tell them apart uses
//! [`Map::get`](crate::Map::get), which answers `None` only for an absent
//! key.
//!
//! **`Vec<T>` is a list for every `T`, `Vec<u8>` included**; a field
//! that must cross as the bytes kind wears [`Bytes`]. Coherence refuses
//! both at once, and the sequence is the common case.
//!
//! **`try_into` covers** every scalar and the borrowed forms `&str`,
//! `&[u8]`, `&[Value]`, `&[Entry]`, `&Map` and `&List`. `Vec<T>` and
//! `Option<T>` read through [`FromValue`] instead. A lookup answers an
//! `Option`, which has no `TryFrom` either: [`Map::required`] is the step
//! from one to the other, and it names the key.
//!
//! [`ToValue`] is implemented for [`Value`], so a subtree in hand can be
//! written -- it deep-copies into the target allocator. There is no
//! `FromValue` for one, because reading it back would have to allocate
//! and [`FromValue`] takes no allocator.

use std::fmt;

use super::alloc::Alloc;
use super::error::ValueError;
use super::types::{Buffer, Entry, List, Map, Number, Tag, Text, Value};

/// Why a value could not be read as a particular Rust type.
///
/// Each names the key it is about, as a dotted path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MapError {
    /// Nothing is stored under this key, and the field is required.
    MissingKey {
        /// The dotted path of the key that was not found.
        key: String,
    },
    /// A value is stored, but it is the wrong kind entirely.
    WrongType {
        /// The dotted path, empty for a value passed in directly.
        key: String,
        /// The kind the field needed.
        expected: Tag,
        /// The kind that was there, or `None` for a tag this build does
        /// not know -- a real answer, which is why this is not a bare
        /// tag.
        found: Option<Tag>,
    },
    /// The right kind, but not a value this field can hold: a number too
    /// large for its integer type, a spelling nobody declared. Separate
    /// from [`MapError::WrongType`] because the producer chose the right
    /// *kind* and the wrong *value*.
    BadValue {
        /// The dotted path of the key.
        key: String,
        /// What the field would have accepted, phrased for a person.
        expected: String,
    },
}

impl MapError {
    /// A missing key: the one error that names its key up front, because
    /// a lookup that found nothing is the only place that knows it.
    pub fn missing(key: &str) -> MapError {
        MapError::MissingKey {
            key: key.to_string(),
        }
    }

    /// A value of the wrong kind, reading what is actually there off the
    /// value itself.
    pub fn wrong_type(expected: Tag, found: &Value) -> MapError {
        MapError::WrongType {
            key: String::new(),
            expected,
            found: found.tag().ok(),
        }
    }

    /// The right kind, the wrong value.
    pub fn bad_value(expected: impl Into<String>) -> MapError {
        MapError::BadValue {
            key: String::new(),
            expected: expected.into(),
        }
    }

    /// The dotted path this error is about.
    pub fn key(&self) -> &str {
        match self {
            MapError::MissingKey { key }
            | MapError::WrongType { key, .. }
            | MapError::BadValue { key, .. } => key,
        }
    }

    /// Re-roots this error under a field, so an error raised inside a
    /// nested map reports `"tls.verify"` rather than `"verify"`.
    #[must_use]
    pub fn under(self, prefix: &str) -> MapError {
        self.rekey(&|key| {
            if key.is_empty() {
                prefix.to_string()
            } else if key.starts_with('[') {
                // An index is already punctuation: `tags[2]`, never
                // `tags.[2]`.
                format!("{prefix}{key}")
            } else {
                format!("{prefix}.{key}")
            }
        })
    }

    /// Re-roots this error under a list position — `"[2]"`, which a
    /// surrounding [`MapError::under`] then turns into `"tags[2]"`.
    #[must_use]
    pub fn at(self, index: usize) -> MapError {
        self.rekey(&|key| format!("[{index}]{key}"))
    }

    fn rekey(self, f: &dyn Fn(&str) -> String) -> MapError {
        match self {
            MapError::MissingKey { key } => MapError::MissingKey { key: f(&key) },
            MapError::WrongType {
                key,
                expected,
                found,
            } => MapError::WrongType {
                key: f(&key),
                expected,
                found,
            },
            MapError::BadValue { key, expected } => MapError::BadValue {
                key: f(&key),
                expected,
            },
        }
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::MissingKey { key } => write!(f, "no value under {}", named(key)),
            MapError::WrongType {
                key,
                expected,
                found,
            } => write!(
                f,
                "{} is {} where {} was needed",
                named(key),
                describe(*found),
                describe(Some(*expected))
            ),
            MapError::BadValue { key, expected } => {
                write!(f, "{} is not {expected}", named(key))
            }
        }
    }
}

impl std::error::Error for MapError {}

/// How to refer to a key in a message. The empty key is a value passed
/// in directly, which has no name.
fn named(key: &str) -> String {
    if key.is_empty() {
        "the value".to_string()
    } else {
        format!("'{key}'")
    }
}

/// A kind in a sentence. Beside [`MapError`] rather than on the tag:
/// diagnostic phrasing is not part of the value model, and on the enum it
/// would invite a consumer to key off the words.
fn describe(tag: Option<Tag>) -> &'static str {
    let Some(tag) = tag else {
        // A newer producer's kind is one this build carries and cannot
        // name.
        return "a kind this build does not know";
    };
    match tag {
        // Reachable: a caller may hand a reader the absent sentinel
        // rather than nothing at all, and "absent" is what that says.
        Tag::GUATIAO_ABSENT => "absent",
        Tag::GUATIAO_NULL => "null",
        Tag::GUATIAO_BOOL => "a boolean",
        Tag::GUATIAO_NUMBER => "a number",
        Tag::GUATIAO_STRING => "a string",
        Tag::GUATIAO_BYTES => "a byte string",
        Tag::GUATIAO_LIST => "a list",
        Tag::GUATIAO_MAP => "a map",
        // Exhaustive, with no catch-all: a kind added to the tag fails
        // to compile here until it is given a description.
    }
}

/// A borrowed read that can say no: [`AsRef`] with the refusal this
/// model needs. The type parameter is on the **trait**, so a binding
/// carries it and a turbofish is available where one does not:
///
/// ```
/// use guatiao::{List, Map, TryAsRef, Value};
///
/// let mut map = Map::new();
/// map.set("host", "10.0.0.1")?;
/// let v = Value::from(map);
///
/// let m: &Map = v.try_as_ref().unwrap();
/// assert_eq!(m.len(), 1);
/// assert!(TryAsRef::<List>::try_as_ref(&v).is_none());
/// assert_eq!(TryAsRef::<str>::try_as_ref(&v), None);
/// # Ok::<(), guatiao::ValueError>(())
/// ```
pub trait TryAsRef<T: ?Sized> {
    /// The value seen as a `T`, or `None` when it holds another kind.
    fn try_as_ref(&self) -> Option<&T>;
}

/// The mutable half of [`TryAsRef`].
///
/// A NUMBER and a STRING share one arm and do **not** share a type: the
/// reader for a [`Text`] answers `None` for a number, which keeps the
/// grammar from being edited away.
///
/// ```
/// use guatiao::{Map, Text, TryAsMut, Value};
///
/// let mut number = Value::from(1i64);
/// assert!(TryAsMut::<Text>::try_as_mut(&mut number).is_none());
///
/// let mut v = Value::from(Map::new());
/// let m: &mut Map = v.try_as_mut().unwrap();
/// m.set("port", 5900)?;
/// # Ok::<(), guatiao::ValueError>(())
/// ```
pub trait TryAsMut<T: ?Sized> {
    /// The value seen as a `T`, mutably, or `None` for another kind.
    fn try_as_mut(&mut self) -> Option<&mut T>;
}

/// Bytes, as a field type: `Vec<u8>` is a list like any other `Vec<T>`,
/// and a field that must cross as the `bytes` kind wears this.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bytes(
    /// The bytes themselves.
    pub Vec<u8>,
);

impl From<Vec<u8>> for Bytes {
    fn from(v: Vec<u8>) -> Bytes {
        Bytes(v)
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(b: Bytes) -> Vec<u8> {
        b.0
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Builds a value from a borrowed `self`. Borrowed, because a generated
/// `to_value(&self)` has only a borrow and a consuming trait would make
/// it clone every field.
pub trait ToValue {
    /// Performs the conversion. A derived struct produces a map.
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError>;
}

/// Reads `Self` out of a borrowed value.
pub trait FromValue: Sized {
    /// Performs the conversion.
    ///
    /// The returned error names **no key**: this value does not know what
    /// it was stored under. Whoever recursed adds that, with
    /// [`MapError::under`] or [`MapError::at`].
    fn from_value(value: &Value) -> Result<Self, MapError>;
}

// --- ToValue ----------------------------------------------------------

impl ToValue for bool {
    fn to_value(&self, _alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Value::from(*self))
    }
}

/// Every integer width, as its **decimal text**: `u64::MAX` and
/// `i128::MIN` cross as themselves, with no machine width to overflow.
macro_rules! integer_to_value {
    ($($t:ty),* $(,)?) => {$(
        impl ToValue for $t {
            fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
                Ok(Number::new_in(alloc, &self.to_string())?.into())
            }
        }
    )*};
}

integer_to_value!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl ToValue for f64 {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Number::float_in(alloc, *self)?.into())
    }
}

impl ToValue for str {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Text::new_in(alloc, self)?.into())
    }
}

impl ToValue for String {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Text::new_in(alloc, self)?.into())
    }
}

impl ToValue for Bytes {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Buffer::new_in(alloc, &self.0)?.into())
    }
}

/// A borrow converts as the thing it borrows does.
impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        (**self).to_value(alloc)
    }
}

/// `None` becomes a null **here**; the derive omits the key instead when
/// it can see the field is an `Option`. Both read back as `None`.
impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        match self {
            Some(v) => v.to_value(alloc),
            None => Ok(Value::null()),
        }
    }
}

impl<T: ToValue> ToValue for [T] {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        for item in self {
            list.push_in(item.to_value(alloc)?, alloc)?;
        }
        Ok(list.into())
    }
}

impl<T: ToValue> ToValue for Vec<T> {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.as_slice().to_value(alloc)
    }
}

/// A raw subtree is deep-copied into the target allocator: an owned tree
/// carries its own, which may not be the one the result belongs to.
impl ToValue for Value {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.clone_in(alloc)
    }
}

// --- the standard conversion traits -----------------------------------
//
// One `TryFrom<&Value>` impl per readable type, each delegating to the
// `FromValue` impl above so the two cannot disagree. Not a blanket one:
// the orphan rule refuses it, and refuses `Vec<T>`/`Option<T>` too.

/// One rule, two spellings of it.
macro_rules! try_from_value {
    ($($t:ty),* $(,)?) => {$(
        impl TryFrom<&Value> for $t {
            type Error = MapError;

            fn try_from(value: &Value) -> Result<$t, MapError> {
                <$t as FromValue>::from_value(value)
            }
        }
    )*};
}

try_from_value!(
    bool, f64, String, Bytes, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize,
);

/// Text borrowed from the value rather than copied out of it, which no
/// owning conversion can offer.
impl<'a> TryFrom<&'a Value> for &'a str {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a str, MapError> {
        TryAsRef::<str>::try_as_ref(value)
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_STRING, value))
    }
}

/// Bytes borrowed rather than copied, for the same reason.
impl<'a> TryFrom<&'a Value> for &'a [u8] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [u8], MapError> {
        TryAsRef::<[u8]>::try_as_ref(value)
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_BYTES, value))
    }
}

/// A list's elements, borrowed.
impl<'a> TryFrom<&'a Value> for &'a [Value] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [Value], MapError> {
        Ok(<&List>::try_from(value)?.items())
    }
}

/// A map's entries, in insertion order, borrowed.
impl<'a> TryFrom<&'a Value> for &'a [Entry] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [Entry], MapError> {
        Ok(<&Map>::try_from(value)?.entries())
    }
}

// --- FromValue --------------------------------------------------------

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<bool, MapError> {
        TryAsRef::<bool>::try_as_ref(value)
            .copied()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_BOOL, value))
    }
}

/// The number as text, or the error saying it was not a number at all.
fn number_text(value: &Value) -> Result<&str, MapError> {
    Ok(TryAsRef::<Number>::try_as_ref(value)
        .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_NUMBER, value))?
        .as_str())
}

/// An integer read **never truncates**: a fractional or exponent
/// spelling is refused rather than rounded, and so is an out-of-range
/// value. Both are the right kind and the wrong value.
macro_rules! integer_from_value {
    ($($t:ty),* $(,)?) => {$(
        impl FromValue for $t {
            fn from_value(value: &Value) -> Result<$t, MapError> {
                let text = number_text(value)?;
                if text.contains(['.', 'e', 'E']) {
                    return Err(MapError::bad_value(concat!(
                        "an integer written plainly -- a fractional or exponent \
                         spelling is refused rather than rounded into a ",
                        stringify!($t),
                    )));
                }
                text.parse::<$t>().map_err(|_| {
                    MapError::bad_value(concat!("an integer a ", stringify!($t), " can hold"))
                })
            }
        }
    )*};
}

integer_from_value!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<f64, MapError> {
        let text = number_text(value)?;
        // Rounding is not an error: an `f64` has 53 bits of mantissa and
        // refusing everything needing more would make most decimals
        // unreadable. What is refused is a number with no finite `f64`.
        text.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| MapError::bad_value("a number with a finite f64 value"))
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<String, MapError> {
        Ok(<&str>::try_from(value)?.to_string())
    }
}

impl FromValue for Bytes {
    fn from_value(value: &Value) -> Result<Bytes, MapError> {
        Ok(Bytes(<&[u8]>::try_from(value)?.to_vec()))
    }
}

/// Each element is read under `[i]`, so a failure names which one.
impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> Result<Vec<T>, MapError> {
        let items = <&List>::try_from(value)?.items();
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            out.push(T::from_value(item).map_err(|e| e.at(i))?);
        }
        Ok(out)
    }
}

/// A stored null reads as `None`; anything else is offered to `T`.
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Option<T>, MapError> {
        if value.tag() == Ok(Tag::GUATIAO_NULL) {
            return Ok(None);
        }
        Ok(Some(T::from_value(value)?))
    }
}
