// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Any serde type as a field of a derived type.
//!
//! guatiao's traits for another crate's type can be implemented only in
//! guatiao, so a serde type crosses here instead, two ways:
//!
//! - **[`Serde<T, S>`]**, a wrapper this crate owns, implements
//!   `ToValue`, `FromValue` and `Schema` for any
//!   `T: Serialize + DeserializeOwned`. Its schema is `S`'s: any value
//!   (`{}`) by default, an open object with `S = guatiao::Map`, or any
//!   type of yours that implements `Schema`.
//! - **`with` modules** for a bare field: `#[map(with =
//!   guatiao_serde::adapt::any)]` describes the field as any value,
//!   [`object`] as an open object. A module of your own reuses
//!   [`to_value_in`] and [`from_value`] and adds its own `kind`.
//!
//! ```
//! use guatiao::{FromValue, Map, Schema, ToValue};
//! use guatiao_serde::Serde;
//!
//! #[derive(ToValue, FromValue, Schema)]
//! struct Hook {
//!     url: String,
//!     #[map(with = guatiao_serde::adapt::object)]
//!     headers: serde_json::Map<String, serde_json::Value>,
//!     body: Serde<serde_json::Value>,
//!     extra: Serde<serde_json::Value, Map>,
//! }
//! ```
//!
//! **What crosses.** A value is serde's data model, externally tagged: a
//! unit variant is its name, any other variant a map of one key. A float
//! keeps a fraction (`1.0` stays `1.0`), a number serialised as its own
//! text (`serde_json`'s under `arbitrary_precision`) keeps that text, and
//! a TOML date or time is the text it was written as. Reading hands a
//! number to serde as an integer of up to 128 bits or as the nearest
//! `f64`, so `1.10` reads as `1.1`, and an integer wider than 128 bits is
//! refused. Text is human-readable, so a type with two forms (a UUID, a
//! timestamp) takes its text form.
//!
//! **Refusals.** Reading names the key it failed at, as the derived
//! readers do: a declared field as `a.b`, a key of an open map as
//! `a[b]`, a position as `a[2]`, and it never quotes the value. Writing
//! answers a [`ValueError`]: a type's own serialisation error, which has
//! no message to carry there, is `ValueError::Unrepresentable`.

#![forbid(unsafe_code)]

use alloc::string::{String, ToString};
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use serde::de::value::BorrowedStrDeserializer;
use serde::de::{
    self, Deserialize, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::ser::{self, Serialize};

use guatiao::schema::KindBuilder;
use guatiao::value::convert::TryAsRef;
use guatiao::value::error::MAX_DEPTH;
use guatiao::value::types::{Buffer, Entry, List, Map, Number, Tag, Text};
use guatiao::{Alloc, AllocError, FromValue, MapError, Schema, ToValue, Value, ValueError};

use crate::de::{RAW_NUMBER, TOML_DATETIME};

/// A serde type as a value: `T` converts through serde, and its schema
/// is `S`'s.
///
/// `S` defaults to [`Value`], any value (`{}`); `guatiao::Map` makes it
/// an open object. Built with [`Serde::new`] or `.into()`; the value is
/// `.0`, or reached through `Deref`.
pub struct Serde<T, S = Value>(pub T, PhantomData<fn() -> S>);

impl<T, S> Serde<T, S> {
    /// Wraps `value`.
    pub const fn new(value: T) -> Serde<T, S> {
        Serde(value, PhantomData)
    }

    /// The value it wraps.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T, S> From<T> for Serde<T, S> {
    fn from(value: T) -> Serde<T, S> {
        Serde::new(value)
    }
}

impl<T, S> Deref for Serde<T, S> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T, S> DerefMut for Serde<T, S> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

// By hand, so `S` (a schema marker) needs none of these traits.
impl<T: fmt::Debug, S> fmt::Debug for Serde<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Serde").field(&self.0).finish()
    }
}

impl<T: Clone, S> Clone for Serde<T, S> {
    fn clone(&self) -> Serde<T, S> {
        Serde::new(self.0.clone())
    }
}

impl<T: PartialEq, S> PartialEq for Serde<T, S> {
    fn eq(&self, other: &Serde<T, S>) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq, S> Eq for Serde<T, S> {}

impl<T: Default, S> Default for Serde<T, S> {
    fn default() -> Serde<T, S> {
        Serde::new(T::default())
    }
}

impl<T: Serialize, S> ToValue for Serde<T, S> {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        to_value_in(&self.0, alloc)
    }
}

