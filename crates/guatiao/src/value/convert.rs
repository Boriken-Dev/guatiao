// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Converting a Rust type to and from a value.
//!
//! Two traits, [`ToValue`] and [`FromValue`], and the derive writes both.
//! There is no separate map trait because there is no separate map type:
//! a map is a value whose tag says map, so a second pair of traits would
//! differ from these in their name and in nothing else.
//!
//! # Writing takes an allocator; reading does not
//!
//! Building a value means allocating one, and the allocator travels with
//! the tree rather than being global, so every write takes one. Reading
//! copies into ordinary Rust types — a `String`, a `Vec<T>` — so it needs
//! nothing.
//!
//! ```
//! use guatiao::Alloc;
//! use guatiao::value::convert::{FromValue, ToValue};
//!
//! let alloc = Alloc::rust();
//!
//! let value = "10.0.0.1".to_value(alloc)?;
//! assert_eq!(String::from_value(&value)?, "10.0.0.1");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Why a named trait as well as `TryFrom`
//!
//! Every readable scalar implements `TryFrom<&Value>`, so `try_into` is
//! the ordinary Rust spelling and the section below says what that costs
//! and covers. These two traits stay underneath it for three reasons the
//! standard ones cannot meet:
//!
//! - **The orphan rule refuses a blanket impl.** `impl<T: FromValue>
//!   TryFrom<&Value> for T` puts a bare parameter where the rule requires
//!   a local type, and `Vec<T>` and `Option<T>` fall to the same rule, so
//!   a generic reader needs a trait of this crate's own to bound on.
//! - **Writing has no `From` to be.** Building a value allocates, the
//!   allocator travels with the tree rather than being global, and
//!   `From::from` takes one argument.
//! - **`TryFrom` for a user's own type is theirs**, and they may already
//!   have written it; a derive that claimed it would be a coherence
//!   hazard rather than a convenience.
//!
//! Both directions are fallible and for different reasons, which is worth
//! keeping straight: a write fails because the allocator refused or a
//! `f64` was not finite — an [`ValueError`] — while a read fails because
//! a key is missing or holds the wrong kind, which is a [`MapError`] and a
//! completely different question.
//!
//! # The error distinguishes its cases, on purpose
//!
//! [`MapError`] separates [`MapError::MissingKey`] from
//! [`MapError::WrongType`] from [`MapError::BadValue`]. One opaque error
//! would make a **typo'd key** and a **type mismatch** indistinguishable,
//! and those have opposite fixes: the first is a spelling correction in
//! the producer, the second is a disagreement about what the value means.
//!
//! # The path is built on the way out, not carried on the way in
//!
//! A leaf error names no key, because a leaf does not know what it was
//! stored under. Each level that recurses adds its own step —
//! [`MapError::under`] for a field, [`MapError::at`] for a list element —
//! so a failure two levels down reports `"tls.verify"` or `"tags[2]"`
//! rather than `"verify"`. A bare field name is ambiguous the moment
//! anything nests, and threading a prefix downwards would make every
//! implementation responsible for something only its caller knows.
//!
//! # `Option<T>`: absent and null are read the same way
//!
//! - **Writing:** the derive **omits the key entirely** for `None`. It
//!   does not store a null. A record with unset options is a record with
//!   fewer keys, which is how a config file spells it.
//! - **Reading:** **both** an absent key **and** a stored null produce
//!   `None`.
//!
//! So `Option<T>` deliberately cannot tell "the producer said nothing"
//! from "the producer said explicitly nothing". That distinction is real,
//! and the way to keep it is to read the map directly — [`Value::get`]
//! answers `None` only for an absent key, and a stored null arrives as a
//! value whose tag says so.
//!
//! The asymmetry (write omits, read accepts both) is what makes the round
//! trip total: every map written here reads back to the value that wrote
//! it, and a map from a producer that spells absence as null is still
//! readable.
//!
//! # A sequence is a `Vec<T>`, and bytes are opt-in
//!
//! `Vec<T>` is a list for every `T`, `Vec<u8>` included. Bytes are a
//! separate kind and a field that wants them says so by its type:
//! [`Bytes`].
//!
//! This is the opposite of the arrangement it replaces, where `Vec<u8>`
//! was bytes and no generic `Vec<T>` impl could exist beside it —
//! coherence refuses both at once, and the sequence is the case that
//! comes up constantly while a byte blob is the rare one. It is also what
//! `serde` settled on for the same reason, so the shape is already
//! familiar.
//!
//! # What `try_into` covers
//!
//! Every scalar — the twelve integer widths, `f64`, `bool`, `String`,
//! [`Bytes`] — and the borrowed forms: `&str`, `&[u8]`, a list's
//! `&[Value]` and a map's `&[Entry]`. There is no second
//! vocabulary of named readers beside it, because a conversion is what
//! `TryInto` is for and one spelling is easier to remember than eight.
//!
//! `Vec<T>` and `Option<T>` are the two the orphan rule keeps out, and
//! they read through [`FromValue`] or through a derived struct, which is
//! where a generic caller already is.
//!
//! A lookup answers an `Option`, which has no `TryFrom` to offer either
//! — `ok_or_missing()` on [`ReadValue`](super::read::ReadValue) is the
//! step from one to the other:
//! `map.get("port").ok_or_missing()?.try_into()?`.
//!
//! # There is no `FromValue` for an owned subtree
//!
//! [`ToValue`] is implemented for [`Value`] itself, so a subtree already
//! in hand can be **written** — it deep-copies into the target
//! allocator, because the tree it is going into may not be on the heap it
//! came from. Reading one back would have to
//! allocate, and [`FromValue`] deliberately takes no allocator. A field
//! that needs the raw thing reads it with [`Value::get`] and clones it
//! explicitly, which is one line and says what it costs.

