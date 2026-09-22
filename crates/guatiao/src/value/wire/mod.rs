// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A value as bytes, exactly, for whatever carries bytes.
//!
//! Every node is its tag's byte and then its body; a length or a count is
//! an unsigned LEB128; a map's entries keep their order. A number crosses
//! as its own text and bytes cross as bytes, so what arrives is the tree
//! that was sent — `1.10` stays `1.10`, a STRING is never mistaken for
//! BYTES — and the receiver needs nothing but the bytes to know what it
//! holds.
//!
//! ```text
//! value := tag:u8 body
//! body  := ε                        ABSENT, NULL
//!        | 0 | 1                    BOOL
//!        | len text                 NUMBER (the JSON grammar), STRING (UTF-8)
//!        | len bytes                BYTES
//!        | count value*             LIST
//!        | count (len key value)*   MAP (keys UTF-8, none repeated)
//! ```
//!
//! No magic and no version: a frame around the bytes says what they are
//! (see [`channel`]). The encoding is the same on every pointer width.
//!
//! Both directions are loops over an explicit stack, as copying is, so
//! depth costs heap, never the thread's stack. Decoding refuses, with the
//! byte offset, everything a sender can get wrong: an unknown tag, a
//! truncated body, a length that is not the shortest LEB128, text that is
//! not UTF-8, a number outside the grammar, a bool byte other than 0 or 1,
//! a key repeated, bytes after the value.

#![forbid(unsafe_code)]

pub mod channel;

use std::fmt;

use super::alloc::Alloc;
use super::convert::TryAsRef;
use super::error::ValueError;
use super::types::{Buffer, Entry, List, Map, Number, Tag, Text, Value};

/// Why bytes are not a value. Every variant says where, as a byte offset
/// into what was given.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WireError {
    /// The bytes end inside a node.
    Truncated {
        /// Where the missing byte would have been.
        at: usize,
    },
    /// A tag byte this build does not know.
    UnknownTag {
        /// The tag byte's offset.
        at: usize,
        /// The byte.
        tag: u8,
    },
    /// A length or count that is not the shortest LEB128, or overflows.
    Overlong {
        /// Where the length starts.
        at: usize,
    },
    /// Text that is not UTF-8: a STRING, or a key.
    NotUtf8 {
        /// Where its length starts.
        at: usize,
    },
    /// A NUMBER whose text is outside the JSON grammar.
    NotANumber {
        /// Where its length starts.
        at: usize,
    },
    /// A BOOL whose byte is neither 0 nor 1.
    NotABool {
        /// The byte's offset.
        at: usize,
        /// The byte.
        byte: u8,
    },
    /// A map key that came before in the same map.
    DuplicateKey {
        /// Where the repeated key's length starts.
        at: usize,
    },
    /// Bytes after the value ended.
    Trailing {
        /// The first byte past the value.
        at: usize,
    },
    /// The allocator refused while the value was being built.
    Alloc {
        /// The node being built.
        at: usize,
        /// What the allocator said.
        error: ValueError,
    },
    /// A frame whose kind byte is not one [`channel`] defines.
    UnknownFrame {
        /// The kind byte's offset.
        at: usize,
        /// The byte.
        kind: u8,
    },
    /// A value on a channel no schema was announced for.
    NoSchema {
        /// The channel.
        channel: u64,
    },
    /// A schema frame whose value is not a schema.
    NotASchema {
        /// The channel.
        channel: u64,
    },
    /// A value its channel's schema refuses.
    Invalid {
        /// The channel.
        channel: u64,
        /// Why the schema refused it.
        error: crate::schema::ValidationError,
    },
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Truncated { at } => write!(f, "the bytes end inside a node at {at}"),
            WireError::UnknownTag { at, tag } => write!(f, "unknown tag {tag} at {at}"),
            WireError::Overlong { at } => {
                write!(f, "a length that is not the shortest LEB128 at {at}")
            }
            WireError::NotUtf8 { at } => write!(f, "text that is not UTF-8 at {at}"),
            WireError::NotANumber { at } => write!(f, "a number outside the JSON grammar at {at}"),
            WireError::NotABool { at, byte } => write!(f, "a bool byte of {byte} at {at}"),
            WireError::DuplicateKey { at } => write!(f, "a key repeated in its map at {at}"),
            WireError::Trailing { at } => write!(f, "bytes after the value, from {at}"),
            WireError::Alloc { at, error } => write!(f, "{error} at {at}"),
            WireError::UnknownFrame { at, kind } => write!(f, "unknown frame kind {kind} at {at}"),
            WireError::NoSchema { channel } => {
                write!(f, "no schema announced on channel {channel}")
            }
            WireError::NotASchema { channel } => write!(
                f,
                "channel {channel} announced something that is not a schema"
            ),
            WireError::Invalid { channel, error } => write!(f, "channel {channel}: {error}"),
        }
    }
}

impl std::error::Error for WireError {}

// --- encoding ---------------------------------------------------------------

