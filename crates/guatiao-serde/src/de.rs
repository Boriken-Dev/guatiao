// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a value out of any serde data format: the document's events,
//! through the bridge every sink takes, into a tree.
//!
//! # Why a seed rather than `Deserialize`
//!
//! Every guatiao container carries the allocator that made it, so building
//! one means naming an allocator — and `Deserialize::deserialize` takes no
//! arguments to name it with. [`ValueSeed`] is serde's own answer:
//! `DeserializeSeed` exists to carry state into a deserialisation, and an
//! allocator is exactly that.
//!
//! It is also what lets a host read a document straight into its own
//! arena, which is the reason the core's constructors have an `_in` form
//! at all.

#![forbid(unsafe_code)]

use alloc::fmt;
use core::cell::Cell;

use serde::Deserializer;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};

use guatiao::value::alloc::Alloc;
use guatiao::value::types::Value;
use guatiao::value::visit::TreeSink;

use crate::Presentation;
use crate::bridge::ToSink;

/// Reads one value, building it through an allocator.
///
/// ```
/// use guatiao::Map;
/// use guatiao::value::alloc::Alloc;
/// use guatiao_serde::ValueSeed;
/// use serde::de::DeserializeSeed;
///
/// let mut de = serde_json::Deserializer::from_str(r#"{"port":5900}"#);
/// let value = ValueSeed::new(Alloc::rust()).deserialize(&mut de).unwrap();
/// let map: &Map = (&value).try_into().unwrap();
/// let port: u32 = map.required("port").unwrap().try_into().unwrap();
/// assert_eq!(port, 5900);
/// ```
///
/// **A document is somebody else's input**, and a reader that recursed
/// once per nested container would overflow the stack on one that nests
/// deeply enough. `serde_json` caps its own recursion at 128; a serde
/// format need not, and this crate exists to read them all. A document
/// nested past [`MAX_DEPTH`](guatiao::MAX_DEPTH) containers is refused.
#[derive(Debug, Clone, Copy)]
pub struct ValueSeed {
    alloc: Alloc,
    how: Presentation,
}

impl ValueSeed {
    /// Build through `alloc`, reading with the default presentation.
    pub const fn new(alloc: Alloc) -> ValueSeed {
        ValueSeed {
            alloc,
            how: Presentation::new(),
        }
    }

    /// The same, told how the document was written.
    pub const fn with(alloc: Alloc, how: Presentation) -> ValueSeed {
        ValueSeed { alloc, how }
    }

    /// Runs `feed` against a tree sink, and answers the tree or the
    /// refusal as the format's error.
    fn build<E: serde::de::Error>(
        self,
        feed: impl FnOnce(ToSink<'_, TreeSink>) -> Result<(), E>,
    ) -> Result<Value, E> {
        let mut sink = TreeSink::new_in(self.alloc);
        let failed = Cell::new(None);
        let fed = feed(ToSink {
            sink: &mut sink,
            failed: &failed,
            depth: 0,
            how: self.how,
        });
        match (fed, failed.take()) {
            (Ok(()), _) => sink.into_value().map_err(E::custom),
            (Err(_), Some(refused)) => Err(E::custom(refused)),
            (Err(format), None) => Err(format),
        }
    }
}

/// One value out of any serde deserialiser, through the crate's allocator.
///
/// [`ValueSeed`] is the form that names one.
pub fn from_serde<'de, D: Deserializer<'de>>(de: D) -> Result<Value, D::Error> {
    ValueSeed::new(Alloc::rust()).deserialize(de)
}

impl<'de> DeserializeSeed<'de> for ValueSeed {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }
}

/// Each visit is the sink bridge's, into a tree.
macro_rules! into_tree {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method<E: serde::de::Error>(self, $($arg: $ty),*) -> Result<Value, E> {
            self.build(|sink| sink.$method($($arg),*))
        }
    )*};
}

impl<'de> Visitor<'de> for ValueSeed {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any value")
    }

    into_tree! {
        visit_unit();
        visit_none();
        visit_bool(v: bool);
        visit_i64(v: i64);
        visit_u64(v: u64);
        visit_i128(v: i128);
        visit_u128(v: u128);
        visit_f64(v: f64);
        visit_str(v: &str);
        visit_bytes(v: &[u8]);
    }

    fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Value, A::Error> {
        self.build(|sink| sink.visit_seq(seq))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Value, A::Error> {
        self.build(|sink| sink.visit_map(map))
    }
}

/// serde_json's reserved key for a number carried as its own text.
///
/// **Not the token the writer uses**, which is the mistake this comment
/// exists to stop somebody repeating. The writer splices raw JSON through
/// `$serde_json::private::RawValue`, which may hold any document at all;
/// an arbitrary-precision NUMBER arrives under the token below, and only
/// that one can safely be read as a number. Measured, after assuming they
/// were the same and watching every number become a map.
///
/// # A number wearing a map's clothes
///
/// With `serde_json/arbitrary_precision` on, `deserialize_any` reports
/// every number as a one-entry map under this key, whose value is the
/// number's text. Reading it is NOT behind this crate's `exact-numbers`
/// feature, and that is the point: cargo unifies features, so ANY crate in
/// a consumer's graph can turn that on. A reader that did not know the
/// token would then quietly turn every number in every document into a
/// map — a bug with no error attached, caused by a dependency this crate
/// never named.
///
/// A map whose first key is this token is read as a number
/// unconditionally, and the rest of it is not drained: no format produces
/// one by accident, and reading the value first to decide would mean
/// buffering every map's first value. A format whose deserializer insists
/// on the remaining entries (JSON, YAML) refuses such a document when it
/// has any; one that does not (TOML) drops them.
pub(crate) const RAW_NUMBER: &str = "$serde_json::private::Number";

