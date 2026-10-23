// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A sink as a visitor: a container's elements walk into the same sink,
//! between its begin and end events.

#![forbid(unsafe_code)]

use super::{ListVisitor, MapVisitor, Sink, Visitor, Walk};

/// A [`Sink`] as a [`Visitor`], so anything that walks can walk into it:
/// `value.walk(Flat(&mut sink))`.
///
/// A wrapper rather than an impl on `&mut S`, so a type that is a sink
/// keeps one method per name with both traits in scope.
#[derive(Debug)]
pub struct Flat<'a, S: ?Sized>(pub &'a mut S);

impl<'a, S: Sink + ?Sized> Visitor for Flat<'a, S> {
    type Ok = ();
    type Error = S::Error;
    type List = FlatList<'a, S>;
    type Map = FlatMap<'a, S>;

    fn absent(self) -> Result<(), S::Error> {
        self.0.absent()
    }

    fn null(self) -> Result<(), S::Error> {
        self.0.null()
    }

    fn bool(self, value: bool) -> Result<(), S::Error> {
        self.0.bool(value)
    }

    fn number(self, text: &str) -> Result<(), S::Error> {
        self.0.number(text)
    }

    fn string(self, text: &str) -> Result<(), S::Error> {
        self.0.string(text)
    }

    fn bytes(self, bytes: &[u8]) -> Result<(), S::Error> {
        self.0.bytes(bytes)
    }

    fn list(self, len: Option<usize>) -> Result<FlatList<'a, S>, S::Error> {
        self.0.list_begin(len)?;
        Ok(FlatList(self.0))
    }

    fn map(self, len: Option<usize>) -> Result<FlatMap<'a, S>, S::Error> {
        self.0.map_begin(len)?;
        Ok(FlatMap(self.0))
    }
}

/// A list a [`Sink`] has open.
#[derive(Debug)]
pub struct FlatList<'a, S: ?Sized>(&'a mut S);

impl<S: Sink + ?Sized> ListVisitor for FlatList<'_, S> {
    type Ok = ();
    type Error = S::Error;

    fn item<W: Walk + ?Sized>(&mut self, item: &W) -> Result<(), S::Error> {
        item.walk(Flat(&mut *self.0))
    }

    fn end(self) -> Result<(), S::Error> {
        self.0.list_end()
    }
}

/// A map a [`Sink`] has open.
#[derive(Debug)]
pub struct FlatMap<'a, S: ?Sized>(&'a mut S);

impl<S: Sink + ?Sized> MapVisitor for FlatMap<'_, S> {
    type Ok = ();
    type Error = S::Error;

    fn entry<W: Walk + ?Sized>(&mut self, key: &str, value: &W) -> Result<(), S::Error> {
        self.0.key(key)?;
        value.walk(Flat(&mut *self.0))
    }

    fn end(self) -> Result<(), S::Error> {
        self.0.map_end()
    }
}