impl<T: DeserializeOwned, S> FromValue for Serde<T, S> {
    fn from_value(value: &Value) -> Result<Serde<T, S>, MapError> {
        from_value(value).map(Serde::new)
    }
}

impl<T, S: Schema> Schema for Serde<T, S> {
    fn kind(alloc: Alloc) -> KindBuilder {
        S::kind(alloc)
    }
}

/// `#[map(with = guatiao_serde::adapt::any)]`: a serde field described
/// as any value, `{}`.
pub mod any {
    pub use super::{from_value, to_value_in};

    /// Any value.
    pub fn kind(alloc: guatiao::Alloc) -> guatiao::schema::KindBuilder {
        guatiao::schema::KindBuilder::any_in(alloc)
    }
}

/// `#[map(with = guatiao_serde::adapt::object)]`: a serde field
/// described as an open object, whose keys are data and whose values are
/// anything.
pub mod object {
    pub use super::{from_value, to_value_in};

    /// An object of any keys and values.
    pub fn kind(alloc: guatiao::Alloc) -> guatiao::schema::KindBuilder {
        use guatiao::schema::KindBuilder;
        KindBuilder::map_of_in(alloc, KindBuilder::any_in(alloc))
    }
}

/// Writes any serde type as a value, through `alloc`.
pub fn to_value_in<T: Serialize + ?Sized>(value: &T, alloc: Alloc) -> Result<Value, ValueError> {
    value.serialize(Writer { alloc, depth: 0 }).map_err(|e| e.0)
}

/// Reads any serde type out of a value. The error names the key it
/// failed at, below this value.
pub fn from_value<T: DeserializeOwned>(value: &Value) -> Result<T, MapError> {
    T::deserialize(Reader { value, depth: 0 }).map_err(|e| e.0)
}

// --- writing ----------------------------------------------------------

#[derive(Debug)]
struct WriteError(ValueError);

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for WriteError {}

impl ser::Error for WriteError {
    fn custom<M: fmt::Display>(_: M) -> WriteError {
        WriteError(ValueError::Unrepresentable)
    }
}

impl From<ValueError> for WriteError {
    fn from(e: ValueError) -> WriteError {
        WriteError(e)
    }
}

impl From<AllocError> for WriteError {
    fn from(e: AllocError) -> WriteError {
        WriteError(e.into())
    }
}

/// A serde serializer whose output is a value.
#[derive(Clone, Copy)]
struct Writer {
    alloc: Alloc,
    /// Containers open above the value being written.
    depth: u32,
}

impl Writer {
    /// The writer for what goes inside a container opened here.
    fn inside(self) -> Result<Writer, WriteError> {
        if self.depth >= MAX_DEPTH {
            return Err(WriteError(ValueError::TooDeep));
        }
        Ok(Writer {
            depth: self.depth + 1,
            ..self
        })
    }

    fn number(self, text: &str) -> Result<Value, WriteError> {
        Ok(Number::new_in(self.alloc, text)?.into())
    }

    fn text(self, text: &str) -> Result<Value, WriteError> {
        Ok(Text::new_in(self.alloc, text)?.into())
    }

    /// `{variant: content}`, serde's external tagging.
    fn tagged(self, variant: &str, content: Value) -> Result<Value, WriteError> {
        let mut map = Map::new_in(self.alloc);
        map.try_set(variant, content)?;
        Ok(map.into())
    }

    fn list(self, variant: Option<&'static str>) -> Result<Items, WriteError> {
        let outer = self;
        let inner = if variant.is_some() {
            self.inside()?
        } else {
            self
        };
        Ok(Items {
            outer,
            writer: inner.inside()?,
            list: List::new_in(self.alloc),
            variant,
        })
    }

    fn map(self, variant: Option<&'static str>) -> Result<Fields, WriteError> {
        let outer = self;
        let inner = if variant.is_some() {
            self.inside()?
        } else {
            self
        };
        Ok(Fields {
            outer,
            writer: inner.inside()?,
            map: Map::new_in(self.alloc),
            key: None,
            variant,
        })
    }
}

impl ser::Serializer for Writer {
    type Ok = Value;
    type Error = WriteError;
    type SerializeSeq = Items;
    type SerializeTuple = Items;
    type SerializeTupleStruct = Items;
    type SerializeTupleVariant = Items;
    type SerializeMap = Fields;
    type SerializeStruct = Fields;
    type SerializeStructVariant = Fields;

