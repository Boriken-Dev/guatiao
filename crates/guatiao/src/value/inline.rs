// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A value as one line of JSON-form text, for a report or a log.
//!
//! [`Inline`] writes what `guatiao-serde`'s JSON text
//! (`guatiao_serde::text::json`) writes for the same tree, bytes as a
//! `data:;base64,` string and numbers as their own text: no whitespace,
//! `[1,"a"]` and `{"k":null}`, a map in the order its keys were set. It
//! needs no format crate, which is why a crate that only shows a value
//! does not take one. Like that text it escapes only `"`, `\\` and the
//! characters below U+0020: U+007F, U+0085 (and the other C1 controls),
//! U+2028 and U+2029 are written as they are.
//!
//! **Display only.** The text is written and never parsed: nothing here
//! reads it back, and it is not a wire format (the wire encoding is in
//! [`wire`](crate::value::wire)). Four things it shows that JSON cannot
//! carry, all as bare placeholders, never quoted, so the line is valid
//! JSON exactly when none appears:
//!
//! - a node in [`Alloc::wiping`](crate::Alloc::wiping), and a map key
//!   there, print as `[sensitive]`, a container without its contents; a
//!   sensitive key leaves its value visible, since the key is the secret;
//! - an absent node prints `absent`, and a node of an unknown tag
//!   `<unknown tag N>`: neither faults;
//! - text, a number or a map key that is not UTF-8 prints as
//!   `<not utf-8>` (a bad key alone, its entry's value still shown);
//! - a number whose text is outside JSON's grammar (`NaN`, `+1`, `1e`)
//!   prints as `<not a number>`;
//! - a node nested [`MAX_DEPTH`] levels below the one printed prints
//!   `...`, so a hostile tree costs a bounded line.

#![forbid(unsafe_code)]

use core::fmt::{self, Write as _};

use super::convert::TryAsRef;
use super::error::MAX_DEPTH;
use super::types::{List, Map, Number, Tag, Value};

/// A value as one line of JSON-form text, through [`Display`](fmt::Display).
///
/// ```
/// use guatiao::{List, Map, Value};
/// use guatiao::value::Inline;
///
/// let mut list = List::new();
/// list.push(1);
/// list.push("a\"b");
/// let mut map = Map::new();
/// map.set("items", list);
/// map.set("none", Value::null());
/// assert_eq!(
///     Inline(&Value::from(map)).to_string(),
///     r#"{"items":[1,"a\"b"],"none":null}"#,
/// );
/// ```
///
/// See the module for what is shown in place of what JSON cannot say.
pub struct Inline<'a>(pub &'a Value);

/// What is printed in place of a node or key in the wiping allocator.
const SENSITIVE: &str = "[sensitive]";

/// What is printed in place of text that is not UTF-8.
const NOT_UTF8: &str = "<not utf-8>";

/// What is printed in place of a number outside JSON's grammar.
const NOT_A_NUMBER: &str = "<not a number>";

impl fmt::Display for Inline<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_value(f, self.0, 0)
    }
}

impl fmt::Debug for Inline<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

