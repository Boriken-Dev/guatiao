// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A bridge to a format guatiao has never heard of, written outside
//! `guatiao` with nothing but its public traits: a [`Sink`] that writes
//! it and a [`Walker`] that reads it. Any type that walks is written in
//! it, and any `FromWalk` type read from it, derived ones included.
//!
//! The format, one character per event:
//!
//! ```text
//! ~ absent   n null   t true   f false
//! #<text>;   a number          '<len>:<bytes>  a string
//! %<hex>;    bytes
//! [ ... ]    a list            { '<key> value ... }  a map
//! ```

extern crate alloc;

use alloc::collections::BTreeMap;

use guatiao::value::visit::{
    Build, Flat, FromWalk, ListWalker, MapWalker, Seed, Sink, TreeSink, Walk, Walker,
};
use guatiao::{FromValue, MapError, ToValue, Value, ValueError};

// --- writing ---------------------------------------------------------------------

/// Writes the format; containers need no counts, so no stack either.
#[derive(Default)]
struct Writer(String);

impl Writer {
    fn text(&mut self, text: &str) {
        self.0.push_str(&format!("'{}:{text}", text.len()));
    }
}

impl Sink for Writer {
    type Error = ValueError;

    fn absent(&mut self) -> Result<(), ValueError> {
        self.0.push('~');
        Ok(())
    }

    fn null(&mut self) -> Result<(), ValueError> {
        self.0.push('n');
        Ok(())
    }

    fn bool(&mut self, value: bool) -> Result<(), ValueError> {
        self.0.push(if value { 't' } else { 'f' });
        Ok(())
    }

    fn number(&mut self, text: &str) -> Result<(), ValueError> {
        if !guatiao::Number::is_number(text) {
            return Err(ValueError::NotANumber);
        }
        self.0.push('#');
        self.0.push_str(text);
        self.0.push(';');
        Ok(())
    }

    fn string(&mut self, text: &str) -> Result<(), ValueError> {
        self.text(text);
        Ok(())
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ValueError> {
        self.0.push('%');
        for b in bytes {
            self.0.push_str(&format!("{b:02x}"));
        }
        self.0.push(';');
        Ok(())
    }

    fn list_begin(&mut self, _: Option<usize>) -> Result<(), ValueError> {
        self.0.push('[');
        Ok(())
    }

    fn list_end(&mut self) -> Result<(), ValueError> {
        self.0.push(']');
        Ok(())
    }

    fn map_begin(&mut self, _: Option<usize>) -> Result<(), ValueError> {
        self.0.push('{');
        Ok(())
    }

    fn key(&mut self, key: &str) -> Result<(), ValueError> {
        self.text(key);
        Ok(())
    }

    fn map_end(&mut self) -> Result<(), ValueError> {
        self.0.push('}');
        Ok(())
    }
}

fn write<T: Walk + ?Sized>(value: &T) -> String {
    let mut writer = Writer::default();
    value.walk(Flat(&mut writer)).expect("writable");
    writer.0
}

// --- reading -------------------------------------------------------------------------

/// Reads the format: a cursor, walked by `&mut` itself.
struct Reader<'a> {
    text: &'a str,
    at: usize,
}

fn malformed(at: usize) -> MapError {
    MapError::bad_value(format!("the made-up format at {at}"))
}

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn take(&mut self) -> Result<u8, MapError> {
        let b = self.peek().ok_or_else(|| malformed(self.at))?;
        self.at += 1;
        Ok(b)
    }

    fn until(&mut self, end: char) -> Result<&'a str, MapError> {
        let rest = &self.text[self.at..];
        let n = rest.find(end).ok_or_else(|| malformed(self.at))?;
        self.at += n + 1;
        Ok(&rest[..n])
    }

    /// `'<len>:<text>`, its quote already taken.
    fn text(&mut self) -> Result<&'a str, MapError> {
        let len: usize = self.until(':')?.parse().map_err(|_| malformed(self.at))?;
        let text = self
            .text
            .get(self.at..self.at + len)
            .ok_or_else(|| malformed(self.at))?;
        self.at += len;
        Ok(text)
    }
}

