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
//!
//! **Streaming.** `Serde<T, S>` also walks and is read from a walk, so a
//! derived type holding one goes to bytes or text and back with no tree:
//! `T`'s `Serialize` writes straight into the visitor, and its
//! `Deserialize` reads straight from the walker.

#![forbid(unsafe_code)]

use core::fmt;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use serde::de::DeserializeOwned;
use serde::ser::Serialize;

use guatiao::schema::KindBuilder;
use guatiao::value::visit::{Flat, FromWalk, TreeSink, Visitor, Walk, Walker};
use guatiao::{Alloc, FromValue, MapError, Schema, ToValue, Value, ValueError};

use crate::bridge::{SerdeWalk, Walking};

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

/// `T`'s `Serialize`, straight into the visitor.
impl<T: Serialize, S> Walk for Serde<T, S> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        SerdeWalk::new(&self.0).walk(visitor)
    }
}

/// `T`'s `Deserialize`, straight from the walker.
impl<T: DeserializeOwned, S> FromWalk for Serde<T, S> {
    fn from_walk<W: Walker>(walker: W) -> Result<Serde<T, S>, MapError> {
        T::deserialize(Walking(walker))
            .map(Serde::new)
            .map_err(|e| e.0)
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
    let mut sink = TreeSink::new_in(alloc);
    SerdeWalk::new(value).walk(Flat(&mut sink))?;
    sink.into_value()
}

/// Reads any serde type out of a value. The error names the key it
/// failed at, below this value.
pub fn from_value<T: DeserializeOwned>(value: &Value) -> Result<T, MapError> {
    T::deserialize(Walking(value)).map_err(|e| e.0)
}
