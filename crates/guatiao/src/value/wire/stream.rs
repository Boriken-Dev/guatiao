// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The wire encoding as events: [`Encoder`] writes any walk as bytes,
//! [`Decoder`] reads bytes into any [`FromWalk`] type, and [`Encoded`]
//! walks bytes into any visitor. None builds a tree, and each writes or
//! accepts exactly the bytes [`encode`](super::encode) and
//! [`decode`](super::decode) do.

#![forbid(unsafe_code)]

use std::collections::HashSet;

use super::{Reader, Token, WireError, put_len};
use crate::value::convert::MapError;
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::Tag;
use crate::value::types::number::validate_json_number;
use crate::value::visit::{
    Build, Flat, FromWalk, ListVisitor, ListWalker, MapVisitor, MapWalker, Seed, Sink, Visitor,
    Walk, Walker,
};

/// Past this many keys, a map's repeated-key check hashes rather than
/// comparing with every earlier key.
const LINEAR: usize = 16;

// --- writing -------------------------------------------------------------------

/// A container being written: where its count is, what was promised, how
/// many have been written, and its keys so far.
#[derive(Debug)]
struct Open {
    /// Where the count's bytes start.
    at: usize,
    /// `Some` when the count was written up front.
    promised: Option<u64>,
    written: u64,
    map: Option<Keys>,
}

/// A map's keys, as ranges of the output, and their hashes once there
/// are enough of them to need one.
#[derive(Debug, Default)]
struct Keys {
    ranges: Vec<(usize, usize)>,
    hashes: Option<HashSet<u64>>,
    keyed: bool,
}

/// Writes events as wire bytes, appending to a buffer.
///
/// A count is written before its elements: given up front when the walk
/// promises one, else one byte is reserved and the count put there,
/// moving the bytes after it when it needs more than one. A walk that
/// promised one count and gave another is refused with
/// [`ValueError::OutOfRange`]; a number outside the grammar with
/// [`ValueError::NotANumber`]; a key repeated in its map with
/// [`ValueError::DuplicateKey`]; events out of order with
/// [`ValueError::WrongKind`]. After a refusal the buffer holds a prefix,
/// which the caller discards.
#[derive(Debug)]
pub struct Encoder<'a> {
    out: &'a mut Vec<u8>,
    open: Vec<Open>,
    done: bool,
}

impl<'a> Encoder<'a> {
    /// Appends to `out`.
    pub fn new(out: &'a mut Vec<u8>) -> Encoder<'a> {
        Encoder {
            out,
            open: Vec::new(),
            done: false,
        }
    }

    /// Whether one whole value was written and every container closed.
    pub fn is_complete(&self) -> bool {
        self.done && self.open.is_empty()
    }

    /// Before a value: a key must have come first in a map, and only one
    /// value is written at the top.
    fn value_here(&mut self) -> Result<(), ValueError> {
        match self.open.last_mut() {
            None if self.done => Err(ValueError::WrongKind),
            None => Ok(()),
            Some(open) => {
                if let Some(keys) = &mut open.map {
                    if !keys.keyed {
                        return Err(ValueError::WrongKind);
                    }
                    keys.keyed = false;
                }
                open.written += 1;
                if open.promised.is_some_and(|n| open.written > n) {
                    return Err(ValueError::OutOfRange);
                }
                Ok(())
            }
        }
    }

    /// After a value: at the top, the walk is over.
    fn value_done(&mut self) {
        if self.open.is_empty() {
            self.done = true;
        }
    }

    fn leaf(&mut self, tag: Tag, body: Option<&[u8]>) -> Result<(), ValueError> {
        self.value_here()?;
        self.out.push(u32::from(tag) as u8);
        if let Some(body) = body {
            put_len(self.out, body.len() as u64);
            self.out.extend_from_slice(body);
        }
        self.value_done();
        Ok(())
    }

    fn begin(&mut self, tag: Tag, len: Option<usize>, map: bool) -> Result<(), ValueError> {
        self.value_here()?;
        self.out.push(u32::from(tag) as u8);
        let at = self.out.len();
        let promised = len.map(|n| n as u64);
        match promised {
            Some(n) => put_len(self.out, n),
            None => self.out.push(0),
        }
        self.open.push(Open {
            at,
            promised,
            written: 0,
            map: map.then(Keys::default),
        });
        Ok(())
    }

    fn end(&mut self, map: bool) -> Result<(), ValueError> {
        let fits = match self.open.last() {
            Some(open) => open.map.as_ref().map_or(!map, |keys| map && !keys.keyed),
            None => false,
        };
        if !fits {
            return Err(ValueError::WrongKind);
        }
        let open = self.open.pop().expect("checked above");
        match open.promised {
            Some(n) if n != open.written => return Err(ValueError::OutOfRange),
            Some(_) => {}
            None => {
                let mut count = Vec::new();
                put_len(&mut count, open.written);
                // One byte was reserved: a count that needs more moves
                // what follows it.
                if let [one] = count[..] {
                    self.out[open.at] = one;
                } else {
                    self.out.splice(open.at..open.at + 1, count).for_each(drop);
                }
            }
        }
        self.value_done();
        Ok(())
    }
}

/// A key's hash, for the repeated-key check of a large map.
fn hash_of(key: &[u8]) -> u64 {
    use core::hash::Hasher;
    // A fixed-key hasher: the set only speeds up a check that compares
    // bytes on every hit, so a collision costs time, never correctness.
    let mut h = std::hash::DefaultHasher::new();
    h.write(key);
    h.finish()
}

impl Sink for Encoder<'_> {
    type Error = ValueError;

