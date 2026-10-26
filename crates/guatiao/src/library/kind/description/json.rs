// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! JSON text (RFC 8259) read into a value: how a provider's kind
//! descriptions arrive. Only what a description needs from a reader, and
//! no more: the document is read once, by a host, and checked after.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::alloc::Alloc;
use crate::value::types::Value;
use crate::value::visit::{Sink, TreeSink};

/// Where the text stopped being JSON, as a byte offset, and why.
pub(crate) type JsonError = (usize, &'static str);

/// The value `text` holds, built in `alloc`. Nesting deeper than a value
/// holds is refused by the builder; a repeated key replaces the first.
pub(crate) fn read(text: &str, alloc: Alloc) -> Result<Value, JsonError> {
    let mut reader = Reader {
        text: text.as_bytes(),
        at: 0,
        sink: TreeSink::new_in(alloc),
    };
    reader.document()?;
    let at = reader.at;
    reader
        .sink
        .into_value()
        .map_err(|_| (at, "an incomplete value"))
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
    sink: TreeSink,
}

/// An open container: a map wants a key next, or each wants a value.
#[derive(Clone, Copy, PartialEq)]
enum Open {
    Map,
    List,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn fail<T>(&self, why: &'static str) -> Result<T, JsonError> {
        Err((self.at, why))
    }

    /// Hands what the sink refused back at the current offset.
    fn sunk(&self, built: Result<(), crate::ValueError>) -> Result<(), JsonError> {
        built.map_err(|_| (self.at, "a value the builder refuses"))
    }

    fn document(&mut self) -> Result<(), JsonError> {
        let mut open: Vec<Open> = Vec::new();
        loop {
            // A value is due.
            self.space();
            match self.peek() {
                Some(b'{') => {
                    self.at += 1;
                    let begun = self.sink.map_begin(None);
                    self.sunk(begun)?;
                    self.space();
                    if self.peek() == Some(b'}') {
                        self.at += 1;
                        let ended = self.sink.map_end();
                        self.sunk(ended)?;
                    } else {
                        open.push(Open::Map);
                        self.key()?;
                        continue;
                    }
                }
                Some(b'[') => {
                    self.at += 1;
                    let begun = self.sink.list_begin(None);
                    self.sunk(begun)?;
                    self.space();
                    if self.peek() == Some(b']') {
                        self.at += 1;
                        let ended = self.sink.list_end();
                        self.sunk(ended)?;
                    } else {
                        open.push(Open::List);
                        continue;
                    }
                }
                _ => self.scalar()?,
            }
            // A value was read: close what it ends, or go on to the next.
            loop {
                self.space();
                let Some(&top) = open.last() else {
                    return match self.peek() {
                        None => Ok(()),
                        Some(_) => self.fail("text after the value"),
                    };
                };
                match (top, self.peek()) {
                    (_, Some(b',')) => {
                        self.at += 1;
                        if top == Open::Map {
                            self.key()?;
                        }
                        break;
                    }
                    (Open::Map, Some(b'}')) => {
                        self.at += 1;
                        open.pop();
                        let ended = self.sink.map_end();
                        self.sunk(ended)?;
                    }
                    (Open::List, Some(b']')) => {
                        self.at += 1;
                        open.pop();
                        let ended = self.sink.list_end();
                        self.sunk(ended)?;
                    }
                    _ => return self.fail("a comma or the end of the container"),
                }
            }
        }
    }

    /// `"key"` and its colon.
    fn key(&mut self) -> Result<(), JsonError> {
        self.space();
        if self.peek() != Some(b'"') {
            return self.fail("a key in quotes");
        }
        let key = self.string()?;
        let keyed = self.sink.key(&key);
        self.sunk(keyed)?;
        self.space();
        if self.peek() != Some(b':') {
            return self.fail("a colon after the key");
        }
        self.at += 1;
        Ok(())
    }

    fn scalar(&mut self) -> Result<(), JsonError> {
        let built = match self.peek() {
            Some(b'"') => {
                let text = self.string()?;
                self.sink.string(&text)
            }
            Some(b't') => {
                self.word("true")?;
                self.sink.bool(true)
            }
            Some(b'f') => {
                self.word("false")?;
                self.sink.bool(false)
            }
            Some(b'n') => {
                self.word("null")?;
                self.sink.null()
            }
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while matches!(
                    self.peek(),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.at += 1;
                }
                // The builder holds the number grammar and refuses the rest.
                let text = core::str::from_utf8(&self.text[start..self.at]).unwrap_or("");
                return self.sink.number(text).map_err(|_| (start, "a number"));
            }
            _ => return self.fail("a value"),
        };
        self.sunk(built)
    }

    fn word(&mut self, word: &'static str) -> Result<(), JsonError> {
        if self.text[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(())
        } else {
            self.fail("true, false or null")
        }
    }

    /// A string at the opening quote, unescaped.
    fn string(&mut self) -> Result<String, JsonError> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while !matches!(self.peek(), None | Some(b'"' | b'\\' | 0..=0x1f)) {
                self.at += 1;
            }
            // The text is a `str`, and every stop is an ASCII byte, so the
            // run between is whole characters.
            out.push_str(core::str::from_utf8(&self.text[start..self.at]).unwrap_or(""));
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escaped = match self.peek() {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.at += 1;
                            out.push(self.unicode()?);
                            continue;
                        }
                        _ => return self.fail("an escape JSON defines"),
                    };
                    self.at += 1;
                    out.push(escaped);
                }
                None => return self.fail("the closing quote"),
                Some(_) => return self.fail("a control character escaped"),
            }
        }
    }

    /// The character a `\u` escape names, past the `u`: a pair of escapes
    /// for one outside the Basic Multilingual Plane.
    fn unicode(&mut self) -> Result<char, JsonError> {
        let high = self.hex4()?;
        let code = if (0xd800..0xdc00).contains(&high) {
            if !self.text[self.at..].starts_with(b"\\u") {
                return self.fail("the second half of a surrogate pair");
            }
            self.at += 2;
            let low = self.hex4()?;
            if !(0xdc00..0xe000).contains(&low) {
                return self.fail("the second half of a surrogate pair");
            }
            0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
        } else {
            high
        };
        match char::from_u32(code) {
            Some(c) => Ok(c),
            None => self.fail("a character, not half of a surrogate pair"),
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.text.get(self.at..self.at + 4);
        let code = digits
            .and_then(|d| core::str::from_utf8(d).ok())
            .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|d| u32::from_str_radix(d, 16).ok());
        match code {
            Some(code) => {
                self.at += 4;
                Ok(code)
            }
            None => self.fail("four hexadecimal digits"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::types::Number;

    fn read_rust(text: &str) -> Result<Value, JsonError> {
        read(text, Alloc::rust())
    }

    #[test]
    fn every_kind_of_value_is_read() {
        let value = read_rust(
            r#" {"a": [1, -2.5e3, true, false, null, {}, []], "s": "q\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00"} "#,
        )
        .unwrap();
        let map: &crate::Map = crate::TryAsRef::try_as_ref(&value).unwrap();
        let list: &crate::List = crate::TryAsRef::try_as_ref(&map["a"]).unwrap();
        assert_eq!(list.len(), 7);
        assert_eq!(list[1], Value::from(Number::new("-2.5e3").unwrap()));
        assert_eq!(list[4], Value::null());
        assert_eq!(
            TryAsRef::<str>::try_as_ref(&map["s"]),
            Some("q\"\\/\u{8}\u{c}\n\r\t\u{e9}\u{1f600}")
        );
    }

    use crate::TryAsRef;

    #[test]
    fn what_is_not_json_is_refused_where_it_stops() {
        for (text, at) in [
            ("", 0),
            ("{", 1),
            ("{\"a\" 1}", 5),
            ("{\"a\":1,}", 7),
            ("[1 2]", 3),
            ("[1,]", 3),
            ("tru", 0),
            ("01", 0),
            ("\"\\x\"", 2),
            ("\"\\ud800\"", 7),
            ("\"a\nb\"", 2),
            ("{} {}", 3),
            ("{1:2}", 1),
        ] {
            assert_eq!(read_rust(text).map(|_| ()).unwrap_err().0, at, "{text:?}");
        }
    }

    #[test]
    fn nesting_past_what_a_value_holds_is_refused() {
        let deep = "[".repeat(crate::MAX_DEPTH as usize + 1);
        assert!(read_rust(&deep).is_err());
    }
}
