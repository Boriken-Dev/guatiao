// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a value comfortably from Rust.
//!
//! The primitives are in [`super::mutate`]; this is the layer a program
//! actually reads with — iteration, comparison, a debug view, and the
//! getters that take a caller's default.
//!
//! # The defaulting getters, and the one that matters
//!
//! [`int_or`] answers the caller's default for three different situations:
//! the value is absent, it is present but another kind, and it is a number
//! that is **not representable** as an `i64`. That third case is the point.
//! `strtoll` in C saturates to `LLONG_MAX` and sets `errno`, which is a
//! wrong answer that looks like a right one; a value with a fractional
//! part or an exponent spelling is not an integer at all. Answering the
//! default for all three is what keeps a consumer from re-deriving the
//! rule and getting it wrong.
//!
//! A caller that must tell the three apart has [`Value::tag`] to separate
//! absent from present, and [`Value::as_number_str`] to separate "not
//! representable" from the rest, since it hands back the exact text
//! whatever its magnitude.
//!
//! These mirror the `static inline` helpers in the generated header one
//! for one, deliberately: two implementations of the same rule that must
//! agree eventually will not, and a test pins them equal.
//!
//! # Get the value, then convert it
//!
//! There are **no per-kind getters on a map** — no `map.str("host")` —
//! and no per-kind readers on a value either. A lookup answers a value
//! and `TryInto` turns that value into a Rust type, which is two steps
//! that compose rather than one method per pair of (container, kind), and
//! the second step is the one every Rust programmer already knows.
//!
//! [`ReadValue`] is what makes the lookup compose: it is implemented for
//! `Option<&Value>` as well as for `&Value`, so a path chains and a key
//! that was not there reports itself at the end.
//!
//! ```
//! use guatiao::{Map, ReadValue};
//!
//! let mut tls = Map::new();
//! tls.set("verify", true)?;
//! let mut map = Map::new();
//! map.set("host", "10.0.0.1")?;
//! map.set("port", 5900)?;
//! map.set("tls", tls)?;
//!
//! let host: &str = map.get("host").ok_or_missing()?.try_into()?;
//! let port: u16 = map.get("port").ok_or_missing()?.try_into()?;
//! let verify: bool = map.get("tls").get("verify").ok_or_missing()?.try_into()?;
//! assert_eq!((host, port, verify), ("10.0.0.1", 5900, true));
//!
//! // Or without a binding to carry the type:
//! assert_eq!(
//!     TryInto::<String>::try_into(map.get("host").ok_or_missing()?)?,
//!     "10.0.0.1"
//! );
//!
//! // A key that is not there, and a value of another kind, each say so.
//! assert!(map.get("nothing").ok_or_missing().is_err());
//! assert!(TryInto::<i64>::try_into(map.get("host").ok_or_missing()?).is_err());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The defaulting getters above answer a different question -- "this or
//! the fallback" rather than "this or why not" -- and both are worth
//! having: a reader building a diagnostic wants the error, and a reader
//! filling in a form wants the default.

#![forbid(unsafe_code)]

use std::fmt;

use super::convert::MapError;
use super::mutate::{MAX_DEPTH, text_bytes};
use super::types::{Entry, Tag, Value};

/// Reading a value as a Rust type, and saying why not when it is not one.
///
/// Implemented for `&Value` and for `Option<&Value>`, so
/// the result of a lookup converts directly. `None` becomes
/// [`MapError::MissingKey`] with no key name, because an `Option` has
/// forgotten what was asked for; a caller that wants the name adds it with
/// [`MapError::under`].
pub trait ReadValue<'a> {
    /// The value under `key`, when this is a map. `None` otherwise, which
    /// is what lets a path of `get` calls chain.
    fn get(self, key: &str) -> Option<&'a Value>;

    /// The element at `index`, when this is a list.
    fn at(self, index: usize) -> Option<&'a Value>;

    /// The value, or [`MapError::MissingKey`] if the lookup found none.
    ///
    /// Named after `Option::ok_or`, which is what it is: the error is
    /// filled in rather than passed, because there is only one it could
    /// be. It is the step from a lookup to a value, and the value converts
    /// from there the way anything converts in Rust:
    ///
    /// ```text
    /// let port: u16 = map.get("port").ok_or_missing()?.try_into()?;
    /// ```
    fn ok_or_missing(self) -> Result<&'a Value, MapError>;
}

impl<'a> ReadValue<'a> for &'a Value {
    // Spelled as a path rather than `self.get(key)`: the inherent method
    // and this one share a name, and on a `&Value` receiver the trait one
    // wins — which would make this call itself.
    fn get(self, key: &str) -> Option<&'a Value> {
        Value::get(self, key)
    }

    fn at(self, index: usize) -> Option<&'a Value> {
        self.items()?.get(index)
    }

    fn ok_or_missing(self) -> Result<&'a Value, MapError> {
        Ok(self)
    }
}

