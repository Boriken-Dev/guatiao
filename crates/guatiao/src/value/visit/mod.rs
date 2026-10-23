// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A value as a stream of events, with no tree in the middle.
//!
//! The events are the value model's own kinds: absent, null, a bool, a
//! number as its text, a string, bytes, a list of values and a map of
//! keyed values. A type that **walks** itself hands them to a visitor;
//! a type that is **built** from a walk takes them from a walker. The
//! wire encoding, a tree and any format a bridge crate reaches are each
//! one implementation, and a struct goes to bytes, or text, and back
//! without a [`Value`](crate::Value) being built.
//!
//! # Writing
//!
//! [`Walk`] is implemented by what can be written: it calls one method
//! of a [`Visitor`] for its value, and for a list or a map hands each
//! element to the [`ListVisitor`] or [`MapVisitor`] that call returned,
//! as something that walks itself in turn.
//!
//! ```
//! use guatiao::value::visit::{MapVisitor, Visitor, Walk};
//!
//! struct Endpoint {
//!     host: String,
//!     port: u16,
//! }
//!
//! impl Walk for Endpoint {
//!     fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
//!         let mut map = visitor.map(Some(2))?;
//!         map.entry("host", &self.host)?;
//!         map.entry("port", &self.port)?;
//!         map.end()
//!     }
//! }
//!
//! let bytes = guatiao::value::wire::to_bytes_of(&Endpoint { host: "h".into(), port: 1 })?;
//! # Ok::<(), guatiao::ValueError>(())
//! ```
//!
//! A [`Sink`] is the same events as `&mut self` calls on one type,
//! with `list_begin` and `list_end` around the elements: the easier
//! thing to implement, and what the C interface's visitor table is.
//! [`Flat`] makes any sink a [`Visitor`].
//!
//! # Reading
//!
//! [`FromWalk`] is implemented by what can be read: it hands a
//! [`Build`] to a [`Walker`], which calls the one method for the value
//! it holds. A list or a map arrives as a [`ListWalker`] or
//! [`MapWalker`], which the builder pulls each element from.
//!
//! ```
//! use guatiao::MapError;
//! use guatiao::value::visit::{Build, FromWalk, MapWalker, Walker};
//! use guatiao::Kinds;
//!
//! #[derive(Debug, PartialEq)]
//! struct Endpoint {
//!     host: String,
//!     port: u16,
//! }
//!
//! struct EndpointBuild;
//!
//! impl Build for EndpointBuild {
//!     type Out = Endpoint;
//!
//!     fn kinds(&self) -> Kinds {
//!         Kinds::MAP
//!     }
//!
//!     fn map<M: MapWalker>(self, mut map: M) -> Result<Endpoint, MapError> {
//!         let (mut host, mut port) = (None, None);
//!         while let Some(key) = map.next_key(|key| key == "host")? {
//!             if key {
//!                 host = Some(map.value::<String>().map_err(|e| e.under("host"))?);
//!             } else {
//!                 port = Some(map.value::<u16>().map_err(|e| e.under("port"))?);
//!             }
//!         }
//!         Ok(Endpoint {
//!             host: host.ok_or_else(|| MapError::missing("host"))?,
//!             port: port.ok_or_else(|| MapError::missing("port"))?,
//!         })
//!     }
//! }
//!
//! impl FromWalk for Endpoint {
//!     fn from_walk<W: Walker>(walker: W) -> Result<Endpoint, MapError> {
//!         walker.walk(EndpointBuild)
//!     }
//! }
//!
//! let mut map = guatiao::Map::new();
//! map.set("host", "h");
//! map.set("port", 1);
//! let value = guatiao::Value::from(map);
//! assert_eq!(Endpoint::from_walk(&value)?, Endpoint { host: "h".into(), port: 1 });
//! # Ok::<(), MapError>(())
//! ```
//!
//! # The rules every implementation keeps
//!
//! - **A number is its text**, in the JSON number grammar (RFC 8259
//!   §6), and every sink checks it. [`Visitor::i64`], `u64` and `f64`
//!   are shortcuts that mean the text an integer's or a float's
//!   `Display` writes, the same text [`ToValue`](crate::ToValue) stores,
//!   so a format writes one number one way whichever path reached it.
//! - **A length is a promise when given**: `Some(n)` says exactly `n`
//!   elements follow, and a sink that writes the count first refuses
//!   any other number; `None` says it is not known.
//! - **Keys are text, and a map keeps their order.**
//! - **Depth is bounded by the source.** A walker over foreign input
//!   (bytes, a document, a tree a C caller built) refuses to open more
//!   than [`MAX_DEPTH`](crate::MAX_DEPTH) containers, so a builder that
//!   recurses once per container cannot overflow the stack.
//! - **Reading refuses with a [`MapError`]** naming the key it failed
//!   under, built on the way out as the tree readers do. A walker whose
//!   own input is wrong (bytes that are not the wire encoding, a
//!   document that does not parse) keeps that error itself and answers
//!   [`MapError::Stopped`]; the function that started the walk reports
//!   the walker's error.

