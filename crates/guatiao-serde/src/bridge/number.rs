// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A number's text on the stack, for what serde hands over as a binary
//! number.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt::{self, Write};

/// Every integer up to 128 bits and a float's shortest text fit.
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
    pub(crate) fn of(args: fmt::Arguments<'_>) -> Text {
        let mut text = Text {
            buf: [0; ROOM],
            len: 0,
            heap: None,
        };
        // Overflow goes to the heap, so writing cannot fail.
        let _ = text.write_fmt(args);
        text
    }

    pub(crate) fn as_str(&self) -> &str {
        match &self.heap {
            Some(heap) => heap,
            // Only whole `&str`s are copied in.
            None => core::str::from_utf8(&self.buf[..self.len]).unwrap_or(""),
        }
    }
}