    fn absent(&mut self) -> Result<(), ValueError> {
        self.leaf(Tag::GUATIAO_ABSENT, None)
    }

    fn null(&mut self) -> Result<(), ValueError> {
        self.leaf(Tag::GUATIAO_NULL, None)
    }

    fn bool(&mut self, value: bool) -> Result<(), ValueError> {
        self.value_here()?;
        self.out.push(u32::from(Tag::GUATIAO_BOOL) as u8);
        self.out.push(u8::from(value));
        self.value_done();
        Ok(())
    }

    fn number(&mut self, text: &str) -> Result<(), ValueError> {
        validate_json_number(text.as_bytes()).map_err(|_| ValueError::NotANumber)?;
        self.leaf(Tag::GUATIAO_NUMBER, Some(text.as_bytes()))
    }

    fn string(&mut self, text: &str) -> Result<(), ValueError> {
        self.leaf(Tag::GUATIAO_STRING, Some(text.as_bytes()))
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ValueError> {
        self.leaf(Tag::GUATIAO_BYTES, Some(bytes))
    }

    fn list_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        self.begin(Tag::GUATIAO_LIST, len, false)
    }

    fn list_end(&mut self) -> Result<(), ValueError> {
        self.end(false)
    }

    fn map_begin(&mut self, len: Option<usize>) -> Result<(), ValueError> {
        self.begin(Tag::GUATIAO_MAP, len, true)
    }

    fn key(&mut self, key: &str) -> Result<(), ValueError> {
        let Some(Open {
            map: Some(keys), ..
        }) = self.open.last_mut()
        else {
            return Err(ValueError::WrongKind);
        };
        if keys.keyed {
            return Err(ValueError::WrongKind);
        }
        let bytes = key.as_bytes();
        let out = &*self.out;
        let same = |&(at, len): &(usize, usize)| &out[at..at + len] == bytes;
        let repeated = match &mut keys.hashes {
            Some(hashes) => !hashes.insert(hash_of(bytes)) && keys.ranges.iter().any(same),
            None => keys.ranges.iter().any(same),
        };
        if repeated {
            return Err(ValueError::DuplicateKey);
        }
        put_len(self.out, bytes.len() as u64);
        let at = self.out.len();
        self.out.extend_from_slice(bytes);
        keys.ranges.push((at, bytes.len()));
        if keys.hashes.is_none() && keys.ranges.len() > LINEAR {
            let out = &*self.out;
            keys.hashes = Some(
                keys.ranges
                    .iter()
                    .map(|&(at, len)| hash_of(&out[at..at + len]))
                    .collect(),
            );
        }
        keys.keyed = true;
        Ok(())
    }

    fn map_end(&mut self) -> Result<(), ValueError> {
        self.end(true)
    }
}