#![forbid(unsafe_code)]

mod number;
mod read;
mod replay;
mod sink;
mod tree;
mod write;

use core::marker::PhantomData;

pub use replay::{NoWalker, Replay};
pub use sink::{Flat, FlatList, FlatMap};
pub use tree::TreeSink;

use super::convert::{Kinds, MapError};
use super::error::ValueError;
use super::types::Tag;

// --- writing -----------------------------------------------------------------

/// A value that can be written as events: the writing half of the model.
///
/// Implemented for the scalars, the strings, the containers of the
/// standard library, the value model's own types, and by
/// `#[derive(ToValue)]`.
pub trait Walk {
    /// Hands this value to `visitor`: one call for a scalar; for a list
    /// or a map, the call that opens it, then each element, then `end`.
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error>;
}

/// What receives one value's events.
///
/// Every method consumes the visitor, so exactly one is called per
/// value. A list or a map is opened with [`list`](Visitor::list) or
/// [`map`](Visitor::map), which answer the visitor its elements go to.
pub trait Visitor: Sized {
    /// What a finished value answers.
    type Ok;
    /// Why a value was refused. A walk that cannot be written (a float
    /// that is not finite, a tree deeper than a walk follows) reports a
    /// [`ValueError`].
    type Error: From<ValueError>;
    /// What a list's elements go to.
    type List: ListVisitor<Ok = Self::Ok, Error = Self::Error>;
    /// What a map's entries go to.
    type Map: MapVisitor<Ok = Self::Ok, Error = Self::Error>;

    /// `GUATIAO_ABSENT`: there is no value here.
    fn absent(self) -> Result<Self::Ok, Self::Error>;
    /// Null.
    fn null(self) -> Result<Self::Ok, Self::Error>;
    /// A boolean.
    fn bool(self, value: bool) -> Result<Self::Ok, Self::Error>;
    /// A number, as its text in the JSON number grammar.
    fn number(self, text: &str) -> Result<Self::Ok, Self::Error>;
    /// A number given as an `i64`: the text its `Display` writes.
    fn i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
        self.number(number::Text::int(value).as_str())
    }
    /// A number given as a `u64`: the text its `Display` writes.
    fn u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
        self.number(number::Text::uint(value).as_str())
    }
    /// A number given as an `f64`: the text its `Display` writes, which is
    /// what [`ToValue`](crate::ToValue) stores. One that is not finite has
    /// no text and is refused with [`ValueError::NotANumber`].
    fn f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        let text = number::Text::float(value).ok_or(ValueError::NotANumber)?;
        self.number(text.as_str())
    }
    /// A string.
    fn string(self, text: &str) -> Result<Self::Ok, Self::Error>;
    /// A byte string.
    fn bytes(self, bytes: &[u8]) -> Result<Self::Ok, Self::Error>;
    /// Opens a list; `len`, when given, is exactly how many elements
    /// follow.
    fn list(self, len: Option<usize>) -> Result<Self::List, Self::Error>;
    /// Opens a map; `len`, when given, is exactly how many entries follow.
    fn map(self, len: Option<usize>) -> Result<Self::Map, Self::Error>;
}

