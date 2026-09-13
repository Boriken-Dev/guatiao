// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a value out of any serde data format.
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

use std::fmt;

use serde::Deserializer;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};

use guatiao::value::alloc::Alloc;
use guatiao::value::types::Value;

use crate::{Presentation, from_data_uri};

/// Reads one value, building it through an allocator.
///
/// ```
/// use guatiao::value::alloc::Alloc;
/// use guatiao_serde::ValueSeed;
/// use serde::de::DeserializeSeed;
///
/// let mut de = serde_json::Deserializer::from_str(r#"{"port":5900}"#);
/// let value = ValueSeed::new(Alloc::rust()).deserialize(&mut de).unwrap();
/// assert_eq!(value.get("port").and_then(|v| v.as_number_str()), Some("5900"));
/// ```
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

impl<'de> Visitor<'de> for ValueSeed {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::null())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::null())
    }

    fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::bool(v))
    }

    // Every integer and float arrives as text, which is what a guatiao
    // number is. `to_string` on an `i64` or `u64` is exact; on an `f64` it
    // is the shortest text that round-trips, which is the best any format
    // that already handed us an `f64` can offer.
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        number(self.alloc, &v.to_string())
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        number(self.alloc, &v.to_string())
    }

    fn visit_i128<E: serde::de::Error>(self, v: i128) -> Result<Value, E> {
        number(self.alloc, &v.to_string())
    }

    fn visit_u128<E: serde::de::Error>(self, v: u128) -> Result<Value, E> {
        number(self.alloc, &v.to_string())
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        if !v.is_finite() {
            // JSON has no infinity or NaN, and neither has this model: a
            // number is text matching RFC 8259 §6.
            return Err(E::custom(format!("{v} is not a JSON number")));
        }
        // `{:?}` on an `f64` is the shortest round-tripping text, and it
        // writes `1.0` rather than `1` so the value stays a float.
        number(self.alloc, &format!("{v:?}"))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        // A data URI becomes bytes only when the caller asked for it. On,
        // a byte string survives a JSON round trip; also on, a string
        // somebody wrote that looks like a data URI silently becomes
        // bytes. Off is the safer default and the one that cannot
        // surprise.
        if self.how.reads_data_uris()
            && let Some(bytes) = from_data_uri(v)
        {
            return Value::bytes_in(self.alloc, &bytes).map_err(E::custom);
        }
        Value::string_in(self.alloc, v).map_err(E::custom)
    }

    fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Value, E> {
        Value::bytes_in(self.alloc, v).map_err(E::custom)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut list = Value::list_in(self.alloc);
        while let Some(item) = seq.next_element_seed(self)? {
            list.push(item).map_err(A::Error::custom)?;
        }
        Ok(list)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Value::map_in(self.alloc);
        // A key arrives as a `String` rather than borrowed, because a
        // format may have had to unescape it and a borrowed key would then
        // point at a buffer that does not outlive the call.
        while let Some(key) = access.next_key::<String>()? {
            let value = access.next_value_seed(self)?;
            // A repeated key REPLACES, which is what `set` does and what a
            // reader of the resulting map would expect. Refusing would be
            // defensible; silently keeping the first would not.
            map.set(&key, value).map_err(A::Error::custom)?;
        }
        Ok(map)
    }
}

