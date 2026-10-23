// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One event, handed on: how a builder that has already been handed a
//! value (an `Option` that saw it was not null, an enum that picked a
//! variant by its kind) lets another type read it.

#![forbid(unsafe_code)]

use super::{Build, ListWalker, MapWalker, Seed, Walker};
use crate::value::convert::MapError;

/// The one event a [`Build`] method was called with, as a [`Walker`]
/// that calls the same method of the next builder.
///
/// `L` and `M` are the list and map walkers a container arrives with;
/// [`NoWalker`] stands in for the one a scalar does not have.
///
/// ```
/// use guatiao::Kinds;
/// use guatiao::MapError;
/// use guatiao::value::visit::{Build, FromWalk, NoWalker, Replay, Walker};
///
/// /// Reads a `T`, counting the strings among what it reads.
/// struct Counted<T>(T, bool);
///
/// struct CountedBuild<T>(core::marker::PhantomData<T>);
///
/// impl<T: FromWalk> Build for CountedBuild<T> {
///     type Out = Counted<T>;
///     fn kinds(&self) -> Kinds {
///         Kinds::ANY
///     }
///     fn string(self, text: &str) -> Result<Counted<T>, MapError> {
///         let value = T::from_walk(Replay::<NoWalker, NoWalker>::String(text))?;
///         Ok(Counted(value, true))
///     }
/// }
/// # let _ = CountedBuild::<String>(core::marker::PhantomData);
/// ```
#[derive(Debug)]
pub enum Replay<'a, L, M> {
    /// `GUATIAO_ABSENT`.
    Absent,
    /// Null.
    Null,
    /// A boolean.
    Bool(bool),
    /// A number's text.
    Number(&'a str),
    /// A number a walker had as an `i64`.
    I64(i64),
    /// A number a walker had as a `u64`.
    U64(u64),
    /// A number a walker had as an `f64`.
    F64(f64),
    /// A string.
    String(&'a str),
    /// A byte string.
    Bytes(&'a [u8]),
    /// A list's elements.
    List(L),
    /// A map's entries.
    Map(M),
}

impl<L: ListWalker, M: MapWalker> Walker for Replay<'_, L, M> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        match self {
            Replay::Absent => build.absent(),
            Replay::Null => build.null(),
            Replay::Bool(v) => build.bool(v),
            Replay::Number(v) => build.number(v),
            Replay::I64(v) => build.i64(v),
            Replay::U64(v) => build.u64(v),
            Replay::F64(v) => build.f64(v),
            Replay::String(v) => build.string(v),
            Replay::Bytes(v) => build.bytes(v),
            Replay::List(items) => build.list(items),
            Replay::Map(entries) => build.map(entries),
        }
    }
}

/// The list or map walker of a [`Replay`] of a scalar, which has none:
/// it has no values, so none can be made.
#[derive(Debug, Clone, Copy)]
pub enum NoWalker {}

impl ListWalker for NoWalker {
    fn len_hint(&self) -> Option<usize> {
        match *self {}
    }

    fn next_seed<S: Seed>(&mut self, _: S) -> Result<Option<S::Out>, MapError> {
        match *self {}
    }
}

impl MapWalker for NoWalker {
    fn len_hint(&self) -> Option<usize> {
        match *self {}
    }

    fn next_key<R>(&mut self, _: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        match *self {}
    }

    fn value_seed<S: Seed>(&mut self, _: S) -> Result<S::Out, MapError> {
        match *self {}
    }
}
