// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! serde and guatiao's own visitor, joined four ways, with no tree:
//!
//! - **a guatiao walk written by any serde format**: [`Walked`] is
//!   `Serialize` for anything that walks;
//! - **any serde type walked into guatiao**: [`Serde<T>`](crate::Serde)
//!   walks through `T`'s `Serialize`;
//! - **any serde format read into a guatiao type**: [`from_deserializer`]
//!   reads a `FromWalk` type from any `Deserializer`, and
//!   [`deserialize_into`] hands a document's events to a
//!   [`Sink`](guatiao::value::visit::Sink);
//! - **any serde type read from a guatiao walker**: `Serde<T>` reads
//!   through `T`'s `Deserialize`.
//!
//! What crosses is serde's data model as the value model has it: a struct
//! is a map, a unit variant its name, any other variant a map of one key;
//! a number is its text, `serde_json`'s own under `arbitrary_precision`
//! included, and a TOML date or time the text it was written as. A
//! document nests no deeper than [`MAX_DEPTH`](guatiao::MAX_DEPTH)
//! containers.

#![forbid(unsafe_code)]

mod de;
mod number;
mod ser;
mod sink;
mod visit;
mod walker;

use core::fmt;

pub(crate) use de::Walking;
pub use ser::Walked;
pub(crate) use ser::serialize_walk;
pub(crate) use sink::ToSink;
pub use sink::deserialize_into;
pub(crate) use visit::SerdeWalk;
pub use walker::from_deserializer;

/// Why a bridge stopped: the format, or the guatiao side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError<F, R> {
    /// The format refused: a document that does not parse, an I/O error,
    /// a value it cannot write.
    Format(F),
    /// The guatiao side refused: a type's reader, or a sink.
    Refused(R),
}

impl<F: fmt::Display, R: fmt::Display> fmt::Display for BridgeError<F, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Format(e) => e.fmt(f),
            BridgeError::Refused(e) => e.fmt(f),
        }
    }
}

impl<F: fmt::Debug + fmt::Display, R: fmt::Debug + fmt::Display> core::error::Error
    for BridgeError<F, R>
{
}