/// A lookup that found nothing converts to [`MapError::MissingKey`]
/// rather than needing an `ok_or` at every call site. That is the whole
/// reason this impl exists: without it, "get then convert" costs an
/// unwrap in the middle and stops being one expression.
impl<'a> ReadValue<'a> for Option<&'a Value> {
    fn get(self, key: &str) -> Option<&'a Value> {
        self.and_then(|v| Value::get(v, key))
    }

    fn at(self, index: usize) -> Option<&'a Value> {
        self.and_then(|v| v.items()?.get(index))
    }

    fn ok_or_missing(self) -> Result<&'a Value, MapError> {
        missing(self)
    }
}

/// The one place the absent case is turned into an error, so every method
/// above reports it identically.
fn missing(v: Option<&Value>) -> Result<&Value, MapError> {
    v.ok_or_else(|| MapError::missing(""))
}

// --- defaulting getters -----------------------------------------------

/// The boolean, or `fallback` when absent or another kind.
pub fn bool_or(v: Option<&Value>, fallback: bool) -> bool {
    v.and_then(Value::as_bool).unwrap_or(fallback)
}

/// The number as an `i64`, or `fallback`.
///
/// **Never truncates.** `fallback` is answered for a value outside
/// `i64`'s range, one with a fractional part, and one written with an
/// exponent — `1e2` is integral in value but not in spelling, and
/// evaluating the exponent correctly for arbitrary precision is the
/// arithmetic this container exists to avoid.
pub fn int_or(v: Option<&Value>, fallback: i64) -> i64 {
    let Some(text) = v.and_then(Value::as_number_str) else {
        return fallback;
    };
    if text.contains(['.', 'e', 'E']) {
        return fallback;
    }
    text.parse::<i64>().unwrap_or(fallback)
}

/// The number as the nearest `f64`, or `fallback` when there is no finite
/// one.
///
/// Lossy by construction: an `f64` has 53 bits of mantissa, so a value
/// needing more comes back rounded and nothing here reports that it was.
pub fn float_or(v: Option<&Value>, fallback: f64) -> f64 {
    v.and_then(Value::as_number_str)
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|x| x.is_finite())
        .unwrap_or(fallback)
}

/// The string, or `fallback` when absent or another kind.
///
/// A number is not a string, and `"5"` is not the number 5. Nothing here
/// coerces: a value that reads as plausible under two kinds hides the
/// producer and the consumer disagreeing about which it is.
pub fn str_or<'a>(v: Option<&'a Value>, fallback: &'a str) -> &'a str {
    v.and_then(Value::as_str).unwrap_or(fallback)
}

/// The bytes, or an empty slice.
pub fn bytes_or<'a>(v: Option<&'a Value>, fallback: &'a [u8]) -> &'a [u8] {
    v.and_then(Value::as_bytes).unwrap_or(fallback)
}

// --- iteration --------------------------------------------------------

/// A map's entries, in **insertion order**, as `(key bytes, value)`.
///
/// The key is bytes rather than `&str` because a key is bytes: no folding,
/// no normalisation, and it may contain a NUL. A caller that wants text
/// asks for it and handles the answer.
///
/// Not the same thing as [`Value::entries`], which hands back the slice
/// and says `None` for anything that is not a map: this one is what a
/// `for` loop takes, and it walks nothing rather than reporting the kind.
pub fn entries(v: &Value) -> impl Iterator<Item = (&[u8], &Value)> {
    v.entries()
        .unwrap_or(&[])
        .iter()
        .map(|e: &Entry| (e.key(), e.value()))
}

/// A map's keys, in insertion order.
pub fn keys(v: &Value) -> impl Iterator<Item = &[u8]> {
    entries(v).map(|(k, _)| k)
}

/// A list's items, in order.
///
/// The iterator counterpart of [`Value::items`], on the same terms as
/// [`entries`] above: a value of another kind walks nothing.
pub fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v.items().unwrap_or(&[]).iter()
}

// --- comparison -------------------------------------------------------

/// Whether two trees hold the same thing.
///
/// **Order is significant for a map**, because insertion order is part of
/// the contract rather than an artefact: consumers render maps as forms,
/// print them as tables and diff them in tests, and all three need it
/// stable and meaningful. Two maps with the same pairs in a different
/// order are two different values here, and saying otherwise would make
/// this function disagree with every consumer that renders one.
///
/// A tag this build does not know compares equal only to the same unknown
/// tag, which is the most anyone can say about it.
///
/// # Bounded, like every other walk here
///
/// Two trees nested deeper than [`MAX_DEPTH`] compare **unequal** without
/// being walked further. The input can come from a foreign producer, and
/// an unbounded recursion over one is a stack overflow — which on Windows
/// is not catchable and takes the host down with it. Answering "not equal"
/// is the conservative half of that trade: a caller using this for
/// uniqueness keeps an item it might have discarded, which is a value kept
/// rather than a value lost.
///
/// Nothing this crate can build reaches that depth anyway:
/// [`Value::clone_in`] refuses past the same bound.
pub fn equal(a: &Value, b: &Value) -> bool {
    equal_at(a, b, 0)
}

