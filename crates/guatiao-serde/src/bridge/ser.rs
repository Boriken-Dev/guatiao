// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao walk, written by any serde format: each event is the
//! serializer call for it, and a container's elements are `Serialize`
//! values that walk themselves.

#![forbid(unsafe_code)]

use core::fmt;

use serde::ser::{Error as _, SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use guatiao::value::visit::{ListVisitor, MapVisitor, Visitor, Walk};
use guatiao::{Number, ValueError};

use crate::{Bytes, Numbers, Presentation, data_uri};

/// Anything that walks, as a serde `Serialize`, carrying the policy for
/// what a format may not have: written by any serde format with no tree
/// built.
///
/// ```
/// use guatiao_serde::{Presentation, Walked};
///
/// let ports = vec![5900u16, 5901];
/// let text = serde_json::to_string(&Walked::new(&ports, Presentation::new()))?;
/// assert_eq!(text, "[5900,5901]");
/// # Ok::<(), serde_json::Error>(())
/// ```
pub struct Walked<'a, T: ?Sized> {
    value: &'a T,
    how: Presentation,
}

impl<'a, T: ?Sized> Walked<'a, T> {
    /// `value`, written the way `how` says.
    pub fn new(value: &'a T, how: Presentation) -> Walked<'a, T> {
        Walked { value, how }
    }
}

impl<T: ?Sized> Clone for Walked<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Walked<'_, T> {}

impl<T: ?Sized> fmt::Debug for Walked<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Walked")
            .field("how", &self.how)
            .finish_non_exhaustive()
    }
}

/// With the default presentation.
impl<'a, T: ?Sized> From<&'a T> for Walked<'a, T> {
    fn from(value: &'a T) -> Walked<'a, T> {
        Walked {
            value,
            how: Presentation::new(),
        }
    }
}

impl<T: Walk + ?Sized> Serialize for Walked<'_, T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_walk(self.value, s, self.how)
    }
}

/// `value`'s walk into `s`.
pub(crate) fn serialize_walk<T: Walk + ?Sized, S: Serializer>(
    value: &T,
    s: S,
    how: Presentation,
) -> Result<S::Ok, S::Error> {
    value.walk(Writer { s, how }).map_err(|e| e.0)
}

/// A serializer's error as a visitor's.
pub(crate) struct Failed<E>(E);

impl<E: serde::ser::Error> From<ValueError> for Failed<E> {
    fn from(e: ValueError) -> Failed<E> {
        Failed(E::custom(e))
    }
}

/// A walk's events as serializer calls.
struct Writer<S> {
    s: S,
    how: Presentation,
}

impl<S: Serializer> Visitor for Writer<S> {
    type Ok = S::Ok;
    type Error = Failed<S::Error>;
    type List = Items<S::SerializeSeq>;
    type Map = Entries<S::SerializeMap>;

    // "There is no value here" -- which a map expresses by not holding
    // the key. Reaching one means a list held it, and `null` would turn
    // "nothing" into "nothing, deliberately".
    fn absent(self) -> Result<S::Ok, Self::Error> {
        Err(Failed(S::Error::custom(
            "an absent value inside a list has no spelling; absent is \
             the ANSWER to a lookup, not a thing a container holds",
        )))
    }

    fn null(self) -> Result<S::Ok, Self::Error> {
        self.s.serialize_unit().map_err(Failed)
    }

    fn bool(self, value: bool) -> Result<S::Ok, Self::Error> {
        self.s.serialize_bool(value).map_err(Failed)
    }

    fn number(self, text: &str) -> Result<S::Ok, Self::Error> {
        // Checked here, since the raw-text policy splices it into the
        // document as it is.
        if !Number::is_number(text) {
            return Err(ValueError::NotANumber.into());
        }
        write_number(self.s, text, self.how.numbers_as()).map_err(Failed)
    }

    // An integer is written native whatever the policy, as its text is.
    fn i64(self, value: i64) -> Result<S::Ok, Self::Error> {
        self.s.serialize_i64(value).map_err(Failed)
    }

    fn u64(self, value: u64) -> Result<S::Ok, Self::Error> {
        match i64::try_from(value) {
            Ok(n) => self.s.serialize_i64(n),
            Err(_) => self.s.serialize_u64(value),
        }
        .map_err(Failed)
    }

    fn string(self, text: &str) -> Result<S::Ok, Self::Error> {
        self.s.serialize_str(text).map_err(Failed)
    }