/// A number from its text, or the error saying it is not one.
fn number<E: serde::de::Error>(alloc: Alloc, text: &str) -> Result<Value, E> {
    Value::number_in(alloc, text).map_err(E::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bytes, to_serde, to_serde_with};

    fn read(json: &str) -> Value {
        let mut de = serde_json::Deserializer::from_str(json);
        ValueSeed::new(Alloc::rust())
            .deserialize(&mut de)
            .expect("valid json")
    }

    fn write(v: &Value) -> String {
        serde_json::to_string(&to_serde(v)).expect("writable")
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

    /// **WHAT THE SERDE ROUTE COSTS, measured rather than described.**
    ///
    /// Writing a number is exact: the text goes out verbatim, so `1.10`
    /// and a 200-digit integer leave this crate unchanged. READING one is
    /// not, and cannot be — `serde_json` without `arbitrary_precision`
    /// resolves any number past `i64`/`u64` to an `f64` before a visitor
    /// is ever called, so the spelling is gone before this crate sees it.
    ///
    /// That feature is deliberately not enabled here: cargo unifies
    /// features across a build, so turning it on would change
    /// `serde_json::Value` for every other crate in a consumer's graph.
    ///
    /// A format that hands over the text — or a `serde_json` built with
    /// that feature — reads exactly. This pins the limit so that a change
    /// to it is a decision rather than a surprise.
    #[test]
    fn a_number_past_u64_loses_its_spelling_on_the_way_in() {
        // Exact in, exact out, for everything that fits.
        assert_eq!(read("5900").as_number_str(), Some("5900"));
        assert_eq!(
            read("18446744073709551615").as_number_str(),
            Some("18446744073709551615")
        );

        // And past that, an f64's shortest round-tripping spelling.
        assert_eq!(read("1.10").as_number_str(), Some("1.1"));
        assert_eq!(
            read("123456789012345678901234567890").as_number_str(),
            Some("1.2345678901234568e29")
        );

        // WRITING is exact whatever the magnitude, which is the half this
        // crate does control.
        let huge = "123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(write(&Value::number(huge).unwrap()), huge);
        assert_eq!(write(&Value::number("1.10").unwrap()), "1.10");
        assert_eq!(write(&Value::number("1e400").unwrap()), "1e400");
    }

    /// Absent by default, on when asked: the trade is the caller's.
    #[test]
    fn a_data_uri_becomes_bytes_only_when_asked() {
        let text = r#""data:;base64,3q0=""#;
        assert_eq!(read(text).as_str(), Some("data:;base64,3q0="));

        let mut de = serde_json::Deserializer::from_str(text);
        let asked = ValueSeed::with(Alloc::rust(), Presentation::new().reading_data_uris())
            .deserialize(&mut de)
            .unwrap();
        assert_eq!(asked.as_bytes(), Some(&[0xde, 0xad][..]));
    }

    /// With that on, a byte string survives a JSON round trip.
    #[test]
    fn bytes_round_trip_through_json_when_both_sides_agree() {
        let how = Presentation::new().reading_data_uris();
        let original = Value::bytes(&[1, 2, 255]);
        let text = serde_json::to_string(&to_serde_with(&original, how)).unwrap();

        let mut de = serde_json::Deserializer::from_str(&text);
        let back = ValueSeed::with(Alloc::rust(), how)
            .deserialize(&mut de)
            .unwrap();
        assert_eq!(back.as_bytes(), Some(&[1u8, 2, 255][..]));
    }

    /// And through a binary format with no agreement needed at all.
    #[test]
    fn bytes_round_trip_through_a_binary_format() {
        let original = Value::bytes(&[1, 2, 255]);
        let packed = rmp_serde::to_vec(&to_serde(&original)).unwrap();
        let mut de = rmp_serde::Deserializer::new(&packed[..]);
        let back = ValueSeed::new(Alloc::rust()).deserialize(&mut de).unwrap();
        assert_eq!(back.as_bytes(), Some(&[1u8, 2, 255][..]));
    }

    #[test]
    fn a_repeated_key_replaces_rather_than_duplicating() {
        let v = read(r#"{"a":1,"a":2}"#);
        assert_eq!(v.entries().map(<[_]>::len), Some(1));
        assert_eq!(v.get("a").and_then(Value::as_number_str), Some("2"));
    }

    #[test]
    fn a_number_this_model_cannot_hold_is_refused() {
        // serde_json will not produce these, but another format can.
        let mut de = serde_json::Deserializer::from_str("1");
        let seed = ValueSeed::new(Alloc::rust());
        assert!(seed.deserialize(&mut de).is_ok());
        assert!(number::<serde_json::Error>(Alloc::rust(), "not a number").is_err());
    }

    #[test]
    fn bytes_as_an_array_reads_back_as_a_list_of_numbers() {
        // Unambiguous to write, and honest about what comes back: the
        // reader sees a list, because that is what the document says.
        let v = Value::bytes(&[1, 2]);
        let text =
            serde_json::to_string(&to_serde_with(&v, Presentation::new().bytes(Bytes::Array)))
                .unwrap();
        assert_eq!(text, "[1,2]");
        assert_eq!(read(&text).items().map(<[_]>::len), Some(2));
    }
}