/// What is left of a container being written.
enum Left<'a> {
    Items(std::slice::Iter<'a, Value>),
    Entries(std::slice::Iter<'a, Entry>),
}

/// Appends `value` to `out`.
///
/// Refuses only what a Rust-built tree cannot hold: a node a foreign
/// producer wrote that its own door refuses (text that is not UTF-8, a
/// number outside the grammar, an unknown tag). `out` then holds a prefix,
/// which the caller discards.
pub fn encode(value: &Value, out: &mut Vec<u8>) -> Result<(), ValueError> {
    let mut stack: Vec<Left<'_>> = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(node) = next.take()
            && let Some(opened) = node_into(node, out)?
        {
            stack.push(opened);
        }
        let Some(top) = stack.last_mut() else {
            return Ok(());
        };
        match top {
            Left::Items(items) => match items.next() {
                Some(item) => next = Some(item),
                None => {
                    stack.pop();
                }
            },
            Left::Entries(entries) => match entries.next() {
                Some(entry) => {
                    put_len(out, entry.key().len() as u64);
                    out.extend_from_slice(entry.key().as_bytes());
                    next = Some(entry.value());
                }
                None => {
                    stack.pop();
                }
            },
        }
    }
}

/// The bytes of `value`, as [`encode`] writes them.
pub fn to_bytes(value: &Value) -> Result<Vec<u8>, ValueError> {
    let mut out = Vec::new();
    encode(value, &mut out)?;
    Ok(out)
}

/// Writes one node's tag and body; a container answers what is left of it.
fn node_into<'a>(node: &'a Value, out: &mut Vec<u8>) -> Result<Option<Left<'a>>, ValueError> {
    let tag = node.tag()?;
    // Every tag this build knows fits a byte.
    out.push(u32::from(tag) as u8);
    match tag {
        Tag::GUATIAO_ABSENT | Tag::GUATIAO_NULL => {}
        Tag::GUATIAO_BOOL => {
            let b = TryAsRef::<bool>::try_as_ref(node).ok_or(ValueError::WrongKind)?;
            out.push(u8::from(*b));
        }
        Tag::GUATIAO_NUMBER => {
            let n = TryAsRef::<Number>::try_as_ref(node).ok_or(ValueError::NotANumber)?;
            put_len(out, n.len() as u64);
            out.extend_from_slice(n.as_bytes());
        }
        Tag::GUATIAO_STRING => {
            let t = TryAsRef::<Text>::try_as_ref(node).ok_or(ValueError::NotUtf8)?;
            put_len(out, t.len() as u64);
            out.extend_from_slice(t.as_bytes());
        }
        Tag::GUATIAO_BYTES => {
            let b = TryAsRef::<Buffer>::try_as_ref(node).ok_or(ValueError::WrongKind)?;
            put_len(out, b.len() as u64);
            out.extend_from_slice(b);
        }
        Tag::GUATIAO_LIST => {
            let l = TryAsRef::<List>::try_as_ref(node).ok_or(ValueError::WrongKind)?;
            put_len(out, l.len() as u64);
            return Ok(Some(Left::Items(l.iter())));
        }
        Tag::GUATIAO_MAP => {
            let m = TryAsRef::<Map>::try_as_ref(node).ok_or(ValueError::NotUtf8)?;
            put_len(out, m.len() as u64);
            return Ok(Some(Left::Entries(m.iter())));
        }
    }
    Ok(None)
}

/// An unsigned LEB128, the shortest form. `u64`, so a channel number
/// means the same on a 32-bit target.
fn put_len(out: &mut Vec<u8>, n: impl Into<u64>) {
    let mut n = n.into();
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

// --- decoding ---------------------------------------------------------------

/// The value `bytes` hold, built on Rust's heap.
pub fn decode(bytes: &[u8]) -> Result<Value, WireError> {
    decode_in(Alloc::rust(), bytes)
}

/// The same, built through `alloc`.
pub fn decode_in(alloc: Alloc, bytes: &[u8]) -> Result<Value, WireError> {
    decode_at(alloc, bytes, 0)
}

/// Decodes `bytes`, reporting offsets as if they began at `base` — what a
/// frame's payload does, so an offset names a byte of the whole frame.
pub(crate) fn decode_at(alloc: Alloc, bytes: &[u8], base: usize) -> Result<Value, WireError> {
    let mut r = Reader { bytes, at: 0, base };
    let mut stack: Vec<Frame<'_>> = Vec::new();
    loop {
        let (value, at) = match stack.last_mut() {
            // A container with nothing left is a finished value.
            Some(top) if top.left() == 0 => {
                let done = stack.pop().expect("the top was just read");
                (done.into_value(), r.offset())
            }
            top => {
                if let Some(Frame::Map { key, .. }) = top {
                    let at = r.offset();
                    *key = Some((r.text()?, at));
                }
                let at = r.offset();
                match r.node(alloc)? {
                    Node::Leaf(value) => (value, at),
                    Node::List(0) => (List::new_in(alloc).into(), at),
                    Node::Map(0) => (Map::new_in(alloc).into(), at),
                    Node::List(left) => {
                        stack.push(Frame::List {
                            list: List::new_in(alloc),
                            left,
                        });
                        continue;
                    }
                    Node::Map(left) => {
                        stack.push(Frame::Map {
                            map: Map::new_in(alloc),
                            left,
                            key: None,
                        });
                        continue;
                    }
                }
            }
        };
        match stack.last_mut() {
            None => {
                if r.at != r.bytes.len() {
                    return Err(WireError::Trailing { at: r.offset() });
                }
                return Ok(value);
            }
            Some(Frame::List { list, left }) => {
                list.push_in(value, alloc)
                    .map_err(|error| WireError::Alloc { at, error })?;
                *left -= 1;
            }
            Some(Frame::Map { map, left, key }) => {
                let (key, key_at) = key.take().expect("a map's value is read after its key");
                if map.contains_key(key) {
                    return Err(WireError::DuplicateKey { at: key_at });
                }
                map.set_in(key, value, alloc)
                    .map_err(|error| WireError::Alloc { at, error })?;
                *left -= 1;
            }
        }
    }
}

/// A container being built.
enum Frame<'a> {
    List {
        list: List,
        left: u64,
    },
    Map {
        map: Map,
        left: u64,
        key: Option<(&'a str, usize)>,
    },
}

impl Frame<'_> {
    fn left(&self) -> u64 {
        match self {
            Frame::List { left, .. } | Frame::Map { left, .. } => *left,
        }
    }

    fn into_value(self) -> Value {
        match self {
            Frame::List { list, .. } => list.into(),
            Frame::Map { map, .. } => map.into(),
        }
    }
}

