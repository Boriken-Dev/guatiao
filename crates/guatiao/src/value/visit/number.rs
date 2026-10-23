// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A number's text without a heap allocation in the common case.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt::{self, Write};

/// What fits on the stack: every integer up to 128 bits, and most
/// floats. An `f64`'s `Display` never uses an exponent, so `1e300` is 301
/// digits and goes to the heap.
const ROOM: usize = 48;

/// Number text: on the stack when it fits, else on the heap.
pub(crate) struct Text {
    buf: [u8; ROOM],
    len: usize,
    heap: Option<String>,
}

impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(heap) = &mut self.heap {
            heap.push_str(s);
            return Ok(());
        }
        let end = self.len + s.len();
        if end <= ROOM {
            self.buf[self.len..end].copy_from_slice(s.as_bytes());
            self.len = end;
        } else {
            let mut heap = String::with_capacity(end);
            heap.push_str(self.as_str());
            heap.push_str(s);
            self.heap = Some(heap);
        }
        Ok(())
    }
}

impl Text {
    fn of(args: fmt::Arguments<'_>) -> Text {
        let mut text = Text {
            buf: [0; ROOM],
            len: 0,
            heap: None,
        };
        // Writing into this never fails: overflow goes to the heap.
        let _ = text.write_fmt(args);
        text
    }

    pub(crate) fn int(v: i64) -> Text {
        Text::of(format_args!("{v}"))
    }

    pub(crate) fn uint(v: u64) -> Text {
        Text::of(format_args!("{v}"))
    }

    pub(crate) fn wide(v: i128) -> Text {
        Text::of(format_args!("{v}"))
    }

    pub(crate) fn wide_uint(v: u128) -> Text {
        Text::of(format_args!("{v}"))
    }

    /// `Display`, which is what `ToValue for f64` stores; `None` for a
    /// float with no JSON spelling.
    pub(crate) fn float(v: f64) -> Option<Text> {
        v.is_finite().then(|| Text::of(format_args!("{v}")))
    }

    /// The same for an `f32`, from its own shortest text.
    pub(crate) fn float32(v: f32) -> Option<Text> {
        v.is_finite().then(|| Text::of(format_args!("{v}")))
    }

    /// `Debug`: the shortest text that reads back the same float, keeping
    /// a fraction, which is how a value read from a format holds a float.
    pub(crate) fn shortest(v: f64) -> Option<Text> {
        v.is_finite().then(|| Text::of(format_args!("{v:?}")))
    }

    pub(crate) fn as_str(&self) -> &str {
        match &self.heap {
            Some(heap) => heap,
            // Only whole `&str`s are ever copied in.
            None => core::str::from_utf8(&self.buf[..self.len]).unwrap_or(""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Text;

    #[test]
    fn integers_floats_and_the_heap() {
        assert_eq!(Text::int(-5900).as_str(), "-5900");
        assert_eq!(Text::wide(i128::MIN).as_str(), i128::MIN.to_string());
        assert_eq!(Text::wide_uint(u128::MAX).as_str(), u128::MAX.to_string());
        assert_eq!(
            Text::float(2.0).map(|t| t.as_str().to_string()).as_deref(),
            Some("2")
        );
        assert_eq!(
            Text::shortest(2.0)
                .map(|t| t.as_str().to_string())
                .as_deref(),
            Some("2.0")
        );
        let big = Text::float(1e300).expect("finite");
        assert_eq!(big.as_str(), 1e300f64.to_string());
        assert!(Text::float(f64::NAN).is_none());
        assert!(Text::float32(f32::INFINITY).is_none());
    }
}