/// Appends the wire bytes of anything that walks, with no tree built:
/// the same bytes [`encode`](super::encode) writes for the tree
/// [`ToValue`](crate::ToValue) would build. On a refusal `out` holds a
/// prefix, which the caller discards.
pub fn encode_of<T: Walk + ?Sized>(value: &T, out: &mut Vec<u8>) -> Result<(), ValueError> {
    let mut encoder = Encoder::new(out);
    value.walk(Flat(&mut encoder))?;
    if !encoder.is_complete() {
        return Err(ValueError::WrongKind);
    }
    Ok(())
}

/// The wire bytes of anything that walks, as [`encode_of`] writes them.
pub fn to_bytes_of<T: Walk + ?Sized>(value: &T) -> Result<Vec<u8>, ValueError> {
    let mut out = Vec::new();
    encode_of(value, &mut out)?;
    Ok(out)
}

// --- reading ----------------------------------------------------------------------

/// Reads wire bytes as a [`Walker`]: `&mut Decoder` hands the one value
/// they hold to a builder, which reads the containers it opens straight
/// from the bytes.
///
/// It refuses what [`decode`](super::decode) refuses, at the same byte
/// offsets, and a value nested deeper than [`MAX_DEPTH`] with
/// [`WireError::TooDeep`]. Its refusal is kept here and the builder is
/// told [`MapError::Stopped`]: [`decode_as`] reports it. Elements a
/// builder leaves unread are passed over, so the bytes after a container
/// are read where they are.
#[derive(Debug)]
pub struct Decoder<'b> {
    reader: Reader<'b>,
    depth: u32,
    /// The keys of every open map, innermost last; each map's start.
    keys: Vec<&'b str>,
    failure: Option<WireError>,
}

impl<'b> Decoder<'b> {
    /// Reads `bytes`, which hold one value.
    pub fn new(bytes: &'b [u8]) -> Decoder<'b> {
        Decoder {
            reader: Reader {
                bytes,
                at: 0,
                base: 0,
            },
            depth: 0,
            keys: Vec::new(),
            failure: None,
        }
    }

    /// What the bytes were refused with, once a read was told
    /// [`MapError::Stopped`].
    pub fn failure(&self) -> Option<&WireError> {
        self.failure.as_ref()
    }

    /// Finishes a read: the bytes' own refusal if there was one, else
    /// [`WireError::Trailing`] if bytes remain past the value.
    pub fn finish(mut self) -> Result<(), WireError> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        if self.reader.at != self.reader.bytes.len() {
            return Err(WireError::Trailing {
                at: self.reader.offset(),
            });
        }
        Ok(())
    }

    /// Keeps the bytes' refusal and tells the builder the read stopped.
    fn fail(&mut self, error: WireError) -> MapError {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        MapError::stopped()
    }

    /// Passes over one value with a stack of its own, so any depth costs
    /// heap, never the thread's stack. A skipped map's keys are checked as
    /// any map's.
    fn skip(&mut self) -> Result<(), MapError> {
        // Each open container: the values it still holds, and for a map
        // where its keys start, since a key comes before each value.
        let mut open: Vec<(u64, Option<usize>)> = vec![(1, None)];
        let base = self.keys.len();
        let skipped = loop {
            let Some(top) = open.last_mut() else {
                break Ok(());
            };
            if top.0 == 0 {
                if let Some((_, Some(start))) = open.pop() {
                    self.keys.truncate(start);
                }
                continue;
            }
            top.0 -= 1;
            if let Some(start) = top.1 {
                let at = self.reader.offset();
                let key = match self.reader.text() {
                    Ok(key) => key,
                    Err(e) => break Err(e),
                };
                if self.keys[start..].contains(&key) {
                    break Err(WireError::DuplicateKey { at });
                }
                self.keys.push(key);
            }
            match self.reader.token() {
                Ok(Token::List(n)) => open.push((n, None)),
                Ok(Token::Map(n)) => open.push((n, Some(self.keys.len()))),
                Ok(_) => {}
                Err(e) => break Err(e),
            }
        };
        self.keys.truncate(base);
        skipped.map_err(|e| self.fail(e))
    }
}

