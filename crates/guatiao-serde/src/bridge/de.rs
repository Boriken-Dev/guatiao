// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Any serde type read from a guatiao walker: a [`Walker`] as a serde
//! `Deserializer`, whose events are the serde visits and whose
//! containers are serde's `SeqAccess` and `MapAccess`.
//!
//! A number reaches serde as an integer of up to 128 bits or as the
//! nearest `f64`, so `1.10` reads as `1.1`, and an integer wider than 128
//! bits is refused. A refusal names the key it failed at, as the derived
//! readers do (`a.b` for a declared field, `a[b]` for a key of an open
//! map, `a[2]` for a position), and never quotes the value.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use core::fmt;
use core::marker::PhantomData;

use serde::de::value::StrDeserializer;
use serde::de::{
    self, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};

use guatiao::value::visit::{Build, ListWalker, MapWalker, NoWalker, Replay, Seed, Walker};
use guatiao::{Kinds, MapError};

/// A refusal, as serde's error, keeping the path.
#[derive(Debug)]
pub(crate) struct ReadError(pub(crate) MapError);

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for ReadError {}

fn refuse(expected: impl Into<String>) -> ReadError {
    ReadError(MapError::bad_value(expected))
}

// serde's defaults quote the value refused; these say only what was
// expected, because a configuration value can be a secret.
impl de::Error for ReadError {
    fn custom<M: fmt::Display>(msg: M) -> ReadError {
        refuse(msg.to_string())
    }

    fn invalid_type(_: de::Unexpected<'_>, expected: &dyn de::Expected) -> ReadError {
        refuse(expected.to_string())
    }

    fn invalid_value(_: de::Unexpected<'_>, expected: &dyn de::Expected) -> ReadError {
        refuse(expected.to_string())
    }

    fn invalid_length(_: usize, expected: &dyn de::Expected) -> ReadError {
        refuse(expected.to_string())
    }

    fn unknown_variant(_: &str, expected: &'static [&'static str]) -> ReadError {
        refuse(format!("one of {}", expected.join(", ")))
    }

    fn unknown_field(field: &str, _: &'static [&'static str]) -> ReadError {
        ReadError(MapError::bad_value("a key the type declares").under(field))
    }

    fn missing_field(field: &'static str) -> ReadError {
        ReadError(MapError::missing(field))
    }

    fn duplicate_field(field: &'static str) -> ReadError {
        ReadError(MapError::bad_value("a key given once").under(field))
    }
}

/// A walker as a serde deserializer.
pub(crate) struct Walking<W>(pub(crate) W);

/// A float that reads back the same, or the refusal.
fn float(text: &str) -> Result<f64, MapError> {
    text.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| MapError::bad_value("a number an f64 can hold"))
}

/// A serde visit's answer as a builder's.
fn visited<T>(r: Result<T, ReadError>) -> Result<T, MapError> {
    r.map_err(|e| e.0)
}

/// The events a serde visitor takes, read as `deserialize_any` reads
/// them: what every other hint falls back to.
struct Any<'de, V>(V, PhantomData<&'de ()>);

impl<'de, V: Visitor<'de>> Build for Any<'de, V> {
    type Out = V::Value;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn absent(self) -> Result<V::Value, MapError> {
        Err(MapError::bad_value(
            "a value serde can read: null, a boolean, a number, text, bytes, a list or a map",
        ))
    }

    fn null(self) -> Result<V::Value, MapError> {
        visited(self.0.visit_unit())
    }

    fn bool(self, v: bool) -> Result<V::Value, MapError> {
        visited(self.0.visit_bool(v))
    }

    fn number(self, text: &str) -> Result<V::Value, MapError> {
        let v = self.0;
        visited(if let Ok(n) = text.parse::<i64>() {
            v.visit_i64(n)
        } else if let Ok(n) = text.parse::<u64>() {
            v.visit_u64(n)
        } else if let Ok(n) = text.parse::<i128>() {
            v.visit_i128(n)
        } else if let Ok(n) = text.parse::<u128>() {
            v.visit_u128(n)
        } else if text.contains(['.', 'e', 'E']) {
            v.visit_f64(float(text)?)
        } else {
            return Err(MapError::bad_value("an integer of at most 128 bits"));
        })
    }

    fn i64(self, v: i64) -> Result<V::Value, MapError> {
        visited(self.0.visit_i64(v))
    }

    fn u64(self, v: u64) -> Result<V::Value, MapError> {
        visited(self.0.visit_u64(v))
    }

    fn f64(self, v: f64) -> Result<V::Value, MapError> {
        visited(self.0.visit_f64(v))
    }

    fn string(self, text: &str) -> Result<V::Value, MapError> {
        visited(self.0.visit_str(text))
    }

    fn bytes(self, bytes: &[u8]) -> Result<V::Value, MapError> {
        visited(self.0.visit_bytes(bytes))
    }

    fn list<L: ListWalker>(self, items: L) -> Result<V::Value, MapError> {
        visited(self.0.visit_seq(Elements { items, at: 0 }))
    }

    fn map<M: MapWalker>(self, entries: M) -> Result<V::Value, MapError> {
        visited(self.0.visit_map(Entries {
            entries,
            key: String::new(),
            named: false,
        }))
    }
}