impl Walker for &mut Reader<'_> {
    fn build<B: Build>(self, build: B) -> Result<B::Out, MapError> {
        match self.take()? {
            b'~' => build.absent(),
            b'n' => build.null(),
            b't' => build.bool(true),
            b'f' => build.bool(false),
            b'#' => {
                let text = self.until(';')?;
                build.number(text)
            }
            b'\'' => {
                let text = self.text()?;
                build.string(text)
            }
            b'%' => {
                let hex = self.until(';')?;
                let bytes: Result<Vec<u8>, _> = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                    .collect();
                build.bytes(&bytes.map_err(|_| malformed(self.at))?)
            }
            b'[' => {
                let out = build.list(Items(&mut *self))?;
                Ok(out)
            }
            b'{' => {
                let out = build.map(Entries(&mut *self))?;
                Ok(out)
            }
            _ => Err(malformed(self.at - 1)),
        }
    }
}

struct Items<'r, 'a>(&'r mut Reader<'a>);

impl ListWalker for Items<'_, '_> {
    fn len_hint(&self) -> Option<usize> {
        None
    }

    fn next_seed<S: Seed>(&mut self, seed: S) -> Result<Option<S::Out>, MapError> {
        if self.0.peek() == Some(b']') {
            self.0.at += 1;
            return Ok(None);
        }
        seed.read(&mut *self.0).map(Some)
    }
}

struct Entries<'r, 'a>(&'r mut Reader<'a>);

impl MapWalker for Entries<'_, '_> {
    fn len_hint(&self) -> Option<usize> {
        None
    }

    fn next_key<R>(&mut self, read: impl FnOnce(&str) -> R) -> Result<Option<R>, MapError> {
        match self.0.take()? {
            b'}' => Ok(None),
            b'\'' => Ok(Some(read(self.0.text()?))),
            _ => Err(malformed(self.0.at - 1)),
        }
    }

    fn value_seed<S: Seed>(&mut self, seed: S) -> Result<S::Out, MapError> {
        seed.read(&mut *self.0)
    }
}

fn read<T: FromWalk>(text: &str) -> Result<T, MapError> {
    let mut reader = Reader { text, at: 0 };
    let value = T::from_walk(&mut reader)?;
    if reader.at != text.len() {
        return Err(malformed(reader.at));
    }
    Ok(value)
}

// --- the round trips -------------------------------------------------------------------

#[derive(ToValue, FromValue, Debug, PartialEq)]
struct Endpoint {
    host: String,
    port: u16,
    secret: Option<guatiao::Bytes>,
    labels: BTreeMap<String, f64>,
    sizes: Vec<u32>,
}

fn endpoint() -> Endpoint {
    Endpoint {
        host: "h:1".into(),
        port: 5900,
        secret: Some(guatiao::Bytes(vec![0xde, 0xad])),
        labels: [("w".to_string(), 1.5)].into_iter().collect(),
        sizes: vec![1, 2],
    }
}

#[test]
fn a_derived_type_round_trips_through_a_format_core_never_heard_of() {
    let text = write(&endpoint());
    assert_eq!(
        text,
        "{'4:host'3:h:1'4:port#5900;'6:secret%dead;'6:labels{'1:w#1.5;}'5:sizes[#1;#2;]}"
    );
    assert_eq!(read::<Endpoint>(&text).unwrap(), endpoint());
}

#[test]
fn a_value_round_trips_and_a_refusal_names_its_key() {
    let tree = endpoint().to_value().unwrap();
    let text = write(&tree);
    assert_eq!(read::<Value>(&text).unwrap(), tree);

    // Read into a tree by any sink, the same tree.
    let mut sink = TreeSink::new();
    read::<Value>(&text).unwrap().walk(Flat(&mut sink)).unwrap();
    assert_eq!(sink.into_value().unwrap(), tree);

    let bad = text.replace("#5900;", "'2:no");
    assert_eq!(read::<Endpoint>(&bad).unwrap_err().key(), "port");
}
