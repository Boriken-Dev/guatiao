// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What `#[derive(ToValue)]` and `#[derive(FromValue)]` write their walks
//! with, reached through `guatiao::__private`. Nothing here is for a
//! person to call.
//!
//! A derived walk takes each field through the field type's own [`Walk`]
//! or [`FromWalk`], and through its tree (`ToValue`, `FromValue`) when it
//! has only those: a type with hand-written tree impls stays a field of a
//! derived one. Which is chosen at compile time, by method resolution:
//! the walking impl is on `&Field<T>`, the tree one on `Field<T>`, and the
//! expansion calls `(&&Field(..)).__walkable()`, so the first one whose
//! bound holds is taken.

#![forbid(unsafe_code)]
// Their constructors exist for generated code to call; a `Default` would
// be a second spelling nobody uses.
#![allow(clippy::new_without_default)]

use alloc::string::ToString;
use core::fmt;
use core::marker::PhantomData;
use core::str::FromStr;

use super::{Build, FromWalk, ListWalker, MapWalker, Seed, Visitor, Walk, Walker};
use crate::value::alloc::Alloc;
use crate::value::convert::{FromValue, Kinds, MapError, ToValue};
use crate::value::types::Value;

/// A field, to pick how it walks.
#[derive(Debug)]
pub struct Field<'a, T: ?Sized>(pub &'a T);

/// A field that walks itself.
#[derive(Debug)]
pub struct ByWalk<'a, T: ?Sized>(&'a T);

/// A field that only builds a tree, walked as that tree.
#[derive(Debug)]
pub struct ByTree<'a, T: ?Sized>(&'a T);

impl<T: Walk + ?Sized> Walk for ByWalk<'_, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        self.0.walk(visitor)
    }
}

impl<T: ToValue + ?Sized> Walk for ByTree<'_, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        self.0.to_value_in(Alloc::rust())?.walk(visitor)
    }
}

/// The walking choice, preferred.
pub trait WalkField<'a, T: ?Sized> {
    /// The field, walking itself.
    fn __walkable(&self) -> ByWalk<'a, T>;
}

/// The tree choice, for a field type with no walk.
pub trait TreeField<'a, T: ?Sized> {
    /// The field, walked as its tree.
    fn __walkable(&self) -> ByTree<'a, T>;
}

impl<'a, T: Walk + ?Sized> WalkField<'a, T> for &Field<'a, T> {
    fn __walkable(&self) -> ByWalk<'a, T> {
        ByWalk(self.0)
    }
}

impl<'a, T: ToValue + ?Sized> TreeField<'a, T> for Field<'a, T> {
    fn __walkable(&self) -> ByTree<'a, T> {
        ByTree(self.0)
    }
}

/// A `#[map(as_text)]` field: its `Display` text, as a string.
#[derive(Debug)]
pub struct AsText<'a, T: ?Sized>(pub &'a T);

impl<T: fmt::Display + ?Sized> Walk for AsText<'_, T> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        visitor.string(&self.0.to_string())
    }
}

/// A type, to pick how a field of it is read.
#[derive(Debug)]
pub struct Pick<T>(PhantomData<fn() -> T>);

impl<T> Pick<T> {
    /// The choice for `T`.
    pub const fn new() -> Pick<T> {
        Pick(PhantomData)
    }
}

/// A field type read as its tree: the value is built, then read.
#[derive(Debug)]
pub struct ByTreeSeed<T>(PhantomData<fn() -> T>);

impl<T: FromValue> Seed for ByTreeSeed<T> {
    type Out = T;

    fn read<W: Walker>(self, walker: W) -> Result<T, MapError> {
        T::from_value(&Value::from_walk(walker)?)
    }
}

/// The walking choice, preferred.
pub trait WalkSeed<T> {
    /// How the field is read.
    fn __seed(&self) -> PhantomData<T>;
}

/// The tree choice, for a field type with no walk.
pub trait TreeSeed<T> {
    /// How the field is read.
    fn __seed(&self) -> ByTreeSeed<T>;
}

impl<T: FromWalk> WalkSeed<T> for &Pick<T> {
    fn __seed(&self) -> PhantomData<T> {
        PhantomData
    }
}

impl<T: FromValue> TreeSeed<T> for Pick<T> {
    fn __seed(&self) -> ByTreeSeed<T> {
        ByTreeSeed(PhantomData)
    }
}

/// A `#[map(with = ..)]` field read: the value is built, then the
/// module's `from_value` reads it.
#[derive(Debug)]
pub struct WithSeed<F>(pub F);