/// `deserialize_option`: null or absent is `None`, anything else `Some`
/// of it.
struct Optional<'de, V>(V, PhantomData<&'de ()>);

/// The scalar events of a builder that hands each one, replayed, to
/// serde's `visit_some`.
macro_rules! some {
    ($($method:ident($($arg:ident: $ty:ty),*) => $replay:expr;)*) => {$(
        fn $method(self, $($arg: $ty),*) -> Result<V::Value, MapError> {
            visited(self.0.visit_some(Walking($replay)))
        }
    )*};
}

impl<'de, V: Visitor<'de>> Build for Optional<'de, V> {
    type Out = V::Value;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn absent(self) -> Result<V::Value, MapError> {
        visited(self.0.visit_none())
    }

    fn null(self) -> Result<V::Value, MapError> {
        visited(self.0.visit_none())
    }

    some! {
        bool(v: bool) => Replay::<NoWalker, NoWalker>::Bool(v);
        number(v: &str) => Replay::<NoWalker, NoWalker>::Number(v);
        i64(v: i64) => Replay::<NoWalker, NoWalker>::I64(v);
        u64(v: u64) => Replay::<NoWalker, NoWalker>::U64(v);
        f64(v: f64) => Replay::<NoWalker, NoWalker>::F64(v);
        string(v: &str) => Replay::<NoWalker, NoWalker>::String(v);
        bytes(v: &[u8]) => Replay::<NoWalker, NoWalker>::Bytes(v);
    }

    fn list<L: ListWalker>(self, items: L) -> Result<V::Value, MapError> {
        visited(
            self.0
                .visit_some(Walking(Replay::<L, NoWalker>::List(items))),
        )
    }

    fn map<M: MapWalker>(self, entries: M) -> Result<V::Value, MapError> {
        visited(
            self.0
                .visit_some(Walking(Replay::<NoWalker, M>::Map(entries))),
        )
    }
}

/// `deserialize_struct`: a map's keys are declared fields, named `a.b`
/// in an error; anything else as `deserialize_any` reads it.
struct Record<'de, V>(V, PhantomData<&'de ()>);

/// `deserialize_f64`: a number of any spelling is a float; anything
/// else as `deserialize_any` reads it.
struct Float<'de, V>(V, PhantomData<&'de ()>);

/// The events a hint does not change, handed to [`Any`].
macro_rules! as_any {
    ($($method:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $method(self, $($arg: $ty),*) -> Result<V::Value, MapError> {
            Any(self.0, PhantomData).$method($($arg),*)
        }
    )*};
}

impl<'de, V: Visitor<'de>> Build for Record<'de, V> {
    type Out = V::Value;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    as_any! {
        absent(); null(); bool(v: bool); number(v: &str); i64(v: i64); u64(v: u64);
        f64(v: f64); string(v: &str); bytes(v: &[u8]);
    }

    fn list<L: ListWalker>(self, items: L) -> Result<V::Value, MapError> {
        Any(self.0, PhantomData).list(items)
    }

    fn map<M: MapWalker>(self, entries: M) -> Result<V::Value, MapError> {
        visited(self.0.visit_map(Entries {
            entries,
            key: String::new(),
            named: true,
        }))
    }
}

