// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Any serde format's document, handed to a guatiao [`Sink`] event by
//! event: the wire encoder (a tape), a tree, a C visitor.
//!
//! A container's length is never given to the sink, since a serde
//! format's is only a hint and a sink takes a given one as a promise.

#![forbid(unsafe_code)]

use core::cell::Cell;
use core::fmt;

use serde::Deserializer;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess};

use guatiao::MAX_DEPTH;
use guatiao::value::visit::Sink;

use super::BridgeError;
use super::number::Text;
use super::walker::{FirstKey, Key, Reserved};
use crate::{Presentation, from_data_uri};

/// Hands every event of the document `de` holds to `sink`, with no tree
/// built. The format's refusal is [`BridgeError::Format`], the sink's
/// [`BridgeError::Refused`].
///
/// ```
/// use guatiao::value::visit::TreeSink;
/// use guatiao_serde::{Presentation, deserialize_into};
///
/// let mut sink = TreeSink::new();
/// let mut de = serde_json::Deserializer::from_str(r#"{"port": 5900}"#);
/// deserialize_into(&mut de, &mut sink, Presentation::new()).unwrap();
/// let value = sink.into_value()?;
/// # Ok::<(), guatiao::ValueError>(())
/// ```
pub fn deserialize_into<'de, D: Deserializer<'de>, S: Sink + ?Sized>(
    de: D,
    sink: &mut S,
    how: Presentation,
) -> Result<(), BridgeError<D::Error, S::Error>> {
    let failed = Cell::new(None);
    let fed = de.deserialize_any(ToSink {
        sink,
        failed: &failed,
        depth: 0,
        how,
    });
    match (fed, failed.take()) {
        (Ok(()), _) => Ok(()),
        (Err(_), Some(refused)) => Err(BridgeError::Refused(refused)),
        (Err(format), None) => Err(BridgeError::Format(format)),
    }
}

/// A sink's events as a serde visitor's, `depth` containers down.
pub(crate) struct ToSink<'a, S: Sink + ?Sized> {
    pub(crate) sink: &'a mut S,
    /// The sink's refusal, kept while serde unwinds.
    pub(crate) failed: &'a Cell<Option<S::Error>>,
    pub(crate) depth: u32,
    pub(crate) how: Presentation,
}

/// A sink's answer as serde's: a refusal waits in `failed`.
fn fed<T, E: de::Error>(failed: &Cell<Option<T>>, r: Result<(), T>) -> Result<(), E> {
    r.map_err(|refused| {
        failed.set(Some(refused));
        E::custom("")
    })
}

impl<'a, S: Sink + ?Sized> ToSink<'a, S> {
    /// Refuses a container opened past the limit, in the words a tree
    /// read from a document has always used.
    fn room<E: de::Error>(&self) -> Result<(), E> {
        if self.depth >= MAX_DEPTH {
            return Err(E::custom(format!(
                "a document nested more than {MAX_DEPTH} containers deep, \
                 which is deeper than a value can be built"
            )));
        }
        Ok(())
    }

    fn inside(&mut self) -> ToSink<'_, S> {
        ToSink {
            sink: &mut *self.sink,
            failed: self.failed,
            depth: self.depth + 1,
            how: self.how,
        }
    }
}

impl<'de, S: Sink + ?Sized> DeserializeSeed<'de> for ToSink<'_, S> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de, S: Sink + ?Sized> de::Visitor<'de> for ToSink<'_, S> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        fed(self.failed, self.sink.null())
    }

    fn visit_none<E: de::Error>(self) -> Result<(), E> {
        fed(self.failed, self.sink.null())
    }

    fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_any(self)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_any(self)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<(), E> {
        fed(self.failed, self.sink.bool(v))
    }

    // Every integer arrives as its text, which is exact; a float as the
    // shortest text that reads back the same, keeping a fraction.
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<(), E> {
        fed(
            self.failed,
            self.sink.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<(), E> {
        fed(
            self.failed,
            self.sink.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_i128<E: de::Error>(self, v: i128) -> Result<(), E> {
        fed(
            self.failed,
            self.sink.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> Result<(), E> {
        fed(
            self.failed,
            self.sink.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<(), E> {
        if !v.is_finite() {
            // JSON has no infinity or NaN, and neither has this model: a
            // number is text matching RFC 8259 §6.
            return Err(E::custom(format!("{v} is not a JSON number")));
        }
        fed(
            self.failed,
            self.sink.number(Text::of(format_args!("{v:?}")).as_str()),
        )
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<(), E> {
        // A data URI becomes bytes only when the caller asked for it: on,
        // a byte string survives a JSON round trip; also on, a string
        // somebody wrote that looks like one silently becomes bytes.
        if self.how.reads_data_uris()
            && let Some(bytes) = from_data_uri(v)
        {
            return fed(self.failed, self.sink.bytes(&bytes));
        }
        fed(self.failed, self.sink.string(v))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<(), E> {
        fed(self.failed, self.sink.bytes(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<(), A::Error> {
        self.room()?;
        fed(self.failed, self.sink.list_begin(None))?;
        while seq.next_element_seed(self.inside())?.is_some() {}
        fed(self.failed, self.sink.list_end())
    }

    fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<(), A::Error> {
        self.room()?;
        // A number or a date wearing a map's clothes: see `de.rs`.
        let first = match map.next_key_seed(FirstKey)? {
            Some(Key::Number) => {
                let sink = &mut *self.sink;
                let r = map.next_value_seed(Reserved(|text: &str| sink.number(text)))?;
                return fed(self.failed, r);
            }
            Some(Key::Datetime) => {
                let sink = &mut *self.sink;
                let r = map.next_value_seed(Reserved(|text: &str| sink.string(text)))?;
                return fed(self.failed, r);
            }
            Some(Key::Plain(key)) => Some(key),
            None => None,
        };
        fed(self.failed, self.sink.map_begin(None))?;
        if let Some(key) = first {
            fed(self.failed, self.sink.key(&key))?;
            map.next_value_seed(self.inside())?;
            loop {
                let failed = self.failed;
                let keyed = map.next_key_seed(KeyInto {
                    sink: &mut *self.sink,
                    failed,
                })?;
                if keyed.is_none() {
                    break;
                }
                map.next_value_seed(self.inside())?;
            }
        }
        fed(self.failed, self.sink.map_end())
    }
}

/// A key, handed to the sink while it is borrowed.
struct KeyInto<'a, S: Sink + ?Sized> {
    sink: &'a mut S,
    failed: &'a Cell<Option<S::Error>>,
}

impl<'de, S: Sink + ?Sized> DeserializeSeed<'de> for KeyInto<'_, S> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_str(self)
    }
}

impl<'de, S: Sink + ?Sized> de::Visitor<'de> for KeyInto<'_, S> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a key")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<(), E> {
        fed(self.failed, self.sink.key(v))
    }
}
