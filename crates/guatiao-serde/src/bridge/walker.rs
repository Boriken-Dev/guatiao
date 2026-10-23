// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Any serde format read into a guatiao type: a `Deserializer` as a
//! [`Walker`], each serde visit the builder's event, a container's
//! elements pulled through serde's own `SeqAccess` and `MapAccess`.
//!
//! # Two refusals, two ways back
//!
//! A builder refuses with a [`MapError`]; serde unwinds only with its own
//! error. So a builder's refusal waits in a cell shared by every level
//! while a placeholder error unwinds serde, and is taken back out where
//! the walk resumes; the format's own error waits in a cell of the level
//! it belongs to, where the builder is told [`MapError::Stopped`], and is
//! handed back to serde there. [`from_deserializer`] answers whichever it
//! was.

#![forbid(unsafe_code)]

use alloc::borrow::Cow;
use alloc::string::String;
use core::cell::Cell;
use core::fmt;
use core::marker::PhantomData;

use serde::Deserializer;
use serde::de::{self, DeserializeSeed, Error as _, MapAccess, SeqAccess};

use guatiao::value::visit::{Build, FromWalk, ListWalker, MapWalker, Seed, Walker};
use guatiao::{MAX_DEPTH, MapError};

use super::BridgeError;
use super::number::Text;
use crate::de::{RAW_NUMBER, TOML_DATETIME};
use crate::{Presentation, from_data_uri};

/// Reads a `T` from any serde deserializer, with no tree built.
///
/// A number serde_json carries as its own text (`arbitrary_precision`)
/// arrives as that text and a TOML date or time as the text it was
/// written as; a `data:;base64,` string is bytes when `how` reads data
/// URIs. The format's refusal is [`BridgeError::Format`], the type's
/// [`BridgeError::Refused`], naming the key.
///
/// ```
/// use guatiao::FromWalk;
/// use guatiao_serde::{Presentation, from_deserializer};
///
/// let mut de = serde_json::Deserializer::from_str("[1, 2, 3]");
/// let read: Vec<u8> = from_deserializer(&mut de, Presentation::new()).unwrap();
/// assert_eq!(read, [1, 2, 3]);
/// ```
pub fn from_deserializer<'de, T: FromWalk, D: Deserializer<'de>>(
    de: D,
    how: Presentation,
) -> Result<T, BridgeError<D::Error, MapError>> {
    let refused = Cell::new(None);
    let slot = Cell::new(None);
    let read = T::from_walk(SerdeWalker {
        de,
        refused: &refused,
        slot: &slot,
        depth: 0,
        how,
        _de: PhantomData,
    });
    match read {
        Ok(value) => Ok(value),
        Err(e) => match slot.take() {
            Some(format) => Err(BridgeError::Format(format)),
            None => Err(BridgeError::Refused(e)),
        },
    }
}

/// A builder's refusal, kept while serde unwinds.
type Kept<'s> = &'s Cell<Option<MapError>>;

/// A deserializer as a walker, `depth` containers down.
struct SerdeWalker<'s, 'de, D: Deserializer<'de>> {
    de: D,
    refused: Kept<'s>,
    /// This level's format error, kept while the builder is told it
    /// stopped.
    slot: &'s Cell<Option<D::Error>>,
    depth: u32,
    how: Presentation,
    _de: PhantomData<&'de ()>,
}

impl<'de, D: Deserializer<'de>> Walker for SerdeWalker<'_, 'de, D> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        let (refused, slot) = (self.refused, self.slot);
        let visited = self.de.deserialize_any(Builder {
            build,
            refused,
            depth: self.depth,
            how: self.how,
        });
        match visited {
            Ok(out) => Ok(out),
            Err(e) => match refused.take() {
                Some(refusal) => Err(refusal),
                None => {
                    slot.set(Some(e));
                    Err(MapError::stopped())
                }
            },
        }
    }
}

/// A builder's answer as serde's: a refusal waits in `refused`.
fn answer<T, E: de::Error>(refused: Kept<'_>, built: Result<T, MapError>) -> Result<T, E> {
    built.map_err(|refusal| {
        refused.set(Some(refusal));
        E::custom("")
    })
}

/// A container's answer: the format's error if an element read failed
/// for the format, else the builder's.
fn answer_container<T, E: de::Error>(
    refused: Kept<'_>,
    failed: Option<E>,
    built: Result<T, MapError>,
) -> Result<T, E> {
    match (built, failed) {
        (Ok(out), _) => Ok(out),
        (Err(_), Some(format)) => Err(format),
        (Err(refusal), None) => {
            refused.set(Some(refusal));
            Err(E::custom(""))
        }
    }
}

/// What a document nested past [`MAX_DEPTH`] is refused with.
fn too_deep() -> MapError {
    MapError::bad_value(alloc::format!(
        "a document nested no deeper than {MAX_DEPTH} containers"
    ))
}