/// One node read: a finished leaf, or a container with how many it holds.
enum Node {
    Leaf(Value),
    List(u64),
    Map(u64),
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    base: usize,
}

impl<'a> Reader<'a> {
    fn offset(&self) -> usize {
        self.base + self.at
    }

    fn byte(&mut self) -> Result<u8, WireError> {
        let b = *self
            .bytes
            .get(self.at)
            .ok_or(WireError::Truncated { at: self.offset() })?;
        self.at += 1;
        Ok(b)
    }

    /// An unsigned LEB128 of at most 64 bits, in its shortest form.
    fn leb(&mut self) -> Result<u64, WireError> {
        let at = self.offset();
        let mut n = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.byte()?;
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 || shift > 63 {
                return Err(WireError::Overlong { at });
            }
            n |= bits << shift;
            if byte & 0x80 == 0 {
                // A final zero byte after the first is a longer spelling of
                // a shorter number: refused, so every length has one form.
                if byte == 0 && shift > 0 {
                    return Err(WireError::Overlong { at });
                }
                return Ok(n);
            }
            shift += 7;
        }
    }

    /// A length, no more than the bytes that remain.
    fn len(&mut self) -> Result<usize, WireError> {
        let n = self.leb()?;
        let remaining = (self.bytes.len() - self.at) as u64;
        if n > remaining {
            return Err(WireError::Truncated { at: self.offset() });
        }
        Ok(n as usize)
    }

    /// A count of elements, each at least a byte, so no more than remain.
    fn count(&mut self) -> Result<u64, WireError> {
        let n = self.leb()?;
        if n > (self.bytes.len() - self.at) as u64 {
            return Err(WireError::Truncated { at: self.offset() });
        }
        Ok(n)
    }

    fn slice(&mut self) -> Result<&'a [u8], WireError> {
        let n = self.len()?;
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn text(&mut self) -> Result<&'a str, WireError> {
        let at = self.offset();
        std::str::from_utf8(self.slice()?).map_err(|_| WireError::NotUtf8 { at })
    }

    fn node(&mut self, alloc: Alloc) -> Result<Node, WireError> {
        let at = self.offset();
        let tag = self.byte()?;
        let built =
            |r: Result<Value, ValueError>| r.map_err(|error| WireError::Alloc { at, error });
        Ok(Node::Leaf(match Tag::try_from(u32::from(tag)) {
            Ok(Tag::GUATIAO_ABSENT) => Value::absent(),
            Ok(Tag::GUATIAO_NULL) => Value::null(),
            Ok(Tag::GUATIAO_BOOL) => {
                let at = self.offset();
                match self.byte()? {
                    0 => Value::from(false),
                    1 => Value::from(true),
                    byte => return Err(WireError::NotABool { at, byte }),
                }
            }
            Ok(Tag::GUATIAO_NUMBER) => {
                let body = self.offset();
                let text = self.text()?;
                match Number::new_in(alloc, text) {
                    Ok(n) => n.into(),
                    Err(ValueError::NotANumber) => return Err(WireError::NotANumber { at: body }),
                    Err(error) => return Err(WireError::Alloc { at, error }),
                }
            }
            Ok(Tag::GUATIAO_STRING) => {
                let text = self.text()?;
                built(Text::new_in(alloc, text).map(Value::from))?
            }
            Ok(Tag::GUATIAO_BYTES) => {
                let bytes = self.slice()?;
                built(Buffer::new_in(alloc, bytes).map(Value::from))?
            }
            Ok(Tag::GUATIAO_LIST) => return Ok(Node::List(self.count()?)),
            Ok(Tag::GUATIAO_MAP) => return Ok(Node::Map(self.count()?)),
            Err(_) => return Err(WireError::UnknownTag { at, tag }),
        }))
    }
}