impl<'de, V: Visitor<'de>> Build for Float<'de, V> {
    type Out = V::Value;

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    as_any! {
        absent(); null(); bool(v: bool); f64(v: f64); string(v: &str); bytes(v: &[u8]);
    }

    // An integer too wide for 128 bits is still a float.
    fn number(self, text: &str) -> Result<V::Value, MapError> {
        visited(self.0.visit_f64(float(text)?))
    }

    fn i64(self, v: i64) -> Result<V::Value, MapError> {
        self.number(&v.to_string())
    }

    fn u64(self, v: u64) -> Result<V::Value, MapError> {
        self.number(&v.to_string())
    }

    fn list<L: ListWalker>(self, items: L) -> Result<V::Value, MapError> {
        Any(self.0, PhantomData).list(items)
    }

    fn map<M: MapWalker>(self, entries: M) -> Result<V::Value, MapError> {
        Any(self.0, PhantomData).map(entries)
    }
}

/// `deserialize_enum`: a variant's name, or a map of one key, its name,
/// holding its content.
struct Choice<'de, V>(V, PhantomData<&'de ()>);

const NOT_A_VARIANT: &str =
    "a variant: its name, or a map of one key, its name, holding its content";

impl<'de, V: Visitor<'de>> Build for Choice<'de, V> {
    type Out = V::Value;

    fn kinds(&self) -> Kinds {
        Kinds::STRING.union(Kinds::MAP)
    }

    fn string(self, name: &str) -> Result<V::Value, MapError> {
        visited(self.0.visit_enum(Named {
            name: name.to_string(),
            content: None::<NoWalker>,
        }))
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<V::Value, MapError> {
        if entries.len_hint().is_some_and(|n| n != 1) {
            return Err(MapError::bad_value(NOT_A_VARIANT));
        }
        let Some(name) = entries.next_key(ToString::to_string)? else {
            return Err(MapError::bad_value(NOT_A_VARIANT));
        };
        let value = visited(self.0.visit_enum(Named {
            name,
            content: Some(&mut entries),
        }))?;
        if entries.next_key(|_| ())?.is_some() {
            return Err(MapError::bad_value(NOT_A_VARIANT));
        }
        Ok(value)
    }
}

/// An enum's variant: a bare name, or a name and the map entry holding
/// its content.
struct Named<M> {
    name: String,
    content: Option<M>,
}

impl<'de, M: MapWalker> EnumAccess<'de> for Named<M> {
    type Error = ReadError;
    type Variant = Named<M>;

    fn variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<(T::Value, Named<M>), ReadError> {
        let name: StrDeserializer<'_, ReadError> = self.name.as_str().into_deserializer();
        let variant = seed.deserialize(name)?;
        Ok((variant, self))
    }
}

impl<M: MapWalker> Named<M> {
    fn under(&self, e: MapError) -> ReadError {
        ReadError(e.under(&self.name))
    }

    /// The content, read by a serde seed through a walker.
    fn content<'de, T: DeserializeSeed<'de>>(mut self, seed: T) -> Result<T::Value, ReadError> {
        let Some(mut entries) = self.content.take() else {
            return Err(refuse("a variant with content"));
        };
        entries
            .value_seed(SerdeSeed(seed, PhantomData))
            .map_err(|e| self.under(e))
    }
}

impl<'de, M: MapWalker> VariantAccess<'de> for Named<M> {
    type Error = ReadError;

    fn unit_variant(self) -> Result<(), ReadError> {
        if self.content.is_none() {
            return Ok(());
        }
        self.content(PhantomData::<()>)
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, ReadError> {
        self.content(seed)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, v: V) -> Result<V::Value, ReadError> {
        self.content(Hinted(v, Hint::Seq))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        self.content(Hinted(v, Hint::Struct))
    }
}

/// How a variant's content is to be read.
enum Hint {
    Seq,
    Struct,
}

/// A serde visitor with the hint its content is read under.
struct Hinted<V>(V, Hint);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Hinted<V> {
    type Value = V::Value;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<V::Value, D::Error> {
        match self.1 {
            Hint::Seq => de.deserialize_seq(self.0),
            Hint::Struct => de.deserialize_struct("", &[], self.0),
        }
    }
}

/// A serde seed as a guatiao one.
struct SerdeSeed<'de, T>(T, PhantomData<&'de ()>);