impl<'b> Walker for &mut Decoder<'b> {
    fn walk<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        if self.failure.is_some() {
            return Err(MapError::stopped());
        }
        let at = self.reader.offset();
        let token = match self.reader.token() {
            Ok(token) => token,
            Err(e) => return Err(self.fail(e)),
        };
        match token {
            Token::Absent => build.absent(),
            Token::Null => build.null(),
            Token::Bool(b) => build.bool(b),
            Token::Number(text) => build.number(text),
            Token::String(text) => build.string(text),
            Token::Bytes(bytes) => build.bytes(bytes),
            Token::List(count) | Token::Map(count) => {
                if self.depth >= MAX_DEPTH {
                    return Err(self.fail(WireError::TooDeep { at }));
                }
                self.depth += 1;
                let mut frame = Frame {
                    left: count,
                    pending: false,
                    start: self.keys.len(),
                };
                let built = if matches!(token, Token::Map(_)) {
                    build
                        .map(Entries {
                            decoder: &mut *self,
                            frame: &mut frame,
                        })
                        .and_then(|built| {
                            // What the builder left unread is passed
                            // over, so the next value is read where it
                            // starts.
                            let mut rest = Entries {
                                decoder: &mut *self,
                                frame: &mut frame,
                            };
                            while rest.next_key(|_| ())?.is_some() {
                                rest.skip_value()?;
                            }
                            Ok(built)
                        })
                } else {
                    build
                        .list(Items {
                            decoder: &mut *self,
                            frame: &mut frame,
                        })
                        .and_then(|built| {
                            for _ in 0..frame.left {
                                self.skip()?;
                            }
                            Ok(built)
                        })
                };
                self.keys.truncate(frame.start);
                self.depth -= 1;
                built
            }
        }
    }
}

/// An open container's progress, kept by the walk that opened it so it
/// outlives the builder's walker.
struct Frame {
    /// Elements or entries not yet reached.
    left: u64,
    /// A map's key was read and its value is due.
    pending: bool,
    /// Where a map's keys start in the decoder's list.
    start: usize,
}

struct Items<'d, 'b> {
    decoder: &'d mut Decoder<'b>,
    frame: &'d mut Frame,
}

impl ListWalker for Items<'_, '_> {
    fn len_hint(&self) -> Option<usize> {
        usize::try_from(self.frame.left).ok()
    }

    fn next_seed<S: Seed>(&mut self, seed: S) -> Result<Option<S::Out>, MapError> {
        if self.frame.left == 0 {
            return Ok(None);
        }
        self.frame.left -= 1;
        seed.read(&mut *self.decoder).map(Some)
    }
}

struct Entries<'d, 'b> {
    decoder: &'d mut Decoder<'b>,
    frame: &'d mut Frame,
}

impl MapWalker for Entries<'_, '_> {
    fn len_hint(&self) -> Option<usize> {
        usize::try_from(self.frame.left).ok()
    }

    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        if self.frame.pending {
            // The value under the last key was neither read nor skipped.
            self.skip_value()?;
        }
        if self.frame.left == 0 {
            return Ok(None);
        }
        self.frame.left -= 1;
        let d = &mut *self.decoder;
        let at = d.reader.offset();
        let key = match d.reader.text() {
            Ok(key) => key,
            Err(e) => return Err(d.fail(e)),
        };
        if d.keys[self.frame.start..].contains(&key) {
            return Err(d.fail(WireError::DuplicateKey { at }));
        }
        d.keys.push(key);
        self.frame.pending = true;
        Ok(Some(read(key)))
    }

    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError> {
        if !self.frame.pending {
            return Err(MapError::bad_value("a key read before its value"));
        }
        self.frame.pending = false;
        seed.read(&mut *self.decoder)
    }

    fn skip_value(&mut self) -> Result<(), MapError> {
        if !self.frame.pending {
            return Err(MapError::bad_value("a key read before its value"));
        }
        self.frame.pending = false;
        self.decoder.skip()
    }
}

/// Reads wire bytes into any [`FromWalk`] type with no tree built.
///
/// The bytes' own refusal is reported as [`decode`](super::decode)
/// reports it; a value the type refuses is [`WireError::Refused`], naming
/// the key; bytes past the value are [`WireError::Trailing`].
pub fn decode_as<T: FromWalk>(bytes: &[u8]) -> Result<T, WireError> {
    let mut decoder = Decoder::new(bytes);
    let read = T::from_walk(&mut decoder);
    if let Some(failure) = decoder.failure.take() {
        return Err(failure);
    }
    let value = read.map_err(|error| WireError::Refused { error })?;
    decoder.finish()?;
    Ok(value)
}

