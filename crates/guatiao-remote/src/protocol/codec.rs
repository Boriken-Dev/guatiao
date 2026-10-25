// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The pieces every frame is made of: LEB128 integers (zigzag for signed
//! ones), text and bytes behind their length, values in the wire encoding
//! behind theirs, fixed-width floats.

use guatiao::value::visit::Walk;
use guatiao::value::wire;
use guatiao::{Alloc, Value};

use super::ProtocolError;

/// Appends `n` as an unsigned LEB128.
pub(crate) fn put_uint(out: &mut Vec<u8>, mut n: u64) {
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

/// Appends `n` zigzagged, as an unsigned LEB128.
pub(crate) fn put_int(out: &mut Vec<u8>, n: i64) {
    put_uint(out, ((n << 1) ^ (n >> 63)) as u64);
}

/// Appends bytes behind their length.
pub(crate) fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_uint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

/// Appends text behind its length.
pub(crate) fn put_text(out: &mut Vec<u8>, text: &str) {
    put_bytes(out, text.as_bytes());
}

/// Appends a value in the wire encoding, behind its length.
pub(crate) fn put_value(out: &mut Vec<u8>, value: &Value) -> Result<(), ProtocolError> {
    let at = out.len();
    wire::encode(value, out).map_err(|e| ProtocolError::Value(e.to_string()))?;
    behind_length(out, at);
    Ok(())
}

/// Appends a map, a list or anything else that walks, in the wire
/// encoding, behind its length: no tree is built for it.
pub(crate) fn put_walk<T: Walk + ?Sized>(
    out: &mut Vec<u8>,
    value: &T,
) -> Result<(), ProtocolError> {
    let at = out.len();
    wire::encode_of(value, out).map_err(|e| ProtocolError::Value(e.to_string()))?;
    behind_length(out, at);
    Ok(())
}

/// Moves the bytes from `at` on behind their length, which is at most ten
/// bytes: an encoding's length is known only once it is written.
fn behind_length(out: &mut Vec<u8>, at: usize) {
    let len = out.len() - at;
    let mut head = Vec::with_capacity(10);
    put_uint(&mut head, len as u64);
    out.splice(at..at, head);
}

/// Reads a frame's fields in order, refusing what is short or malformed.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    /// Where the next field starts.
    pub(crate) fn at(&self) -> usize {
        self.at
    }

    /// What is left, unread.
    pub(crate) fn rest(&self) -> &'a [u8] {
        &self.bytes[self.at..]
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.at == self.bytes.len()
    }

    /// Refuses anything left after the last field.
    pub(crate) fn end(&self) -> Result<(), ProtocolError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(ProtocolError::Trailing { at: self.at })
        }
    }

    pub(crate) fn byte(&mut self) -> Result<u8, ProtocolError> {
        let b = *self
            .bytes
            .get(self.at)
            .ok_or(ProtocolError::Truncated { at: self.at })?;
        self.at += 1;
        Ok(b)
    }

    pub(crate) fn fixed<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        let end = self
            .at
            .checked_add(N)
            .filter(|&e| e <= self.bytes.len())
            .ok_or(ProtocolError::Truncated { at: self.at })?;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.bytes[self.at..end]);
        self.at = end;
        Ok(out)
    }

    /// An unsigned LEB128, refused when longer than its shortest form or
    /// wider than 64 bits.
    pub(crate) fn uint(&mut self) -> Result<u64, ProtocolError> {
        let start = self.at;
        let mut n: u64 = 0;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 {
                return Err(ProtocolError::Overlong { at: start });
            }
            n |= bits << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift > 0 {
                    return Err(ProtocolError::Overlong { at: start });
                }
                return Ok(n);
            }
        }
        Err(ProtocolError::Overlong { at: start })
    }

    pub(crate) fn int(&mut self) -> Result<i64, ProtocolError> {
        let z = self.uint()?;
        Ok(((z >> 1) as i64) ^ -((z & 1) as i64))
    }

    /// A length that must fit what is left.
    pub(crate) fn len(&mut self) -> Result<usize, ProtocolError> {
        let start = self.at;
        let n = self.uint()?;
        usize::try_from(n)
            .ok()
            .filter(|&n| n <= self.bytes.len() - self.at)
            .ok_or(ProtocolError::Truncated { at: start })
    }

    pub(crate) fn bytes(&mut self) -> Result<&'a [u8], ProtocolError> {
        let n = self.len()?;
        let out = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }

    pub(crate) fn text(&mut self) -> Result<&'a str, ProtocolError> {
        let start = self.at;
        core::str::from_utf8(self.bytes()?).map_err(|_| ProtocolError::NotUtf8 { at: start })
    }

    /// A value's wire bytes, unread.
    pub(crate) fn value_bytes(&mut self) -> Result<&'a [u8], ProtocolError> {
        self.bytes()
    }

    /// A value, decoded into `alloc`.
    pub(crate) fn value(&mut self, alloc: Alloc) -> Result<Value, ProtocolError> {
        let start = self.at;
        let bytes = self.value_bytes()?;
        wire::decode_in(alloc, bytes).map_err(|e| ProtocolError::Wire {
            at: start,
            why: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_round_trip_in_their_shortest_form() {
        for n in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            let mut out = Vec::new();
            put_uint(&mut out, n);
            let mut r = Reader::new(&out);
            assert_eq!(r.uint().unwrap(), n);
            r.end().unwrap();
        }
        for n in [0i64, -1, 1, -64, 64, i64::MIN, i64::MAX] {
            let mut out = Vec::new();
            put_int(&mut out, n);
            assert_eq!(Reader::new(&out).int().unwrap(), n);
        }
        let mut out = Vec::new();
        put_int(&mut out, -1);
        assert_eq!(out, [1]);
    }

    #[test]
    fn an_overlong_or_truncated_integer_is_refused() {
        assert_eq!(
            Reader::new(&[0x80, 0x00]).uint(),
            Err(ProtocolError::Overlong { at: 0 })
        );
        assert_eq!(
            Reader::new(&[0x80]).uint(),
            Err(ProtocolError::Truncated { at: 1 })
        );
        let eleven = [0xff; 10];
        assert!(Reader::new(&eleven).uint().is_err());
        let mut max = Vec::new();
        put_uint(&mut max, u64::MAX);
        assert_eq!(max.len(), 10);
        let mut over = max.clone();
        over[9] = 0x02;
        assert_eq!(
            Reader::new(&over).uint(),
            Err(ProtocolError::Overlong { at: 0 })
        );
    }

    #[test]
    fn a_length_past_the_end_is_refused_before_anything_is_read() {
        assert_eq!(
            Reader::new(&[5, b'a']).bytes(),
            Err(ProtocolError::Truncated { at: 0 })
        );
        assert_eq!(
            Reader::new(&[2, 0xff, 0xfe]).text(),
            Err(ProtocolError::NotUtf8 { at: 0 })
        );
    }
}