impl<'de, T: DeserializeSeed<'de>> Seed for SerdeSeed<'de, T> {
    type Out = T::Value;

    fn read<W: Walker>(self, walker: W) -> Result<T::Value, MapError> {
        self.0.deserialize(Walking(walker)).map_err(|e| e.0)
    }
}

struct Elements<L> {
    items: L,
    at: usize,
}

impl<'de, L: ListWalker> SeqAccess<'de> for Elements<L> {
    type Error = ReadError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, ReadError> {
        let at = self.at;
        self.at += 1;
        self.items
            .next_seed(SerdeSeed(seed, PhantomData))
            .map_err(|e| ReadError(e.at(at)))
    }

    fn size_hint(&self) -> Option<usize> {
        self.items.len_hint()
    }
}

struct Entries<M> {
    entries: M,
    /// The key just read, to name it in an error.
    key: String,
    /// Whether the keys are a struct's declared fields (`a.b`) or data
    /// (`a[b]`).
    named: bool,
}

impl<'de, M: MapWalker> MapAccess<'de> for Entries<M> {
    type Error = ReadError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ReadError> {
        let key = &mut self.key;
        let read = self.entries.next_key(|text| {
            key.clear();
            key.push_str(text);
            let text: StrDeserializer<'_, ReadError> = text.into_deserializer();
            seed.deserialize(text)
        });
        match read {
            Ok(Some(key)) => key.map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(ReadError(e)),
        }
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, ReadError> {
        let key = &self.key;
        let named = self.named;
        self.entries
            .value_seed(SerdeSeed(seed, PhantomData))
            .map_err(|e| ReadError(if named { e.under(key) } else { e.at_key(key) }))
    }

    fn size_hint(&self) -> Option<usize> {
        self.entries.len_hint()
    }
}

/// Reads any value and keeps nothing: `deserialize_ignored_any`, which
/// must still pass over what the walker holds.
struct Ignore;

impl Build for Ignore {
    type Out = ();

    fn kinds(&self) -> Kinds {
        Kinds::ANY
    }

    fn absent(self) -> Result<(), MapError> {
        Ok(())
    }

    fn null(self) -> Result<(), MapError> {
        Ok(())
    }

    fn bool(self, _: bool) -> Result<(), MapError> {
        Ok(())
    }

    fn number(self, _: &str) -> Result<(), MapError> {
        Ok(())
    }

    fn i64(self, _: i64) -> Result<(), MapError> {
        Ok(())
    }

    fn u64(self, _: u64) -> Result<(), MapError> {
        Ok(())
    }

    fn f64(self, _: f64) -> Result<(), MapError> {
        Ok(())
    }

    fn string(self, _: &str) -> Result<(), MapError> {
        Ok(())
    }

    fn bytes(self, _: &[u8]) -> Result<(), MapError> {
        Ok(())
    }

    fn list<L: ListWalker>(self, mut items: L) -> Result<(), MapError> {
        while items.next_seed(IgnoreSeed)?.is_some() {}
        Ok(())
    }

    fn map<M: MapWalker>(self, mut entries: M) -> Result<(), MapError> {
        while entries.next_key(|_| ())?.is_some() {
            entries.skip_value()?;
        }
        Ok(())
    }
}

struct IgnoreSeed;

impl Seed for IgnoreSeed {
    type Out = ();

    fn read<W: Walker>(self, walker: W) -> Result<(), MapError> {
        walker.build(Ignore)
    }
}

impl<'de, W: Walker> de::Deserializer<'de> for Walking<W> {
    type Error = ReadError;

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.0.build(Any(v, PhantomData)).map_err(ReadError)
    }

    fn deserialize_f32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.deserialize_f64(v)
    }

    fn deserialize_f64<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.0.build(Float(v, PhantomData)).map_err(ReadError)
    }

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.0.build(Optional(v, PhantomData)).map_err(ReadError)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        v: V,
    ) -> Result<V::Value, ReadError> {
        v.visit_newtype_struct(self)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        self.0.build(Record(v, PhantomData)).map_err(ReadError)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        self.0.build(Choice(v, PhantomData)).map_err(ReadError)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.0.build(Ignore).map_err(ReadError)?;
        v.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 char str string bytes
        byte_buf unit unit_struct seq tuple tuple_struct map identifier
    }
}
