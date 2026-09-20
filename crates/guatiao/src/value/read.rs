// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a value with a caller's default, and a debug view.
//!
//! # The defaulting getters, and the one that matters
//!
//! [`int_or`] answers the caller's default for three different situations:
//! the value is absent, it is present but another kind, and it is a number
//! that is **not representable** as an `i64`. That third case is the point.
//! `strtoll` in C saturates to `LLONG_MAX` and sets `errno`, which is a
//! wrong answer that looks like a right one; a value with a fractional
//! part or an exponent spelling is not an integer at all.
//!
//! A caller that must tell the three apart has [`Value::tag`] to separate
//! absent from present, and `TryAsRef::<Number>` to separate "not
//! representable" from the rest, since a [`Number`] hands back the exact
//! text whatever its magnitude.
//!
//! These mirror the `static inline` helpers in the generated header one
//! for one, deliberately: two implementations of the same rule that must
//! agree eventually will not, and a test pins them equal.
//!
//! # Get the value, then convert it
//!
//! There are **no per-kind getters on a map** — no `map.str("host")` — and
//! no per-kind readers on a value either. A lookup answers a value and
//! `TryInto` turns that value into a Rust type, which is two steps that
//! compose rather than one method per pair of (container, kind).
//!
//! [`Map::required`] is the step from a lookup to a value, and it names
//! the key an `Option` has already forgotten:
//!
//! ```
//! use guatiao::{Map, TryAsRef};
//!
//! let mut tls = Map::new();
//! tls.set("verify", true)?;
//! let mut map = Map::new();
//! map.set("host", "10.0.0.1")?;
//! map.set("port", 5900)?;
//! map.set("tls", tls)?;
//!
//! let host: &str = map.required("host")?.try_into()?;
//! let port: u16 = map.required("port")?.try_into()?;
//! let tls: &Map = <&Map>::try_from(map.required("tls")?)?;
//! let verify: bool = tls.required("verify")?.try_into()?;
//! assert_eq!((host, port, verify), ("10.0.0.1", 5900, true));
//!
//! // A key that is not there, and a value of another kind, each say so.
//! assert!(map.required("nothing").is_err());
//! assert!(TryInto::<i64>::try_into(map.required("host")?).is_err());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The defaulting getters answer a different question — "this or the
//! fallback" rather than "this or why not" — and both are worth having: a
//! reader building a diagnostic wants the error, and a reader filling in a
//! form wants the default.

#![forbid(unsafe_code)]

use std::fmt;

use super::convert::TryAsRef;
use super::types::{List, Map, Number, Tag, Value};

// --- defaulting getters -----------------------------------------------

/// The boolean, or `fallback` when absent or another kind.
pub fn bool_or(v: Option<&Value>, fallback: bool) -> bool {
    v.and_then(|v| bool::try_from(v).ok()).unwrap_or(fallback)
}

/// A number's exact text, or `None` when this is not a number.
fn number_text(v: Option<&Value>) -> Option<&str> {
    Some(TryAsRef::<Number>::try_as_ref(v?)?.as_str())
}

/// The number as an `i64`, or `fallback`.
///
/// **Never truncates.** `fallback` is answered for a value outside
/// `i64`'s range, one with a fractional part, and one written with an
/// exponent — `1e2` is integral in value but not in spelling, and
/// evaluating the exponent correctly for arbitrary precision is the
/// arithmetic this container exists to avoid.
pub fn int_or(v: Option<&Value>, fallback: i64) -> i64 {
    let Some(text) = number_text(v) else {
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
    number_text(v)
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
    v.and_then(TryAsRef::<str>::try_as_ref).unwrap_or(fallback)
}

/// The bytes, or an empty slice.
pub fn bytes_or<'a>(v: Option<&'a Value>, fallback: &'a [u8]) -> &'a [u8] {
    v.and_then(TryAsRef::<[u8]>::try_as_ref).unwrap_or(fallback)
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
        Ok(Tag::GUATIAO_BOOL) => write!(f, "{}", bool_or(Some(v), false)),
        Ok(Tag::GUATIAO_NUMBER) => f.write_str(number_text(Some(v)).unwrap_or("<not utf-8>")),
        Ok(Tag::GUATIAO_STRING) => write!(
            f,
            "{:?}",
            TryAsRef::<str>::try_as_ref(v).unwrap_or("<not utf-8>")
        ),
        Ok(Tag::GUATIAO_BYTES) => {
            let b = bytes_or(Some(v), &[]);
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
            let empty = List::new();
            f.write_str("[")?;
            for (i, item) in TryAsRef::<List>::try_as_ref(v)
                .unwrap_or(&empty)
                .iter()
                .enumerate()
            {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write(f, item, depth + 1)?;
            }
            f.write_str("]")
        }
        Ok(Tag::GUATIAO_MAP) => {
            let empty = Map::new();
            let map = TryAsRef::<Map>::try_as_ref(v).unwrap_or(&empty);
            f.write_str("{")?;
            for (i, entry) in map.iter().enumerate() {
                let (key, value) = (entry.key(), entry.value());
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
