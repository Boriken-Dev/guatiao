// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Channels: a schema announced before its values, so a receiver knows
//! what each value means and refuses what does not fit it.
//!
//! ```text
//! frame := kind:u8 channel:LEB128 payload
//! kind  := 0 schema   (payload: a schema, wire-encoded)
//!        | 1 value    (payload: a value, wire-encoded)
//!        | 2 close    (no payload)
//! ```
//!
//! Bytes in, bytes out, and no I/O: where one frame ends is the
//! transport's to say — a length prefix on a stream, one datagram, one
//! WebSocket message.
//!
//! ```
//! use guatiao::value::wire::channel::{self, Receiver, Received};
//! use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
//! use guatiao::{Map, Value};
//!
//! let schema: Value = SchemaBuilder::new()
//!     .field(FieldBuilder::new("greeting", KindBuilder::string()))
//!     .finish()?;
//! let mut hello = Map::new();
//! hello.set("greeting", "hello")?;
//!
//! let mut receiver = Receiver::new();
//! receiver.accept(&channel::announce(1, &schema)?)?;
//! match receiver.accept(&channel::send(1, &hello.into())?)? {
//!     Received::Value { channel: 1, value } => assert!(value.tag().is_ok()),
//!     other => panic!("{other:?}"),
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use super::{Reader, WireError, decode_at, encode, put_len};
use crate::schema::read::SchemaRef;
use crate::schema::validate::validate_map;
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::types::Value;

/// A frame that announces a channel's schema.
pub const SCHEMA: u8 = 0;
/// A frame that carries one value.
pub const VALUE: u8 = 1;
/// A frame that ends a channel.
pub const CLOSE: u8 = 2;

fn frame(kind: u8, channel: u64, value: Option<&Value>) -> Result<Vec<u8>, ValueError> {
    let mut out = vec![kind];
    put_len(&mut out, channel);
    if let Some(value) = value {
        encode(value, &mut out)?;
    }
    Ok(out)
}

/// The frame that announces `schema` on `channel`. A later announcement
/// replaces it.
pub fn announce(channel: u64, schema: &Value) -> Result<Vec<u8>, ValueError> {
    frame(SCHEMA, channel, Some(schema))
}

/// The frame that carries `value` on `channel`.
pub fn send(channel: u64, value: &Value) -> Result<Vec<u8>, ValueError> {
    frame(VALUE, channel, Some(value))
}

/// The frame that ends `channel`: its schema is forgotten.
pub fn close(channel: u64) -> Vec<u8> {
    frame(CLOSE, channel, None).expect("a close frame encodes no value")
}

/// What a frame said.
#[derive(Debug)]
pub enum Received {
    /// A schema, now the channel's.
    Schema(u64),
    /// A value its channel's schema accepts.
    Value {
        /// The channel it came on.
        channel: u64,
        /// The value, checked against the channel's schema.
        value: Value,
    },
    /// The channel ended.
    Closed(u64),
}

/// Keeps each channel's schema, and checks every value against it.
#[derive(Debug)]
pub struct Receiver {
    alloc: Alloc,
    schemas: BTreeMap<u64, Value>,
}

impl Default for Receiver {
    fn default() -> Receiver {
        Receiver::new()
    }
}

impl Receiver {
    /// A receiver that builds what it decodes on Rust's heap.
    pub fn new() -> Receiver {
        Receiver::new_in(Alloc::rust())
    }

    /// The same, building through `alloc`.
    pub fn new_in(alloc: Alloc) -> Receiver {
        Receiver {
            alloc,
            schemas: BTreeMap::new(),
        }
    }

    /// The schema announced on `channel`, if any.
    pub fn schema(&self, channel: u64) -> Option<&Value> {
        self.schemas.get(&channel)
    }

    /// Reads one frame. A value arrives only once its channel's schema
    /// has, and only if that schema accepts it; offsets in an error name
    /// bytes of `frame`.
    pub fn accept(&mut self, frame: &[u8]) -> Result<Received, WireError> {
        let kind = *frame.first().ok_or(WireError::Truncated { at: 0 })?;
        let (channel, body) = channel_of(frame)?;
        let payload = &frame[body..];
        match kind {
            SCHEMA => {
                let schema = decode_at(self.alloc, payload, body)?;
                if SchemaRef::new(&schema).is_none() {
                    return Err(WireError::NotASchema { channel });
                }
                self.schemas.insert(channel, schema);
                Ok(Received::Schema(channel))
            }
            VALUE => {
                let schema = self
                    .schemas
                    .get(&channel)
                    .ok_or(WireError::NoSchema { channel })?;
                let value = decode_at(self.alloc, payload, body)?;
                let schema = SchemaRef::new(schema).expect("checked when it was announced");
                validate_map(schema, &value)
                    .map_err(|error| WireError::Invalid { channel, error })?;
                Ok(Received::Value { channel, value })
            }
            CLOSE => {
                if !payload.is_empty() {
                    return Err(WireError::Trailing { at: body });
                }
                self.schemas.remove(&channel);
                Ok(Received::Closed(channel))
            }
            kind => Err(WireError::UnknownFrame { at: 0, kind }),
        }
    }
}

/// The channel number after the kind byte, and where the payload starts.
fn channel_of(frame: &[u8]) -> Result<(u64, usize), WireError> {
    let mut r = Reader {
        bytes: frame,
        at: 1,
        base: 0,
    };
    let channel = r.leb()?;
    Ok((channel, r.at))
}