/// The `toml` crate's reserved key for a date or time carried as its
/// text: all four forms (offset date-time, local date-time, local date,
/// local time) arrive as a one-entry map under it, and read as the text
/// they were written as rather than as a table with a startling key.
pub(crate) const TOML_DATETIME: &str = "$__toml_private_datetime";

/// The refusal a number outside the grammar gets, kept for the test that
/// pins it.
#[cfg(test)]
fn number<E: serde::de::Error>(alloc: Alloc, text: &str) -> Result<Value, E> {
    guatiao::Number::new_in(alloc, text)
        .map(Value::from)
        .map_err(|e: guatiao::ValueError| E::custom(e))
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use guatiao::value::convert::TryAsRef;
    use guatiao::value::types::{Buffer, List, Map, Number};

    use super::*;
    use crate::{Bytes, Serializable};

    fn read(json: &str) -> Value {
        let mut de = serde_json::Deserializer::from_str(json);
        ValueSeed::new(Alloc::rust())
            .deserialize(&mut de)
            .expect("valid json")
    }

    /// As in `ser`: JSON's own number policy, which is what makes a
    /// number's spelling survive.
    fn write(v: &Value) -> String {
        let how = Presentation::new().numbers(crate::Numbers::RawText);
        serde_json::to_string(&Serializable::new(v, how)).expect("writable")
    }

    /// TEXT ROUND-TRIPS, for everything a format can hand over intact.
    ///
    /// The exceptions are numbers past `i64`/`u64`, and they have their
    /// own test below saying exactly what happens to them instead.
    #[test]
    fn text_round_trips() {
        for text in [
            "null",
            "true",
            "5900",
            "-1",
            "18446744073709551615",
            r#""hi""#,
            r#""a\"b\\c""#,
            "[]",
            "{}",
            "[1,\"two\",null]",
            r#"{"zebra":1,"aardvark":{"deep":[true]}}"#,
        ] {
            assert_eq!(write(&read(text)), text, "round trip of {text}");
        }
    }

    /// **A number keeps its SPELLING, not merely its magnitude.**
    ///
    /// The `exact-numbers` feature turns on `serde_json/arbitrary_precision`,
    /// so a number arrives as its own text rather than as an `f64`. That is
    /// more than an arbitrary-precision number TYPE preserves: those hold
    /// a value and normalise `1.10` to `1.1`, where this model holds the
    /// text and does not.
    #[test]
    #[cfg(feature = "exact-numbers")]
    fn a_number_keeps_its_spelling_through_a_round_trip() {
        // Exact in, exact out, for everything that fits.
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&read("5900")).map(AsRef::<str>::as_ref),
            Some("5900")
        );
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&read("18446744073709551615")).map(AsRef::<str>::as_ref),
            Some("18446744073709551615")
        );

        // And past that, through the token: exact, spelling included.
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&read("1.10")).map(AsRef::<str>::as_ref),
            Some("1.10")
        );
        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&read("123456789012345678901234567890"))
                .map(AsRef::<str>::as_ref),
            Some("123456789012345678901234567890")
        );
        // `1e400` comes back `1e+400`: serde_json writes the exponent's
        // sign in. The MAGNITUDE is exact, which is what no `f64` could
        // have done; the spelling of an exponent is the format's to
        // normalise, and this is what it chose.

        assert_eq!(
            TryAsRef::<Number>::try_as_ref(&read("1e400")).map(AsRef::<str>::as_ref),
            Some("1e+400")
        );

        // And writing is exact whatever the magnitude.
        let huge = "123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(write(&Value::from(Number::new(huge).unwrap())), huge);
        assert_eq!(write(&Value::from(Number::new("1.10").unwrap())), "1.10");
        assert_eq!(write(&Value::from(Number::new("1e400").unwrap())), "1e400");
    }

    /// Without `exact-numbers`, `serde_json` resolves anything past
    /// `i64`/`u64` to an `f64` before a visitor is called, so the spelling
    /// is gone before this crate can see it: `1.10` reads as `1.1` and a
    /// number no `f64` holds is refused by the parser.
    #[test]
    #[cfg(not(feature = "exact-numbers"))]
    fn without_exact_numbers_a_wide_number_reads_through_an_f64() {
        let text = |json: &str| {
            TryAsRef::<Number>::try_as_ref(&read(json))
                .map(AsRef::<str>::as_ref)
                .map(str::to_string)
        };
        assert_eq!(text("5900").as_deref(), Some("5900"));
        assert_eq!(
            text("18446744073709551615").as_deref(),
            Some("18446744073709551615"),
            "an integer that fits a u64 is exact either way"
        );
        assert_eq!(text("1.10").as_deref(), Some("1.1"));
        assert_eq!(
            text("123456789012345678901234567890").as_deref(),
            Some("1.2345678901234568e29")
        );
        let mut de = serde_json::Deserializer::from_str("1e400");
        assert!(ValueSeed::new(Alloc::rust()).deserialize(&mut de).is_err());

        // Writing is exact all the same: that is a policy of the writer.
        let huge = "123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(write(&Value::from(Number::new(huge).unwrap())), huge);
    }

    /// Absent by default, on when asked: the trade is the caller's.
    #[test]
    fn a_data_uri_becomes_bytes_only_when_asked() {
        let text = r#""data:;base64,3q0=""#;
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&read(text)),
            Some("data:;base64,3q0=")
        );

        let mut de = serde_json::Deserializer::from_str(text);
        let asked = ValueSeed::with(Alloc::rust(), Presentation::new().reading_data_uris())
            .deserialize(&mut de)
            .unwrap();
        assert_eq!(
            TryAsRef::<[u8]>::try_as_ref(&asked),
            Some(&[0xde, 0xad][..])
        );
    }

    /// With that on, a byte string survives a JSON round trip.
    #[test]
    fn bytes_round_trip_through_json_when_both_sides_agree() {
        let how = Presentation::new().reading_data_uris();
        let original = Value::from(Buffer::new(&[1, 2, 255]));
        let text = serde_json::to_string(&Serializable::new(&original, how)).unwrap();

        let mut de = serde_json::Deserializer::from_str(&text);
        let back = ValueSeed::with(Alloc::rust(), how)
            .deserialize(&mut de)
            .unwrap();
        assert_eq!(
            TryAsRef::<[u8]>::try_as_ref(&back),
            Some(&[1u8, 2, 255][..])
        );
    }

    /// And through a binary format with no agreement needed at all.
    #[test]
    fn bytes_round_trip_through_a_binary_format() {
        let original = Value::from(Buffer::new(&[1, 2, 255]));
        let packed = rmp_serde::to_vec(&Serializable::from(&original)).unwrap();
        let mut de = rmp_serde::Deserializer::new(&packed[..]);
        let back = ValueSeed::new(Alloc::rust()).deserialize(&mut de).unwrap();
        assert_eq!(
            TryAsRef::<[u8]>::try_as_ref(&back),
            Some(&[1u8, 2, 255][..])
        );
    }

    #[test]
    fn a_repeated_key_replaces_rather_than_duplicating() {
        let v = read(r#"{"a":1,"a":2}"#);
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&v)
                .map(Map::entries)
                .map(<[_]>::len),
            Some(1)
        );
        assert_eq!(
            TryAsRef::<Map>::try_as_ref(&v)
                .and_then(|m| m.get("a"))
                .and_then(TryAsRef::<Number>::try_as_ref)
                .map(AsRef::<str>::as_ref),
            Some("2")
        );
    }

    #[test]
    fn a_number_this_model_cannot_hold_is_refused() {
        // serde_json will not produce these, but another format can.
        let mut de = serde_json::Deserializer::from_str("1");
        let seed = ValueSeed::new(Alloc::rust());
        assert!(seed.deserialize(&mut de).is_ok());
        assert!(number::<serde_json::Error>(Alloc::rust(), "not a number").is_err());
    }

    /// **A document nests only as deep as a value can.**
    ///
    /// Driven through MessagePack rather than JSON on purpose:
    /// `serde_json` caps its own recursion at 128 and would refuse this
    /// before a visitor ever ran, so a JSON test would pin serde_json's
    /// limit instead of this crate's. A nested one-element array is one
    /// byte each in MessagePack, and nothing in that reader stops.
    #[test]
    fn a_document_nested_past_the_limit_is_refused() {
        fn read(depth: usize) -> Result<Value, rmp_serde::decode::Error> {
            // `0x91` opens an array of one element; `0xc0` is nil.
            let mut packed = vec![0x91u8; depth];
            packed.push(0xc0);
            let mut de = rmp_serde::Deserializer::new(&packed[..]);
            ValueSeed::new(Alloc::rust()).deserialize(&mut de)
        }

        assert!(read(300).is_err(), "300 containers deep is refused");
        assert!(
            read(100).is_ok(),
            "and the limit is not so eager that an ordinary document trips it"
        );
    }

    #[test]
    fn bytes_as_an_array_reads_back_as_a_list_of_numbers() {
        // Unambiguous to write, and honest about what comes back: the
        // reader sees a list, because that is what the document says.
        let v = Value::from(Buffer::new(&[1, 2]));
        let text = serde_json::to_string(&Serializable::new(
            &v,
            Presentation::new().bytes(Bytes::Array),
        ))
        .unwrap();
        assert_eq!(text, "[1,2]");
        assert_eq!(
            TryAsRef::<List>::try_as_ref(&read(&text))
                .map(|list| &list[..])
                .map(<[_]>::len),
            Some(2)
        );
    }
}
