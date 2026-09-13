// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Writing a value into any serde data format.

#![forbid(unsafe_code)]

use serde::ser::{Error as _, SerializeMap, SerializeSeq, SerializeStruct};
use serde::{Serialize, Serializer};

use guatiao::value::types::{Tag, Value};

use crate::{Bytes, Numbers, Presentation, data_uri};

/// A value, ready for any serde [`Serializer`], carrying the policy for
/// the kinds a format may not have.
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
        fn child<'v>(v: &'v Value, how: Presentation) -> Serializable<'v> {
            Serializable { value: v, how }
        }

        match self.value.tag() {
            // A tag this build does not know. Writing anything at all
            // would put something in a document that a reader believes;
            // refusing says what actually happened.
            Err(_) => Err(S::Error::custom(
                "a value tag this build does not know, which cannot be written",
            )),
            // "There is no value here" — which a map expresses by not
            // holding the key. Reaching one means a list held it, and
            // `null` would turn "nothing" into "nothing, deliberately".
            Ok(Tag::GUATIAO_ABSENT) => Err(S::Error::custom(
                "an absent value inside a list has no spelling; absent is \
                 the ANSWER to a lookup, not a thing a container holds",
            )),
            Ok(Tag::GUATIAO_NULL) => s.serialize_unit(),
            Ok(Tag::GUATIAO_BOOL) => s.serialize_bool(
                self.value
                    .as_bool()
                    .ok_or_else(|| malformed::<S>("a bool"))?,
            ),
            // VERBATIM, through the format's raw-number door where it has
            // one. A guatiao number IS the text that declared it, so
            // reparsing it into an `f64` to write it back is exactly how a
            // 200-digit integer or `1e400` gets silently mangled.
            //
            // `serialize_i64`/`f64` would do that mangling, so neither is
            // used: the text goes out through a newtype every
            // self-describing format renders as a bare number, and a
            // format that cannot gets the text.
            Ok(Tag::GUATIAO_NUMBER) => write_number(
                s,
                self.value
                    .as_number_str()
                    .ok_or_else(|| malformed::<S>("a number"))?,
                self.how.numbers_as(),
            ),
            Ok(Tag::GUATIAO_STRING) => s.serialize_str(
                self.value
                    .as_str()
                    .ok_or_else(|| malformed::<S>("a string"))?,
            ),
            Ok(Tag::GUATIAO_BYTES) => {
                let bytes = self
                    .value
                    .as_bytes()
                    .ok_or_else(|| malformed::<S>("a byte string"))?;
                // A format WITH a byte string gets one, whatever the
                // policy says: native bytes round-trip perfectly, and a
                // spelling could only lose that. The policy is for the
                // formats that have nowhere to put them.
                if s.is_human_readable() {
                    match self.how.bytes_as() {
                        Bytes::DataUri => s.serialize_str(&data_uri(bytes)),
                        Bytes::Base64 => s.serialize_str(&crate::base64(bytes)),
                        Bytes::Array => {
                            let mut seq = s.serialize_seq(Some(bytes.len()))?;
                            for b in bytes {
                                seq.serialize_element(b)?;
                            }
                            seq.end()
                        }
                        Bytes::Refuse => Err(S::Error::custom(
                            "this document holds a byte string and the \
                             presentation refuses to spell one",
                        )),
                    }
                } else {
                    s.serialize_bytes(bytes)
                }
            }
            Ok(Tag::GUATIAO_LIST) => {
                let items = self.value.items().ok_or_else(|| malformed::<S>("a list"))?;
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&child(item, self.how))?;
                }
                seq.end()
            }
            Ok(Tag::GUATIAO_MAP) => {
                let entries = self
                    .value
                    .entries()
                    .ok_or_else(|| malformed::<S>("a map"))?;
                let mut map = s.serialize_map(Some(entries.len()))?;
                for entry in entries {
                    // A key is raw bytes; a serde map key here is a
                    // string. One that is not UTF-8 has no spelling, and a
                    // lossy replacement would silently rename it.
                    let Some(key) = entry.key_str() else {
                        return Err(S::Error::custom(
                            "a map key that is not UTF-8 cannot be written; \
                             guatiao keys are raw bytes",
                        ));
                    };
                    map.serialize_entry(key, &child(entry.value(), self.how))?;
                }
                map.end()
            }
        }
    }
}