impl<T, F: FnOnce(&Value) -> Result<T, MapError>> Seed for WithSeed<F> {
    type Out = T;

    fn read<W: Walker>(self, walker: W) -> Result<T, MapError> {
        (self.0)(&Value::from_walk(walker)?)
    }
}

/// A `#[map(as_text)]` field read: text through its `FromStr`, refused
/// in the parse error's own words, as `from_text` refuses it.
#[derive(Debug)]
pub struct TextSeed<T>(PhantomData<fn() -> T>);

impl<T> TextSeed<T> {
    /// The reader for `T`.
    pub const fn new() -> TextSeed<T> {
        TextSeed(PhantomData)
    }
}

fn parsed<T: FromStr>(text: &str) -> Result<T, MapError>
where
    T::Err: fmt::Display,
{
    text.parse()
        .map_err(|e: T::Err| MapError::bad_value(alloc::format!("text this field reads ({e})")))
}

impl<T: FromStr> Seed for TextSeed<T>
where
    T::Err: fmt::Display,
{
    type Out = T;

    fn read<W: Walker>(self, walker: W) -> Result<T, MapError> {
        parsed(&alloc::string::String::from_walk(walker)?)
    }
}

/// The same for an `Option<T>` field: a stored null is `None`.
#[derive(Debug)]
pub struct TextOrNullSeed<T>(PhantomData<fn() -> T>);

impl<T> TextOrNullSeed<T> {
    /// The reader for `T`.
    pub const fn new() -> TextOrNullSeed<T> {
        TextOrNullSeed(PhantomData)
    }
}

struct TextOrNullBuild<T>(PhantomData<fn() -> T>);

impl<T: FromStr> Build for TextOrNullBuild<T>
where
    T::Err: fmt::Display,
{
    type Out = Option<T>;

    fn kinds(&self) -> Kinds {
        Kinds::STRING
    }

    fn null(self) -> Result<Option<T>, MapError> {
        Ok(None)
    }

    fn string(self, text: &str) -> Result<Option<T>, MapError> {
        parsed(text).map(Some)
    }
}

impl<T: FromStr> Seed for TextOrNullSeed<T>
where
    T::Err: fmt::Display,
{
    type Out = Option<T>;

    fn read<W: Walker>(self, walker: W) -> Result<Option<T>, MapError> {
        walker.build(TextOrNullBuild(PhantomData))
    }
}

/// A struct whose fields are read from a map's entries: what the derive
/// writes for a struct with named fields.
pub trait MapRead: Sized {
    /// Reads every entry.
    fn read_map<M: MapWalker>(entries: M) -> Result<Self, MapError>;
}

/// The builder of a [`MapRead`] type.
#[derive(Debug)]
pub struct ReadMap<T>(PhantomData<fn() -> T>);

impl<T> ReadMap<T> {
    /// The builder for `T`.
    pub const fn new() -> ReadMap<T> {
        ReadMap(PhantomData)
    }
}

impl<T: MapRead> Build for ReadMap<T> {
    type Out = T;

    fn kinds(&self) -> Kinds {
        Kinds::MAP
    }

    fn map<M: MapWalker>(self, entries: M) -> Result<T, MapError> {
        T::read_map(entries)
    }
}

/// A struct whose fields are read by position from a list: what the
/// derive writes for a tuple struct.
pub trait ListRead: Sized {
    /// Reads every element.
    fn read_list<L: ListWalker>(items: L) -> Result<Self, MapError>;
}

/// The builder of a [`ListRead`] type.
#[derive(Debug)]
pub struct ReadList<T>(PhantomData<fn() -> T>);

impl<T> ReadList<T> {
    /// The builder for `T`.
    pub const fn new() -> ReadList<T> {
        ReadList(PhantomData)
    }
}

impl<T: ListRead> Build for ReadList<T> {
    type Out = T;

    fn kinds(&self) -> Kinds {
        Kinds::LIST
    }

    fn list<L: ListWalker>(self, items: L) -> Result<T, MapError> {
        T::read_list(items)
    }
}

/// A type walked as the tree its `ToValue` builds: an enum, or a struct
/// with a flattened member.
pub fn walk_tree<T: ToValue + ?Sized, V: Visitor>(
    value: &T,
    visitor: V,
) -> Result<V::Ok, V::Error> {
    value.to_value_in(Alloc::rust())?.walk(visitor)
}

/// A type read as its `FromValue` reads the tree a walk holds.
pub fn read_tree<T: FromValue, W: Walker>(walker: W) -> Result<T, MapError> {
    T::from_value(&Value::from_walk(walker)?)
}