// --- bytes that walk themselves ------------------------------------------------------

/// Wire bytes as a [`Walk`]: walking them hands their value's events to
/// a visitor as they are read, checked as [`decode`](super::decode)
/// checks them, with no tree built.
///
/// A refusal of the bytes reaches the visitor's error as
/// [`ValueError::Unparsed`], at the byte offset `decode` names; a value
/// nested deeper than [`MAX_DEPTH`] as [`ValueError::TooDeep`]. Bytes
/// past the value are found only once the visitor has seen it: a
/// visitor that acts as it goes validates first, with `decode` or a
/// walk into a sink that keeps nothing.
#[derive(Debug, Clone, Copy)]
pub struct Encoded<'b>(pub &'b [u8]);

/// A wire refusal as a writing error, at its offset.
fn unparsed(error: WireError) -> ValueError {
    let (at, expected) = match error {
        WireError::Truncated { at } => (at, "a node that is not cut short"),
        WireError::UnknownTag { at, .. } => (at, "a tag this build knows"),
        WireError::Overlong { at } => (at, "a length in its shortest LEB128 form"),
        WireError::NotUtf8 { at } => (at, "text that is UTF-8"),
        WireError::NotANumber { at } => (at, "a number in the JSON number grammar"),
        WireError::NotABool { at, .. } => (at, "a bool byte of 0 or 1"),
        WireError::DuplicateKey { at } => (at, "a key not repeated in its map"),
        WireError::Trailing { at } => (at, "the end of the bytes"),
        WireError::TooDeep { .. } => return ValueError::TooDeep,
        _ => (0, "the wire encoding"),
    };
    ValueError::Unparsed { at, expected }
}

/// The node at the reader's position, with the keys of the maps above.
struct Node<'r, 'b> {
    reader: &'r core::cell::RefCell<Reader<'b>>,
    keys: &'r core::cell::RefCell<Vec<&'b str>>,
    depth: u32,
}

impl Walk for Node<'_, '_> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        let at = self.reader.borrow().offset();
        let token = self.reader.borrow_mut().token().map_err(unparsed)?;
        match token {
            Token::Absent => visitor.absent(),
            Token::Null => visitor.null(),
            Token::Bool(b) => visitor.bool(b),
            Token::Number(text) => visitor.number(text),
            Token::String(text) => visitor.string(text),
            Token::Bytes(bytes) => visitor.bytes(bytes),
            Token::List(count) | Token::Map(count) => {
                if self.depth >= MAX_DEPTH {
                    return Err(unparsed(WireError::TooDeep { at }).into());
                }
                let inside = Node {
                    reader: self.reader,
                    keys: self.keys,
                    depth: self.depth + 1,
                };
                let len = usize::try_from(count).ok();
                if matches!(token, Token::List(_)) {
                    let mut items = visitor.list(len)?;
                    for _ in 0..count {
                        items.item(&inside)?;
                    }
                    return items.end();
                }
                let start = self.keys.borrow().len();
                let mut entries = visitor.map(len)?;
                let walked = (|| -> Result<(), V::Error> {
                    for _ in 0..count {
                        let at = self.reader.borrow().offset();
                        let key = self.reader.borrow_mut().text().map_err(unparsed)?;
                        if self.keys.borrow()[start..].contains(&key) {
                            return Err(unparsed(WireError::DuplicateKey { at }).into());
                        }
                        self.keys.borrow_mut().push(key);
                        entries.entry(key, &inside)?;
                    }
                    Ok(())
                })();
                self.keys.borrow_mut().truncate(start);
                walked?;
                entries.end()
            }
        }
    }
}

impl Walk for Encoded<'_> {
    fn walk<V: Visitor>(&self, visitor: V) -> Result<V::Ok, V::Error> {
        let reader = core::cell::RefCell::new(Reader {
            bytes: self.0,
            at: 0,
            base: 0,
        });
        let keys = core::cell::RefCell::new(Vec::new());
        let walked = Node {
            reader: &reader,
            keys: &keys,
            depth: 0,
        }
        .walk(visitor)?;
        let reader = reader.borrow();
        if reader.at != reader.bytes.len() {
            return Err(unparsed(WireError::Trailing {
                at: reader.offset(),
            })
            .into());
        }
        Ok(walked)
    }
}