use std::fmt;

use super::alloc::Alloc;
use super::mutate::ValueError;
use super::types::{Entry, Tag, Value};

/// Why a value could not be read as a particular Rust type.
///
/// The three cases are kept apart deliberately — see this module's
/// documentation. Each names the key it is about, as a dotted path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MapError {
    /// Nothing is stored under this key, and the field is not optional.
    MissingKey {
        /// The dotted path of the key that was not found.
        key: String,
    },
    /// A value is stored, but it is the wrong kind entirely.
    WrongType {
        /// The dotted path of the key, empty for a value passed in
        /// directly rather than found under one.
        key: String,
        /// The kind the field needed.
        expected: Tag,
        /// The kind that was actually there, or `None` for a tag this
        /// build does not know — which is a real answer rather than a
        /// missing one, and the reason this is not a bare tag.
        found: Option<Tag>,
    },
    /// The right kind, but not a value this field can hold: a number too
    /// large for the field's integer type, a spelling nobody declared.
    ///
    /// Separate from [`MapError::WrongType`] because the fix is
    /// different: the producer chose the right *kind* and the wrong
    /// *value*.
    BadValue {
        /// The dotted path of the key.
        key: String,
        /// What the field would have accepted, phrased for a person.
        expected: String,
    },
}

impl MapError {
    /// A missing key. The one error that names its key up front, because
    /// a lookup that found nothing is the only place that knows it.
    pub fn missing(key: &str) -> MapError {
        MapError::MissingKey {
            key: key.to_string(),
        }
    }

    /// A value of the wrong kind, reading the kind that is actually there
    /// off the value itself so no caller has to spell it.
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
    ///
    /// Without this, a caller reading a struct with two nested maps that
    /// both have a `verify` field is told which *field* failed and not
    /// which *map*, which is the more useful half.
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

/// How to refer to a key in a message.
///
/// The empty key is the value a caller passed in directly, which has no
/// name — writing `''` there reads as a key that is genuinely called
/// nothing.
fn named(key: &str) -> String {
    if key.is_empty() {
        "the value".to_string()
    } else {
        format!("'{key}'")
    }
}

/// A kind in a sentence. Kept beside [`MapError`] rather than on the tag
/// itself: it is diagnostic phrasing, not part of the value model, and
/// putting it on the enum would invite a consumer to key off the words.
fn describe(tag: Option<Tag>) -> &'static str {
    let Some(tag) = tag else {
        // Not an error and not a lie: a newer producer's kind is a thing
        // this build can carry and cannot name.
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
        // Deliberately EXHAUSTIVE, with no catch-all: a kind added to the
        // tag fails to compile here until it is given a description,
        // which is what should happen. A `_` arm would let a new kind
        // read as "a kind this build does not know" in every message,
        // silently and forever.
    }
}