fn write_value(f: &mut fmt::Formatter<'_>, v: &Value, depth: u32) -> fmt::Result {
    if depth >= MAX_DEPTH {
        return f.write_str("...");
    }
    if v.alloc().is_ok_and(|a| a.is_wiping()) {
        return f.write_str(SENSITIVE);
    }
    match v.tag() {
        Err(_) => write!(f, "<unknown tag {}>", v.tag),
        Ok(Tag::GUATIAO_ABSENT) => f.write_str("absent"),
        Ok(Tag::GUATIAO_NULL) => f.write_str("null"),
        Ok(Tag::GUATIAO_BOOL) => f.write_str(if bool::try_from(v).unwrap_or(false) {
            "true"
        } else {
            "false"
        }),
        Ok(Tag::GUATIAO_NUMBER) => {
            match v.text_bytes(Tag::GUATIAO_NUMBER).map(core::str::from_utf8) {
                Some(Ok(text)) if Number::is_number(text) => f.write_str(text),
                Some(Ok(_)) => f.write_str(NOT_A_NUMBER),
                _ => f.write_str(NOT_UTF8),
            }
        }
        Ok(Tag::GUATIAO_STRING) => match TryAsRef::<str>::try_as_ref(v) {
            Some(s) => write_quoted(f, s),
            None => f.write_str(NOT_UTF8),
        },
        Ok(Tag::GUATIAO_BYTES) => {
            f.write_str("\"data:;base64,")?;
            write_base64(f, TryAsRef::<[u8]>::try_as_ref(v).unwrap_or(&[]))?;
            f.write_char('"')
        }
        Ok(Tag::GUATIAO_LIST) => {
            let empty = List::new();
            f.write_char('[')?;
            for (i, item) in TryAsRef::<List>::try_as_ref(v)
                .unwrap_or(&empty)
                .iter()
                .enumerate()
            {
                if i > 0 {
                    f.write_char(',')?;
                }
                write_value(f, item, depth + 1)?;
            }
            f.write_char(']')
        }
        Ok(Tag::GUATIAO_MAP) => {
            let empty = Map::new();
            f.write_char('{')?;
            // The keys are unchecked here: one that is not UTF-8 is shown
            // as a placeholder, not allowed to hide the whole map.
            for (i, entry) in v.map_arm().unwrap_or(&empty).iter().enumerate() {
                if i > 0 {
                    f.write_char(',')?;
                }
                if entry.key.alloc().is_ok_and(|a| a.is_wiping()) {
                    f.write_str(SENSITIVE)?;
                } else {
                    match core::str::from_utf8(entry.key_bytes()) {
                        Ok(key) => write_quoted(f, key)?,
                        Err(_) => f.write_str(NOT_UTF8)?,
                    }
                }
                f.write_char(':')?;
                write_value(f, entry.value(), depth + 1)?;
            }
            f.write_char('}')
        }
    }
}

/// `s` in double quotes with JSON's escapes: `\"`, `\\`, `\b`, `\f`, `\n`,
/// `\r`, `\t`, any other control character below U+0020 as `\u00xx`, and
/// everything else as it is.
fn write_quoted(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_char('"')?;
    let mut plain = 0;
    for (at, c) in s.char_indices() {
        let escape = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if c < '\u{20}' => {
                f.write_str(&s[plain..at])?;
                plain = at + 1;
                write!(f, "\\u{:04x}", c as u32)?;
                continue;
            }
            _ => continue,
        };
        f.write_str(&s[plain..at])?;
        plain = at + 1;
        f.write_str(escape)?;
    }
    f.write_str(&s[plain..])?;
    f.write_char('"')
}

/// Standard base64 (RFC 4648 section 4), padded.
fn write_base64(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                f.write_char(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]))?;
            } else {
                f.write_char('=')?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    fn text(v: Value) -> alloc::string::String {
        Inline(&v).to_string()
    }

    #[test]
    fn the_scalars() {
        assert_eq!(text(Value::null()), "null");
        assert_eq!(text(Value::absent()), "absent");
        assert_eq!(text(Value::from(true)), "true");
        assert_eq!(text(Value::from(false)), "false");
        assert_eq!(text(Value::from(Number::new("1.10").unwrap())), "1.10");
    }

    #[test]
    fn escapes_are_jsons() {
        assert_eq!(
            text(Value::from("a\"\\\u{8}\u{c}\n\r\t\u{1}\u{1f}\u{7f}é/")),
            "\"a\\\"\\\\\\b\\f\\n\\r\\t\\u0001\\u001f\u{7f}é/\""
        );
    }

    #[test]
    fn bytes_are_a_data_uri_at_every_padding() {
        use crate::Buffer;
        for (raw, b64) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (&[0xfb, 0xff, 0xfe], "+//+"),
        ] {
            assert_eq!(
                text(Value::from(Buffer::new(raw))),
                alloc::format!("\"data:;base64,{b64}\"")
            );
        }
    }

    #[test]
    fn a_tree_of_maps_past_the_bound_prints_dots() {
        let mut v = Value::null();
        for _ in 0..MAX_DEPTH + 2 {
            let mut map = Map::new();
            map.set("k", v);
            v = map.into();
        }
        let shown = text(v);
        assert!(shown.contains("{\"k\":...}"), "{shown}");
        assert_eq!(shown.matches('{').count(), MAX_DEPTH as usize);
    }

    #[test]
    fn a_tree_past_the_bound_prints_dots() {
        let mut v = Value::null();
        for _ in 0..MAX_DEPTH + 2 {
            let mut list = List::new();
            list.push(v);
            v = list.into();
        }
        let shown = text(v);
        assert!(shown.contains("[...]"), "{shown}");
        assert_eq!(shown.matches('[').count(), MAX_DEPTH as usize);
    }
}