/// A node whose tag and payload disagree.
///
/// **Refused, the same way an unknown tag is.** A node this build cannot
/// read is one some other producer built wrong, and writing `0`, `false`
/// or `""` for it would put a value in the document that a reader
/// believes -- a silent substitution, where the unknown-tag arm a few
/// lines up says what actually happened.
fn malformed<S: Serializer>(what: &str) -> S::Error {
    S::Error::custom(format!(
        "a node tagged as {what} whose payload cannot be read, which cannot be written"
    ))
}

/// A number, written as exactly as the format can hold it.
///
/// A guatiao number is the TEXT that declared it, and the whole point is
/// that nothing re-formats it. What a format can be told varies, so this
/// tries in order of exactness:
///
/// 1. **`i64`, then `u64`** — native, exact, and what every format has.
///    This is nearly every number anybody writes.
/// 2. **Raw text**, for a human-readable format. serde_json splices it in
///    verbatim through the reserved token below, so `1e400` and a
///    200-digit integer survive as numbers rather than as strings.
/// 3. **`f64`**, for a binary format with a number too large for `u64`.
///    Lossy, and unavoidably so: MessagePack and CBOR have nowhere to put
///    an arbitrary-precision number, and a string would change the type a
///    reader sees.
fn write_number<S: Serializer>(s: S, text: &str, how: Numbers) -> Result<S::Ok, S::Error> {
    // An integer that fits goes out native whatever the policy: it is
    // exact either way, and every format understands it.
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
        let mut raw = s.serialize_struct(RAW_NUMBER, 1)?;
        raw.serialize_field(RAW_NUMBER, text)?;
        return raw.end();
    }

    match text.parse::<f64>() {
        // Lossy past 53 bits of mantissa, and unavoidably so: this is
        // what a format with no arbitrary-precision number can hold.
        Ok(n) if n.is_finite() => s.serialize_f64(n),
        _ => Err(S::Error::custom(format!(
            "{text} is a number this format cannot hold; \
             `Numbers::RawText` carries one through serde_json"
        ))),
    }
}

/// serde_json's reserved token for already-formatted JSON text.
const RAW_NUMBER: &str = "$serde_json::private::RawValue";

// As in `de`: the unit tests drive JSON, and `tests/every_format.rs`
// covers a format this crate names nowhere.
#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::Numbers;

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
        assert_eq!(json(&Value::bool(true)), "true");
        assert_eq!(json(&Value::int(5900)), "5900");
        assert_eq!(json(&Value::string("hi")), "\"hi\"");

        let mut list = Value::list();
        list.push(1).unwrap();
        list.push("two").unwrap();
        assert_eq!(json(&list), "[1,\"two\"]");
    }

    /// A map is written in the order keys were set, never sorted.
    ///
    /// A reader that wants them sorted can sort them; a writer that sorted
    /// them would destroy an order somebody chose and no reader could get
    /// it back.
    #[test]
    fn a_map_keeps_the_order_it_was_built_in() {
        let mut map = Value::map();
        map.set("zebra", 1).unwrap();
        map.set("aardvark", 2).unwrap();
        assert_eq!(json(&map), r#"{"zebra":1,"aardvark":2}"#);
    }

    /// THE PROPERTY THIS MODEL EXISTS FOR. A number is its text, and no
    /// `f64` ever touches it.
    #[test]
    fn a_number_survives_that_no_f64_could_hold() {
        let huge = "123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(json(&Value::number(huge).unwrap()), huge);
        assert_eq!(json(&Value::number("1e400").unwrap()), "1e400");
        // And a spelling is preserved rather than normalised: `1.10` is
        // not `1.1`, because nothing re-formatted it.
        assert_eq!(json(&Value::number("1.10").unwrap()), "1.10");
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
        let v = Value::number("1.10").unwrap();

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
        assert_eq!(json(&Value::string("a\"b\\c")), r#""a\"b\\c""#);
        assert_eq!(json(&Value::string("tab\there")), r#""tab\there""#);
        assert_eq!(json(&Value::string("café ☕")), "\"café ☕\"");
    }

    #[test]
    fn bytes_take_the_presentation_they_are_given() {
        let v = Value::bytes(&[0xde, 0xad]);
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
        let v = Value::bytes(&[0xde, 0xad]);
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
        let mut list = Value::list();
        list.push(Value::absent()).unwrap();
        assert!(serde_json::to_string(&Serializable::from(&list)).is_err());
    }
}