/// Bytes, as a field type.
///
/// `Vec<u8>` is a list of numbers like any other `Vec<T>`; a field that
/// must cross as the `bytes` kind says so by wearing this. See this
/// module's documentation for why round that way.
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

/// Builds a value from a borrowed `self`.
///
/// Borrowed rather than consuming, because a generated `to_value(&self)`
/// has only a borrow and a consuming trait would make it clone every
/// field.
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

/// Refuses a value that is not a map, naming the kind it found.
///
/// What a generated [`FromValue`] calls first, so handing a string to a
/// struct's reader reports *that* rather than reporting every field
/// missing.
pub fn expect_map(value: &Value) -> Result<(), MapError> {
    if value.tag() == Ok(Tag::GUATIAO_MAP) {
        Ok(())
    } else {
        Err(MapError::wrong_type(Tag::GUATIAO_MAP, value))
    }
}

/// The text a value holds, or [`MapError::WrongType`] naming what it held
/// instead.
///
/// What a generated enum reader calls: a variant is stored as its name, so
/// the reader needs the text to match on — borrowed, because allocating a
/// `String` only to compare it against a handful of literals would be the
/// one allocation in an otherwise copy-free read.
pub fn expect_str(value: &Value) -> Result<&str, MapError> {
    value
        .as_str()
        .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_STRING, value))
}

/// The value stored under `key`, or [`MapError::MissingKey`] naming it.
///
/// The other half of what a generated reader needs: a required field that
/// is absent must say so, and say which key.
pub fn expect_key<'a>(map: &'a Value, key: &str) -> Result<&'a Value, MapError> {
    map.get(key).ok_or_else(|| MapError::missing(key))
}

/// The value stored under `key`, if any.
///
/// The optional-field half of [`expect_key`]. It exists so generated code
/// names one module and not two, and so the absent-versus-null rule is
/// read off one place: this answers `None` only for an absent key, and a
/// stored null arrives as a value whose tag says so.
pub fn find_key<'a>(map: &'a Value, key: &str) -> Option<&'a Value> {
    map.get(key)
}

// --- ToValue ----------------------------------------------------------

impl ToValue for bool {
    fn to_value(&self, _alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Value::bool(*self))
    }
}

/// Every integer width, written as its **decimal text**.
///
/// Not squeezed through an `i64` on the way: a number is the exact text
/// that declared it, so `u64::MAX` and `i128::MIN` cross as themselves
/// rather than failing or wrapping. The text form is what makes that free
/// -- there is no machine width at the boundary to overflow.
macro_rules! integer_to_value {
    ($($t:ty),* $(,)?) => {$(
        impl ToValue for $t {
            fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
                Value::number_in(alloc, &self.to_string())
            }
        }
    )*};
}

integer_to_value!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl ToValue for f64 {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Value::float_in(alloc, *self)
    }
}

impl ToValue for str {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Value::string_in(alloc, self)
    }
}

impl ToValue for String {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Value::string_in(alloc, self)
    }
}

impl ToValue for Bytes {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Value::bytes_in(alloc, &self.0)
    }
}

/// A borrow converts exactly as the thing it borrows does, so a field
/// typed `&str` needs no separate impl.
impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        (**self).to_value(alloc)
    }
}

/// `None` becomes a null **here**; the derive omits the key instead when
/// it can see the field is an `Option`. Both readings come back as
/// `None`, so the two spellings agree — see this module's documentation.
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
        let mut list = Value::list_in(alloc);
        for item in self {
            list.push(item.to_value(alloc)?)?;
        }
        Ok(list)
    }
}

impl<T: ToValue> ToValue for Vec<T> {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.as_slice().to_value(alloc)
    }
}

