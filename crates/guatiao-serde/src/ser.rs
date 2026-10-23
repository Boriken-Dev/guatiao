// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a value into any serde data format: the value's walk, through
//! the bridge every walk takes (`bridge::Walked`).

#![forbid(unsafe_code)]

use serde::{Serialize, Serializer};

use guatiao::value::types::Value;

use crate::Presentation;
use crate::bridge::serialize_walk;

/// A value, ready for any serde [`Serializer`], carrying the policy for
/// the kinds a format may not have.
///
/// The same as [`Walked`](crate::Walked) over a value, named for the
/// common case. A value nested deeper than
/// [`MAX_DEPTH`](guatiao::MAX_DEPTH) is refused, as every walk refuses
/// it.
#[derive(Debug, Clone, Copy)]
pub struct Serializable<'a> {
    value: &'a Value,
    how: Presentation,
}

impl<'a> Serializable<'a> {
    /// A value, written the way you say.
    ///
    /// [`From`] is the same thing with the default presentation, and is
    /// what to reach for when you have no policy to state.
    pub fn new(value: &'a Value, how: Presentation) -> Serializable<'a> {
        Serializable { value, how }
    }

    /// What it will write.
    pub fn value(&self) -> &'a Value {
        self.value
    }

    /// How it will write it.
    pub fn presentation(&self) -> Presentation {
        self.how
    }
}

/// A value with the default presentation.
///
/// The standard trait rather than a `to_serde` of our own, so
/// `serde_json::to_string(&value.into())` works and anything taking
/// `impl Into<Serializable>` takes a bare value.
impl<'a> From<&'a Value> for Serializable<'a> {
    fn from(value: &'a Value) -> Serializable<'a> {
        Serializable {
            value,
            how: Presentation::new(),
        }
    }
}

impl Serialize for Serializable<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_walk(self.value, s, self.how)
    }
}

// As in `de`: the unit tests drive JSON, and `tests/every_format.rs`
// covers a format this crate names nowhere.
#[cfg(all(test, feature = "json"))]
mod tests {
    use guatiao::value::types::{Buffer, List, Map, Number, Text};

    use super::*;
    use crate::{Bytes, Numbers};

    /// JSON, with JSON's own number policy — which `text::json` sets for
    /// a caller and which the default deliberately does not, since the
    /// token means nothing to any other format.
    fn json(v: &Value) -> String {
        let how = Presentation::new().numbers(Numbers::RawText);
        serde_json::to_string(&Serializable::new(v, how)).expect("this value is writable")
    }

    #[test]
    fn the_shapes_a_document_is_made_of() {
        assert_eq!(json(&Value::null()), "null");
        assert_eq!(json(&Value::from(true)), "true");
        assert_eq!(json(&Value::from(5900i64)), "5900");
        assert_eq!(json(&Value::from(Text::new("hi"))), "\"hi\"");

        let mut list = List::new();
        list.push(1);
        list.push("two");
        assert_eq!(json(&Value::from(list)), "[1,\"two\"]");
    }

    /// A map is written in the order keys were set, never sorted.
    ///
    /// A reader that wants them sorted can sort them; a writer that sorted
    /// them would destroy an order somebody chose and no reader could get
    /// it back.
    #[test]
    fn a_map_keeps_the_order_it_was_built_in() {
        let mut map = Map::new();
        map.set("zebra", 1);
        map.set("aardvark", 2);
        assert_eq!(json(&Value::from(map)), r#"{"zebra":1,"aardvark":2}"#);
    }

    /// THE PROPERTY THIS MODEL EXISTS FOR. A number is its text, and no
    /// `f64` ever touches it.
    #[test]
    fn a_number_survives_that_no_f64_could_hold() {
        let huge = "123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(json(&Value::from(Number::new(huge).unwrap())), huge);
        assert_eq!(json(&Value::from(Number::new("1e400").unwrap())), "1e400");
        // And a spelling is preserved rather than normalised: `1.10` is
        // not `1.1`, because nothing re-formatted it.
        assert_eq!(json(&Value::from(Number::new("1.10").unwrap())), "1.10");
    }

    /// **Verbatim is a POLICY, not the default.**
    ///
    /// `Numbers::RawText` is what splices a number's own text into the
    /// document, and only `text::json` sets it. The default
    /// `Presentation` -- which is what `Serializable::from(&value)` carries
    /// -- hands the format an `f64` for anything past `i64`/`u64`, so
    /// `1.10` arrives as `1.1`. Both are pinned here because the
    /// difference is invisible until a document is compared byte for byte.
    #[test]
    fn a_spelling_survives_only_under_the_raw_text_policy() {
        let v = Value::from(Number::new("1.10").unwrap());

        assert_eq!(
            crate::text::json::to_string(&v, Presentation::new()).unwrap(),
            "1.10",
            "`text::json` sets `Numbers::RawText`, which writes the text itself"
        );
        assert_eq!(
            serde_json::to_string(&Serializable::from(&v)).unwrap(),
            "1.1",
            "the default presentation goes through an `f64`, which has no `1.10`"
        );
    }

    #[test]
    fn escaping_is_serdes_and_needs_no_second_answer() {
        assert_eq!(json(&Value::from(Text::new("a\"b\\c"))), r#""a\"b\\c""#);
        assert_eq!(json(&Value::from(Text::new("tab\there"))), r#""tab\there""#);
        assert_eq!(json(&Value::from(Text::new("café ☕"))), "\"café ☕\"");
    }

    #[test]
    fn bytes_take_the_presentation_they_are_given() {
        let v = Value::from(Buffer::new(&[0xde, 0xad]));
        let one = |how| serde_json::to_string(&Serializable::new(&v, how)).ok();
        assert_eq!(
            one(Presentation::new()).as_deref(),
            Some("\"data:;base64,3q0=\"")
        );
        assert_eq!(
            one(Presentation::new().bytes(Bytes::Base64)).as_deref(),
            Some("\"3q0=\"")
        );
        assert_eq!(
            one(Presentation::new().bytes(Bytes::Array)).as_deref(),
            Some("[222,173]")
        );
        assert_eq!(one(Presentation::new().bytes(Bytes::Refuse)), None);
    }

    /// A format WITH a byte string gets one, whatever the policy says.
    #[test]
    fn a_binary_format_gets_native_bytes() {
        let v = Value::from(Buffer::new(&[0xde, 0xad]));
        // MessagePack's bin8: 0xc4, length, then the bytes.
        let packed = rmp_serde::to_vec(&Serializable::new(
            &v,
            Presentation::new().bytes(Bytes::Refuse),
        ))
        .expect("a format with bytes never reaches the policy");
        assert_eq!(packed, vec![0xc4, 0x02, 0xde, 0xad]);
    }

    /// Absent is the answer to a lookup, not something a container holds.
    #[test]
    fn an_absent_value_has_no_spelling() {
        let mut list = List::new();
        list.push(Value::absent());
        let list = Value::from(list);
        assert!(serde_json::to_string(&Serializable::from(&list)).is_err());
    }
}