/// A list being visited.
pub trait ListVisitor {
    /// What the finished list answers.
    type Ok;
    /// Why an element, or the list, was refused.
    type Error;
    /// One element, which walks itself into this list.
    fn item<W: Walk + ?Sized>(&mut self, item: &W) -> Result<(), Self::Error>;
    /// The list is complete.
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

/// A map being visited.
pub trait MapVisitor {
    /// What the finished map answers.
    type Ok;
    /// Why an entry, or the map, was refused.
    type Error;
    /// One entry: its key, and its value, which walks itself.
    fn entry<W: Walk + ?Sized>(&mut self, key: &str, value: &W) -> Result<(), Self::Error>;
    /// The map is complete.
    fn end(self) -> Result<Self::Ok, Self::Error>;
}

/// The events as calls on one type, each container closed by its own
/// event: what a sink that keeps its own stack implements, and what the
/// C interface's visitor table is.
///
/// [`Flat`]`(&mut sink)` is a [`Visitor`], so a sink receives any walk. A sink
/// refuses events out of order (an end with nothing open, a key outside
/// a map, a value where a key is due) with its own error.
pub trait Sink {
    /// Why an event was refused.
    type Error: From<ValueError>;

    /// `GUATIAO_ABSENT`.
    fn absent(&mut self) -> Result<(), Self::Error>;
    /// Null.
    fn null(&mut self) -> Result<(), Self::Error>;
    /// A boolean.
    fn bool(&mut self, value: bool) -> Result<(), Self::Error>;
    /// A number, as its text in the JSON number grammar.
    fn number(&mut self, text: &str) -> Result<(), Self::Error>;
    /// A string.
    fn string(&mut self, text: &str) -> Result<(), Self::Error>;
    /// A byte string.
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
    /// A list opens; `len`, when given, is exactly how many values follow.
    fn list_begin(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    /// The innermost open list closes.
    fn list_end(&mut self) -> Result<(), Self::Error>;
    /// A map opens; `len`, when given, is exactly how many entries follow.
    fn map_begin(&mut self, len: Option<usize>) -> Result<(), Self::Error>;
    /// The next entry's key; its value is the next value.
    fn key(&mut self, key: &str) -> Result<(), Self::Error>;
    /// The innermost open map closes.
    fn map_end(&mut self) -> Result<(), Self::Error>;
}

// --- reading -----------------------------------------------------------------

/// A type that can be read from a walk: the reading half of the model.
///
/// Implemented for the types [`FromValue`](crate::FromValue) reads and
/// by `#[derive(FromValue)]`, refusing what `FromValue` refuses with the
/// same errors. Reading copies out of the walker: nothing returned
/// borrows from it.
pub trait FromWalk: Sized {
    /// Reads `Self` from the one value `walker` holds.
    fn from_walk<W: Walker>(walker: W) -> Result<Self, MapError>;
}

/// A source of one value: hands it to a [`Build`].
///
/// `&Value` is one, and so are a wire decoder and a bridge to another
/// format. A walker is consumed by the walk; one that reads from a
/// buffer is implemented for `&mut` the reader.
pub trait Walker: Sized {
    /// Calls the one method of `build` for the value this walker holds.
    fn walk<B: Build>(self, build: B) -> Result<B::Out, MapError>;
}

/// What a walker hands one value to, which makes `Out` from it.
///
/// Every method has a default refusing the kind it was handed, with
/// [`kinds`](Build::kinds) as what was expected: a builder implements
/// the ones it takes.
pub trait Build: Sized {
    /// What it builds.
    type Out;

    /// The kinds it takes, for the refusal of any other.
    fn kinds(&self) -> Kinds;