struct Builder<'s, B> {
    build: B,
    refused: Kept<'s>,
    depth: u32,
    how: Presentation,
}

impl<'de, B: Build> de::Visitor<'de> for Builder<'_, B> {
    type Value = B::Out;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.build.kinds())
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<B::Out, E> {
        answer(self.refused, self.build.bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<B::Out, E> {
        answer(self.refused, self.build.i64(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<B::Out, E> {
        answer(self.refused, self.build.u64(v))
    }

    fn visit_i128<E: de::Error>(self, v: i128) -> Result<B::Out, E> {
        answer(
            self.refused,
            self.build.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_u128<E: de::Error>(self, v: u128) -> Result<B::Out, E> {
        answer(
            self.refused,
            self.build.number(Text::of(format_args!("{v}")).as_str()),
        )
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<B::Out, E> {
        answer(self.refused, self.build.f64(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<B::Out, E> {
        // A data URI is bytes only when the caller asked for it.
        if self.how.reads_data_uris()
            && let Some(bytes) = from_data_uri(v)
        {
            return answer(self.refused, self.build.bytes(&bytes));
        }
        answer(self.refused, self.build.string(v))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<B::Out, E> {
        answer(self.refused, self.build.bytes(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<B::Out, E> {
        answer(self.refused, self.build.null())
    }

    fn visit_none<E: de::Error>(self) -> Result<B::Out, E> {
        answer(self.refused, self.build.null())
    }

    fn visit_some<D: Deserializer<'de>>(self, de: D) -> Result<B::Out, D::Error> {
        de.deserialize_any(self)
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, de: D) -> Result<B::Out, D::Error> {
        de.deserialize_any(self)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<B::Out, A::Error> {
        if self.depth >= MAX_DEPTH {
            return answer(self.refused, Err(too_deep()));
        }
        let failed = Cell::new(None);
        let built = self.build.list(Items {
            seq,
            refused: self.refused,
            failed: &failed,
            depth: self.depth + 1,
            how: self.how,
            _de: PhantomData,
        });
        answer_container(self.refused, failed.take(), built)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<B::Out, A::Error> {
        if self.depth >= MAX_DEPTH {
            return answer(self.refused, Err(too_deep()));
        }
        // A number or a date wearing a map's clothes: see `de.rs`.
        let first = match map.next_key_seed(FirstKey)? {
            None => None,
            Some(Key::Number) => {
                let build = self.build;
                let built = map.next_value_seed(Reserved(|text: &str| build.number(text)))?;
                return answer(self.refused, built);
            }
            Some(Key::Datetime) => {
                let build = self.build;
                let built = map.next_value_seed(Reserved(|text: &str| build.string(text)))?;
                return answer(self.refused, built);
            }
            Some(Key::Plain(key)) => Some(key),
        };
        let failed = Cell::new(None);
        let built = self.build.map(Entries {
            map,
            first,
            done: false,
            refused: self.refused,
            failed: &failed,
            depth: self.depth + 1,
            how: self.how,
            _de: PhantomData,
        });
        answer_container(self.refused, failed.take(), built)
    }
}

/// A seed read through a deserializer one level down.
struct SeedOf<'s, S> {
    seed: S,
    refused: Kept<'s>,
    depth: u32,
    how: Presentation,
}

impl<'de, S: Seed> DeserializeSeed<'de> for SeedOf<'_, S> {
    type Value = S::Out;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<S::Out, D::Error> {
        let slot = Cell::new(None);
        let read = self.seed.read(SerdeWalker {
            de,
            refused: self.refused,
            slot: &slot,
            depth: self.depth,
            how: self.how,
            _de: PhantomData,
        });
        match read {
            Ok(out) => Ok(out),
            Err(refusal) => match slot.take() {
                Some(format) => Err(format),
                None => {
                    self.refused.set(Some(refusal));
                    Err(D::Error::custom(""))
                }
            },
        }
    }
}

/// What an element read answered: the builder's refusal if one waits,
/// else the format's, kept for the container to hand back.
fn element<T, E>(
    refused: Kept<'_>,
    failed: &Cell<Option<E>>,
    read: Result<T, E>,
) -> Result<T, MapError> {
    read.map_err(|e| match refused.take() {
        Some(refusal) => refusal,
        None => {
            failed.set(Some(e));
            MapError::stopped()
        }
    })
}

struct Items<'s, 'de, A: SeqAccess<'de>> {
    seq: A,
    refused: Kept<'s>,
    failed: &'s Cell<Option<A::Error>>,
    depth: u32,
    how: Presentation,
    _de: PhantomData<&'de ()>,
}

impl<'de, A: SeqAccess<'de>> ListWalker for Items<'_, 'de, A> {
    fn len_hint(&self) -> Option<usize> {
        self.seq.size_hint()
    }

    fn next_seed<S: Seed>(&mut self, seed: S) -> Result<Option<S::Out>, MapError> {
        let read = self.seq.next_element_seed(SeedOf {
            seed,
            refused: self.refused,
            depth: self.depth,
            how: self.how,
        });
        element(self.refused, self.failed, read)
    }
}

struct Entries<'s, 'de, A: MapAccess<'de>> {
    map: A,
    /// The first key, read to see whether it was a reserved one.
    first: Option<Cow<'de, str>>,
    done: bool,
    refused: Kept<'s>,
    failed: &'s Cell<Option<A::Error>>,
    depth: u32,
    how: Presentation,
    _de: PhantomData<&'de ()>,
}

impl<'de, A: MapAccess<'de>> MapWalker for Entries<'_, 'de, A> {
    fn len_hint(&self) -> Option<usize> {
        self.map
            .size_hint()
            .map(|n| n + usize::from(self.first.is_some()))
    }

    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        if let Some(first) = self.first.take() {
            return Ok(Some(read(&first)));
        }
        if self.done {
            return Ok(None);
        }
        let key = self.map.next_key_seed(KeyWith(read, PhantomData));
        let key = element(self.refused, self.failed, key)?;
        self.done = key.is_none();
        Ok(key)
    }

    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError> {
        let read = self.map.next_value_seed(SeedOf {
            seed,
            refused: self.refused,
            depth: self.depth,
            how: self.how,
        });
        element(self.refused, self.failed, read)
    }

    fn skip_value(&mut self) -> Result<(), MapError> {
        let read = self.map.next_value::<de::IgnoredAny>();
        element(self.refused, self.failed, read).map(|_| ())
    }
}

/// A map's first key: a reserved one, or text.
pub(crate) enum Key<'de> {
    Number,
    Datetime,
    Plain(Cow<'de, str>),
}

/// Reads a map's first key.
pub(crate) struct FirstKey;

impl<'de> DeserializeSeed<'de> for FirstKey {
    type Value = Key<'de>;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Key<'de>, D::Error> {
        de.deserialize_str(self)
    }
}

impl FirstKey {
    fn key(text: Cow<'_, str>) -> Key<'_> {
        match &*text {
            RAW_NUMBER => Key::Number,
            TOML_DATETIME => Key::Datetime,
            _ => Key::Plain(text),
        }
    }
}

impl<'de> de::Visitor<'de> for FirstKey {
    type Value = Key<'de>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a key")
    }

    fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Key<'de>, E> {
        Ok(FirstKey::key(Cow::Borrowed(v)))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Key<'de>, E> {
        Ok(match FirstKey::key(Cow::Borrowed(v)) {
            Key::Plain(_) => Key::Plain(Cow::Owned(String::from(v))),
            Key::Number => Key::Number,
            Key::Datetime => Key::Datetime,
        })
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Key<'de>, E> {
        Ok(match FirstKey::key(Cow::Borrowed(&v)) {
            Key::Plain(_) => Key::Plain(Cow::Owned(v)),
            Key::Number => Key::Number,
            Key::Datetime => Key::Datetime,
        })
    }
}

/// A key, handed to `read` while it is borrowed.
struct KeyWith<F, R>(F, PhantomData<fn() -> R>);

impl<'de, R, F: FnOnce(&str) -> R> DeserializeSeed<'de> for KeyWith<F, R> {
    type Value = R;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<R, D::Error> {
        de.deserialize_str(self)
    }
}

impl<'de, R, F: FnOnce(&str) -> R> de::Visitor<'de> for KeyWith<F, R> {
    type Value = R;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a key")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<R, E> {
        Ok((self.0)(v))
    }
}

/// The text under a reserved key, handed to `read` while it is borrowed,
/// and refused without saying what was there instead: the value could be
/// a secret.
pub(crate) struct Reserved<F>(pub(crate) F);

impl<'de, R, F: FnOnce(&str) -> R> DeserializeSeed<'de> for Reserved<F> {
    type Value = R;

    // `any`, so a value of another kind reaches a method below rather
    // than the format's own refusal, which quotes it.
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<R, D::Error> {
        de.deserialize_any(self)
    }
}

/// What anything but text under a reserved key is refused with.
const NOT_TEXT: &str = "a reserved key holds something other than text";

impl<'de, R, F: FnOnce(&str) -> R> de::Visitor<'de> for Reserved<F> {
    type Value = R;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("text")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<R, E> {
        Ok((self.0)(v))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_bytes<E: de::Error>(self, _: &[u8]) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_unit<E: de::Error>(self) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_none<E: de::Error>(self) -> Result<R, E> {
        Err(E::custom(NOT_TEXT))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<R, A::Error> {
        Err(A::Error::custom(NOT_TEXT))
    }

    fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<R, A::Error> {
        Err(A::Error::custom(NOT_TEXT))
    }
}