fn equal_at(a: &Value, b: &Value, depth: u32) -> bool {
    if a.tag != b.tag {
        return false;
    }
    if depth >= MAX_DEPTH {
        return false;
    }
    match a.tag() {
        Ok(Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL) => true,
        Ok(Tag::GUATIAO_BOOL) => a.as_bool() == b.as_bool(),
        // Byte equality of the text, which is what makes `1.10` different
        // from `1.1`. Comparing as numbers would be the lossy view, and
        // the whole point of storing text is that it is not taken.
        //
        // The BYTES, not the `&str`: a string whose content is not UTF-8
        // reads back as `None`, and two different such strings would then
        // compare equal to each other on the strength of both being
        // unreadable.
        Ok(Tag::GUATIAO_NUMBER | Tag::GUATIAO_STRING) => text_bytes(a) == text_bytes(b),
        Ok(Tag::GUATIAO_BYTES) => a.as_bytes() == b.as_bytes(),
        Ok(Tag::GUATIAO_LIST) => {
            let (x, y) = (a.items().unwrap_or(&[]), b.items().unwrap_or(&[]));
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal_at(p, q, depth + 1))
        }
        Ok(Tag::GUATIAO_MAP) => {
            let (x, y) = (a.entries().unwrap_or(&[]), b.entries().unwrap_or(&[]));
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|(p, q)| p.key() == q.key() && equal_at(p.value(), q.value(), depth + 1))
        }
        // Two values this build cannot read are equal exactly when their
        // tags are, which the first line already established.
        Err(_) => true,
    }
}

// --- a debug view -----------------------------------------------------

/// A readable dump of a tree, for diagnostics.
///
/// **Not a format.** It does not round-trip, nothing parses it, and its
/// exact spelling is not a contract — which is the difference between
/// this and the serialisation that deliberately lives in another crate.
/// Bytes are shown as a length and a short hex prefix rather than decoded,
/// because a byte value is not text and pretending otherwise is how a
/// diagnostic misleads.
pub struct Dump<'a>(pub &'a Value);

impl fmt::Debug for Dump<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write(f, self.0, 0)
    }
}

/// How deep the dump will follow before saying so.
///
/// A tree can arrive from a foreign producer, and a debug printer that
/// recursed without a bound would turn a diagnostic into a stack overflow
/// — in the one code path a person reaches for when something is already
/// wrong.
const DUMP_DEPTH: u32 = 32;

fn write(f: &mut fmt::Formatter<'_>, v: &Value, depth: u32) -> fmt::Result {
    if depth >= DUMP_DEPTH {
        return f.write_str("...");
    }
    match v.tag() {
        Err(_) => write!(f, "<unknown tag {}>", v.tag),
        Ok(Tag::GUATIAO_ABSENT) => f.write_str("absent"),
        Ok(Tag::GUATIAO_NULL) => f.write_str("null"),
        Ok(Tag::GUATIAO_BOOL) => write!(f, "{}", v.as_bool().unwrap_or(false)),
        Ok(Tag::GUATIAO_NUMBER) => f.write_str(v.as_number_str().unwrap_or("<not utf-8>")),
        Ok(Tag::GUATIAO_STRING) => write!(f, "{:?}", v.as_str().unwrap_or("<not utf-8>")),
        Ok(Tag::GUATIAO_BYTES) => {
            let b = v.as_bytes().unwrap_or(&[]);
            write!(f, "<{} bytes", b.len())?;
            for byte in b.iter().take(8) {
                write!(f, " {byte:02x}")?;
            }
            if b.len() > 8 {
                f.write_str(" ...")?;
            }
            f.write_str(">")
        }
        Ok(Tag::GUATIAO_LIST) => {
            f.write_str("[")?;
            for (i, item) in items(v).enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write(f, item, depth + 1)?;
            }
            f.write_str("]")
        }
        Ok(Tag::GUATIAO_MAP) => {
            f.write_str("{")?;
            for (i, (key, value)) in entries(v).enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                match std::str::from_utf8(key) {
                    Ok(text) => write!(f, "{text:?}")?,
                    Err(_) => write!(f, "<{} key bytes>", key.len())?,
                }
                f.write_str(": ")?;
                write(f, value, depth + 1)?;
            }
            f.write_str("}")
        }
    }
}