    /// `GUATIAO_ABSENT`.
    fn absent(self) -> Result<Self::Out, MapError> {
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_ABSENT))
    }
    /// Null.
    fn null(self) -> Result<Self::Out, MapError> {
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_NULL))
    }
    /// A boolean.
    fn bool(self, value: bool) -> Result<Self::Out, MapError> {
        let _ = value;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_BOOL))
    }
    /// A number, as its text in the JSON number grammar.
    fn number(self, text: &str) -> Result<Self::Out, MapError> {
        let _ = text;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_NUMBER))
    }
    /// A number a walker had as an `i64`: by default, its text.
    fn i64(self, value: i64) -> Result<Self::Out, MapError> {
        self.number(number::Text::int(value).as_str())
    }
    /// A number a walker had as a `u64`: by default, its text.
    fn u64(self, value: u64) -> Result<Self::Out, MapError> {
        self.number(number::Text::uint(value).as_str())
    }
    /// A number a walker had as an `f64`, such as a format that parsed
    /// it: by default, the shortest text that reads back the same float,
    /// keeping a fraction (`1.0` stays `1.0`), as a value read from such
    /// a format holds it. One that is not finite is no number.
    fn f64(self, value: f64) -> Result<Self::Out, MapError> {
        match number::Text::shortest(value) {
            Some(text) => self.number(text.as_str()),
            None => Err(MapError::bad_value("a number with a finite value")),
        }
    }
    /// A string.
    fn string(self, text: &str) -> Result<Self::Out, MapError> {
        let _ = text;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_STRING))
    }
    /// A byte string.
    fn bytes(self, bytes: &[u8]) -> Result<Self::Out, MapError> {
        let _ = bytes;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_BYTES))
    }
    /// A list, whose elements `items` hands out in order.
    fn list<L: ListWalker>(self, items: L) -> Result<Self::Out, MapError> {
        let _ = items;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_LIST))
    }
    /// A map, whose entries `entries` hands out in order.
    fn map<M: MapWalker>(self, entries: M) -> Result<Self::Out, MapError> {
        let _ = entries;
        Err(MapError::wrong_kind(self.kinds(), Tag::GUATIAO_MAP))
    }
}

/// What reads one value from a walker, carrying whatever it needs to:
/// the stateful form of [`FromWalk`], which `PhantomData<T>` is for any
/// `T: FromWalk`.
pub trait Seed: Sized {
    /// What it reads.
    type Out;
    /// Reads from the one value `walker` holds.
    fn read<W: Walker>(self, walker: W) -> Result<Self::Out, MapError>;
}

impl<T: FromWalk> Seed for PhantomData<T> {
    type Out = T;

    fn read<W: Walker>(self, walker: W) -> Result<T, MapError> {
        T::from_walk(walker)
    }
}

/// A list's elements, pulled in order.
pub trait ListWalker {
    /// How many elements are left, when the walker knows.
    fn len_hint(&self) -> Option<usize>;
    /// The next element, read by `seed`, or `None` past the last.
    fn next_seed<S: Seed>(&mut self, seed: S) -> Result<Option<S::Out>, MapError>;
    /// The next element as a `T`, or `None` past the last.
    fn next<T: FromWalk>(&mut self) -> Result<Option<T>, MapError> {
        self.next_seed(PhantomData::<T>)
    }
}

/// A map's entries, pulled in order: a key, then its value, then the
/// next key.
pub trait MapWalker {
    /// How many entries are left, when the walker knows.
    fn len_hint(&self) -> Option<usize>;
    /// The next key, handed to `read` while it is borrowed, or `None`
    /// past the last entry. Its value must be read or skipped before the
    /// next key.
    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError>;
    /// The value under the key just read, read by `seed`.
    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError>;
    /// The value under the key just read, as a `T`.
    fn value<T: FromWalk>(&mut self) -> Result<T, MapError> {
        self.value_seed(PhantomData::<T>)
    }
    /// Passes over the value under the key just read.
    fn skip_value(&mut self) -> Result<(), MapError> {
        self.value::<read::Ignored>().map(|_| ())
    }
}