    fn bytes(self, bytes: &[u8]) -> Result<S::Ok, Self::Error> {
        // A format WITH a byte string gets one, whatever the policy says:
        // native bytes round-trip perfectly, and a spelling could only
        // lose that. The policy is for the formats that have nowhere to
        // put them.
        if !self.s.is_human_readable() {
            return self.s.serialize_bytes(bytes).map_err(Failed);
        }
        match self.how.bytes_as() {
            Bytes::DataUri => self.s.serialize_str(&data_uri(bytes)),
            Bytes::Base64 => self.s.serialize_str(&crate::base64(bytes)),
            Bytes::Array => bytes_as_array(self.s, bytes),
            Bytes::Refuse => Err(S::Error::custom(
                "this document holds a byte string and the presentation refuses to spell one",
            )),
        }
        .map_err(Failed)
    }

    fn list(self, len: Option<usize>) -> Result<Self::List, Self::Error> {
        Ok(Items {
            seq: self.s.serialize_seq(len).map_err(Failed)?,
            how: self.how,
        })
    }

    fn map(self, len: Option<usize>) -> Result<Self::Map, Self::Error> {
        Ok(Entries {
            map: self.s.serialize_map(len).map_err(Failed)?,
            how: self.how,
        })
    }
}

struct Items<Q> {
    seq: Q,
    how: Presentation,
}

impl<Q: SerializeSeq> ListVisitor for Items<Q> {
    type Ok = Q::Ok;
    type Error = Failed<Q::Error>;

    fn item<W: Walk + ?Sized>(&mut self, item: &W) -> Result<(), Self::Error> {
        self.seq
            .serialize_element(&Walked::new(item, self.how))
            .map_err(Failed)
    }

    fn end(self) -> Result<Q::Ok, Self::Error> {
        self.seq.end().map_err(Failed)
    }
}

struct Entries<M> {
    map: M,
    how: Presentation,
}

impl<M: SerializeMap> MapVisitor for Entries<M> {
    type Ok = M::Ok;
    type Error = Failed<M::Error>;

    fn entry<W: Walk + ?Sized>(&mut self, key: &str, value: &W) -> Result<(), Self::Error> {
        self.map
            .serialize_entry(key, &Walked::new(value, self.how))
            .map_err(Failed)
    }

    fn end(self) -> Result<M::Ok, Self::Error> {
        self.map.end().map_err(Failed)
    }
}

/// A byte string as a list of numbers: unambiguous, about four characters
/// a byte.
fn bytes_as_array<S: Serializer>(s: S, bytes: &[u8]) -> Result<S::Ok, S::Error> {
    let mut seq = s.serialize_seq(Some(bytes.len()))?;
    for b in bytes {
        seq.serialize_element(b)?;
    }
    seq.end()
}

/// A number, written as exactly as the format can hold it.
///
/// A guatiao number is the TEXT that declared it, and the whole point is
/// that nothing re-formats it. What a format can be told varies, so this
/// tries in order of exactness:
///
/// 1. **`i64`, then `u64`** — native, exact, and what every format has.
///    This is nearly every number anybody writes.
/// 2. **Raw text**, under [`Numbers::RawText`]. serde_json splices it in
///    verbatim through the reserved token below, so `1e400` and a
///    200-digit integer survive as numbers rather than as strings.
/// 3. **`f64`**, otherwise. Lossy past 53 bits of mantissa, and
///    unavoidably so: a format with no arbitrary-precision number has
///    nowhere else to put one, and a string would change the type a
///    reader sees.
pub(crate) fn write_number<S: Serializer>(
    s: S,
    text: &str,
    how: Numbers,
) -> Result<S::Ok, S::Error> {
    if let Ok(n) = text.parse::<i64>() {
        return s.serialize_i64(n);
    }
    if let Ok(n) = text.parse::<u64>() {
        return s.serialize_u64(n);
    }

    if how == Numbers::RawText {
        // serde_json's own protocol for "this string IS the document": a
        // one-field struct whose name and field name are the token.
        //
        // ONLY serde_json understands it, which is why this is a stated
        // policy and not a guess from `is_human_readable` — TOML and YAML
        // are human-readable too, and they wrote the struct out literally,
        // turning `1.5` into a table with a startling key. Measured.
        let mut raw = s.serialize_struct(RAW_TEXT, 1)?;
        raw.serialize_field(RAW_TEXT, text)?;
        return raw.end();
    }

    match text.parse::<f64>() {
        Ok(n) if n.is_finite() => s.serialize_f64(n),
        _ => Err(S::Error::custom(format!(
            "{text} is a number this format cannot hold; \
             `Numbers::RawText` carries one through serde_json"
        ))),
    }
}

/// serde_json's reserved token for already-formatted JSON text.
const RAW_TEXT: &str = "$serde_json::private::RawValue";