    fn serialize_bool(self, v: bool) -> Result<Value, WriteError> {
        Ok(Value::from(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value, WriteError> {
        self.serialize_i64(v.into())
    }

    fn serialize_i16(self, v: i16) -> Result<Value, WriteError> {
        self.serialize_i64(v.into())
    }

    fn serialize_i32(self, v: i32) -> Result<Value, WriteError> {
        self.serialize_i64(v.into())
    }

    fn serialize_i64(self, v: i64) -> Result<Value, WriteError> {
        self.number(&v.to_string())
    }

    fn serialize_i128(self, v: i128) -> Result<Value, WriteError> {
        self.number(&v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Result<Value, WriteError> {
        self.serialize_u64(v.into())
    }

    fn serialize_u16(self, v: u16) -> Result<Value, WriteError> {
        self.serialize_u64(v.into())
    }

    fn serialize_u32(self, v: u32) -> Result<Value, WriteError> {
        self.serialize_u64(v.into())
    }

    fn serialize_u64(self, v: u64) -> Result<Value, WriteError> {
        self.number(&v.to_string())
    }

    fn serialize_u128(self, v: u128) -> Result<Value, WriteError> {
        self.number(&v.to_string())
    }

    // `{:?}` is the shortest text that reads back the same float, and it
    // keeps `1.0` a float where `to_string` would write the integer `1`.
    // A non-finite float is not a number in the value model.
    fn serialize_f32(self, v: f32) -> Result<Value, WriteError> {
        self.number(&format!("{v:?}"))
    }

    fn serialize_f64(self, v: f64) -> Result<Value, WriteError> {
        self.number(&format!("{v:?}"))
    }

    fn serialize_char(self, v: char) -> Result<Value, WriteError> {
        self.text(v.encode_utf8(&mut [0; 4]))
    }

    fn serialize_str(self, v: &str) -> Result<Value, WriteError> {
        self.text(v)
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value, WriteError> {
        Ok(Buffer::new_in(self.alloc, v)?.into())
    }

    fn serialize_none(self) -> Result<Value, WriteError> {
        Ok(Value::null())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<Value, WriteError> {
        v.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, WriteError> {
        Ok(Value::null())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Value, WriteError> {
        Ok(Value::null())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<Value, WriteError> {
        self.text(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        v: &T,
    ) -> Result<Value, WriteError> {
        v.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        v: &T,
    ) -> Result<Value, WriteError> {
        let content = v.serialize(self.inside()?)?;
        self.tagged(variant, content)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Items, WriteError> {
        self.list(None)
    }

    fn serialize_tuple(self, _: usize) -> Result<Items, WriteError> {
        self.list(None)
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Items, WriteError> {
        self.list(None)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Items, WriteError> {
        self.list(Some(variant))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Fields, WriteError> {
        self.map(None)
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Fields, WriteError> {
        self.map(None)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Fields, WriteError> {
        self.map(Some(variant))
    }
}

/// A list being written; a tuple variant's is wrapped in its name.
struct Items {
    outer: Writer,
    writer: Writer,
    list: List,
    variant: Option<&'static str>,
}

impl Items {
    fn push<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        let item = v.serialize(self.writer)?;
        Ok(self.list.try_push(item)?)
    }

    fn finish(self) -> Result<Value, WriteError> {
        match self.variant {
            Some(variant) => self.outer.tagged(variant, self.list.into()),
            None => Ok(self.list.into()),
        }
    }
}

impl ser::SerializeSeq for Items {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        self.push(v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

impl ser::SerializeTuple for Items {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        self.push(v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Items {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        self.push(v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Items {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        self.push(v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

/// A map being written; a struct variant's is wrapped in its name.
struct Fields {
    outer: Writer,
    writer: Writer,
    map: Map,
    key: Option<String>,
    variant: Option<&'static str>,
}

impl Fields {
    fn set<T: Serialize + ?Sized>(&mut self, key: &str, v: &T) -> Result<(), WriteError> {
        let value = v.serialize(self.writer)?;
        Ok(self.map.try_set(key, value)?)
    }

    fn finish(self) -> Result<Value, WriteError> {
        if let Some(variant) = self.variant {
            return self.outer.tagged(variant, self.map.into());
        }
        // A number or a TOML date carried as its text under a reserved
        // key, the disguise `ValueSeed` also takes off.
        if let [only] = self.map.entries()
            && let Some(text) = TryAsRef::<str>::try_as_ref(only.value())
        {
            match only.key() {
                RAW_NUMBER => return self.outer.number(text),
                TOML_DATETIME => return self.outer.text(text),
                _ => {}
            }
        }
        Ok(self.map.into())
    }
}

impl ser::SerializeMap for Fields {
    type Ok = Value;
    type Error = WriteError;

    // A key is text; a number or a character key crosses as its text, as
    // `serde_json` writes one.
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), WriteError> {
        let key = key.serialize(self.writer)?;
        let text = TryAsRef::<str>::try_as_ref(&key)
            .or_else(|| TryAsRef::<Number>::try_as_ref(&key).map(AsRef::<str>::as_ref))
            .ok_or(WriteError(ValueError::WrongKind))?;
        self.key = Some(text.to_string());
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<(), WriteError> {
        let key = self.key.take().ok_or(WriteError(ValueError::WrongKind))?;
        self.set(&key, v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

impl ser::SerializeStruct for Fields {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), WriteError> {
        self.set(key, v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Fields {
    type Ok = Value;
    type Error = WriteError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        v: &T,
    ) -> Result<(), WriteError> {
        self.set(key, v)
    }

    fn end(self) -> Result<Value, WriteError> {
        self.finish()
    }
}

// --- reading ----------------------------------------------------------

#[derive(Debug)]
struct ReadError(MapError);

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

/// A serde deserializer over a borrowed value.
#[derive(Clone, Copy)]
struct Reader<'de> {
    value: &'de Value,
    /// Containers open above this value.
    depth: u32,
}

impl<'de> Reader<'de> {
    /// The depth of what is inside a container opened here.
    fn inside(self) -> Result<u32, ReadError> {
        if self.depth >= MAX_DEPTH {
            return Err(refuse("a tree at most 128 containers deep"));
        }
        Ok(self.depth + 1)
    }

    fn number(self) -> Option<&'de str> {
        TryAsRef::<Number>::try_as_ref(self.value).map(AsRef::<str>::as_ref)
    }

    fn map(self) -> Option<&'de Map> {
        TryAsRef::<Map>::try_as_ref(self.value)
    }

    fn entries<V: Visitor<'de>>(
        self,
        map: &'de Map,
        named: bool,
        v: V,
    ) -> Result<V::Value, ReadError> {
        v.visit_map(Entries {
            entries: map.entries().iter(),
            pending: None,
            depth: self.inside()?,
            named,
        })
    }
}

/// What a node whose tag and payload disagree is refused as.
fn malformed() -> ReadError {
    refuse("a value whose payload matches its kind")
}

/// A float that reads back the same, or the refusal.
fn float(text: &str) -> Result<f64, ReadError> {
    text.parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| refuse("a number an f64 can hold"))
}

impl<'de> de::Deserializer<'de> for Reader<'de> {
    type Error = ReadError;

    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        let value = self.value;
        match value.tag() {
            Ok(Tag::GUATIAO_NULL) => v.visit_unit(),
            Ok(Tag::GUATIAO_BOOL) => v.visit_bool(bool::from_value(value).map_err(ReadError)?),
            Ok(Tag::GUATIAO_NUMBER) => {
                let text = self.number().ok_or_else(malformed)?;
                if let Ok(n) = text.parse::<i64>() {
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
                    Err(refuse("an integer of at most 128 bits"))
                }
            }
            Ok(Tag::GUATIAO_STRING) => {
                v.visit_borrowed_str(TryAsRef::<str>::try_as_ref(value).ok_or_else(malformed)?)
            }
            Ok(Tag::GUATIAO_BYTES) => {
                v.visit_borrowed_bytes(TryAsRef::<[u8]>::try_as_ref(value).ok_or_else(malformed)?)
            }
            Ok(Tag::GUATIAO_LIST) => {
                let list = TryAsRef::<List>::try_as_ref(value).ok_or_else(malformed)?;
                v.visit_seq(Elements {
                    items: list.iter().enumerate(),
                    depth: self.inside()?,
                })
            }
            Ok(Tag::GUATIAO_MAP) => self.entries(self.map().ok_or_else(malformed)?, false, v),
            _ => Err(refuse(
                "a value serde can read: null, a boolean, a number, text, bytes, a list or a map",
            )),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        self.deserialize_f64(v)
    }

    // An integer too wide for 128 bits is still a float.
    fn deserialize_f64<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        match self.number() {
            Some(text) => v.visit_f64(float(text)?),
            None => self.deserialize_any(v),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        match self.value.tag() {
            Ok(Tag::GUATIAO_NULL | Tag::GUATIAO_ABSENT) => v.visit_none(),
            _ => v.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        v: V,
    ) -> Result<V::Value, ReadError> {
        v.visit_newtype_struct(self)
    }

    // A struct's keys are declared fields, named `a.b` in an error; any
    // other map's are data, named `a[b]`.
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        match self.map() {
            Some(map) => self.entries(map, true, v),
            None => self.deserialize_any(v),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        if let Some(name) = TryAsRef::<str>::try_as_ref(self.value) {
            return v.visit_enum(Variant {
                name,
                content: None,
            });
        }
        match self.map().map(Map::entries) {
            Some([entry]) => v.visit_enum(Variant {
                name: entry.key(),
                content: Some(Reader {
                    value: entry.value(),
                    depth: self.inside()?,
                }),
            }),
            _ => Err(refuse(
                "a variant: its name, or a map of one key, its name, holding its content",
            )),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value, ReadError> {
        v.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 char str string bytes
        byte_buf unit unit_struct seq tuple tuple_struct map identifier
    }
}

struct Elements<'de> {
    items: core::iter::Enumerate<core::slice::Iter<'de, Value>>,
    depth: u32,
}

impl<'de> SeqAccess<'de> for Elements<'de> {
    type Error = ReadError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, ReadError> {
        let Some((i, value)) = self.items.next() else {
            return Ok(None);
        };
        let reader = Reader {
            value,
            depth: self.depth,
        };
        seed.deserialize(reader)
            .map(Some)
            .map_err(|e| ReadError(e.0.at(i)))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

struct Entries<'de> {
    entries: core::slice::Iter<'de, Entry>,
    pending: Option<&'de Entry>,
    depth: u32,
    named: bool,
}

impl<'de> MapAccess<'de> for Entries<'de> {
    type Error = ReadError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ReadError> {
        let Some(entry) = self.entries.next() else {
            return Ok(None);
        };
        self.pending = Some(entry);
        seed.deserialize(BorrowedStrDeserializer::new(entry.key()))
            .map(Some)
    }

    fn next_value_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<T::Value, ReadError> {
        let entry = self
            .pending
            .take()
            .ok_or_else(|| refuse("a key before its value"))?;
        let reader = Reader {
            value: entry.value(),
            depth: self.depth,
        };
        seed.deserialize(reader).map_err(|e| {
            ReadError(if self.named {
                e.0.under(entry.key())
            } else {
                e.0.at_key(entry.key())
            })
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

/// An enum's variant: a bare name, or the one key of a map and what it
/// holds.
struct Variant<'de> {
    name: &'de str,
    content: Option<Reader<'de>>,
}

impl Variant<'_> {
    fn under(&self, e: ReadError) -> ReadError {
        ReadError(e.0.under(self.name))
    }
}

impl<'de> EnumAccess<'de> for Variant<'de> {
    type Error = ReadError;
    type Variant = Variant<'de>;

    fn variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<(T::Value, Variant<'de>), ReadError> {
        let name = seed.deserialize(BorrowedStrDeserializer::new(self.name))?;
        Ok((name, self))
    }
}

impl<'de> VariantAccess<'de> for Variant<'de> {
    type Error = ReadError;

    fn unit_variant(self) -> Result<(), ReadError> {
        match self.content {
            None => Ok(()),
            Some(content) => <()>::deserialize(content).map_err(|e| self.under(e)),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, ReadError> {
        let content = self
            .content
            .ok_or_else(|| refuse("a variant with content"))?;
        seed.deserialize(content).map_err(|e| self.under(e))
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, v: V) -> Result<V::Value, ReadError> {
        let content = self
            .content
            .ok_or_else(|| refuse("a variant with content"))?;
        de::Deserializer::deserialize_seq(content, v).map_err(|e| self.under(e))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        v: V,
    ) -> Result<V::Value, ReadError> {
        let content = self
            .content
            .ok_or_else(|| refuse("a variant with content"))?;
        de::Deserializer::deserialize_struct(content, "", fields, v).map_err(|e| self.under(e))
    }
}