/// A raw subtree is deep-copied into the target allocator, because the
/// one it is built on may not be the one the result belongs to and an
/// owned tree carries its allocator with it.
impl ToValue for Value {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> {
        // SAFETY: `Value::clone_in` needs a well-formed node, and a
        // `&Value` reaching a safe function in this crate is one -- the
        // same precondition every reader here already relies on. Every
        // buffer in the tree it hands back came from `alloc`, and nothing
        // else holds it, so the caller owns the whole copy.
        unsafe { self.clone_in(alloc) }
    }
}

// --- the standard conversion traits -----------------------------------
//
// `value.try_into()` is the spelling a Rust reader reaches for first, so
// it works: one `TryFrom<&Value>` impl per readable type, each
// delegating to the [`FromValue`] impl above so the two cannot disagree.
//
// **One impl per concrete type, and not a blanket one.** The orphan rule
// refuses `impl<T: FromValue> TryFrom<&Value> for T` outright:
// the self type is a bare parameter standing before the local type, which
// is exactly the shape the rule exists to forbid. It refuses `Vec<T>` and
// `Option<T>` for the same reason -- a parameter inside a foreign type
// constructor is not covered -- so those two convert through [`FromValue`]
// and through the derive, which is where a generic caller is anyway.
//
// The **writing** direction has no `From` to be. Building a value means
// allocating one, the allocator travels with the tree rather than being
// global, and `From::from` takes one argument. That is a property of the
// value model rather than an omission here: [`ToValue`] carries the
// allocator because something has to.

/// One `TryFrom<&Value>` per type that can be read back, delegating to
/// [`FromValue`] so there is one rule and two spellings of it.
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
        value
            .as_str()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_STRING, value))
    }
}

/// Bytes borrowed rather than copied, for the same reason.
impl<'a> TryFrom<&'a Value> for &'a [u8] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [u8], MapError> {
        value
            .as_bytes()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_BYTES, value))
    }
}

/// A list's elements, borrowed.
impl<'a> TryFrom<&'a Value> for &'a [Value] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [Value], MapError> {
        value
            .items()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_LIST, value))
    }
}

/// A map's entries, in insertion order, borrowed.
impl<'a> TryFrom<&'a Value> for &'a [Entry] {
    type Error = MapError;

    fn try_from(value: &'a Value) -> Result<&'a [Entry], MapError> {
        value
            .entries()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_MAP, value))
    }
}

// --- FromValue --------------------------------------------------------

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<bool, MapError> {
        value
            .as_bool()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_BOOL, value))
    }
}

/// The number as text, or the error saying it was not a number at all.
fn number_text(value: &Value) -> Result<&str, MapError> {
    value
        .as_number_str()
        .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_NUMBER, value))
}

/// An integer read **never truncates**: a fractional or exponent spelling
/// is refused rather than rounded, and so is a value outside the target
/// type. Both are the right kind and the wrong value.
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
        // Rounding is not an error here: an `f64` has 53 bits of mantissa
        // and refusing every number that needs more would make most
        // decimals unreadable. What is refused is a number with no finite
        // `f64` at all.
        text.parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .ok_or_else(|| MapError::bad_value("a number with a finite f64 value"))
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<String, MapError> {
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_STRING, value))
    }
}

impl FromValue for Bytes {
    fn from_value(value: &Value) -> Result<Bytes, MapError> {
        value
            .as_bytes()
            .map(|b| Bytes(b.to_vec()))
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_BYTES, value))
    }
}

/// Each element is read under `[i]`, so a failure names which element
/// rather than only which field.
impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> Result<Vec<T>, MapError> {
        let items = value
            .items()
            .ok_or_else(|| MapError::wrong_type(Tag::GUATIAO_LIST, value))?;
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            out.push(T::from_value(item).map_err(|e| e.at(i))?);
        }
        Ok(out)
    }
}

/// A stored null reads as `None`; anything else is offered to `T`. See
/// this module's documentation for why absent and null collapse.
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Option<T>, MapError> {
        if value.tag() == Ok(Tag::GUATIAO_NULL) {
            return Ok(None);
        }
        Ok(Some(T::from_value(value)?))
    }
}
