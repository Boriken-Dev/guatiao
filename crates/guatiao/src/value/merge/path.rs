// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A place inside a merged value, as a list of segments, and the text
//! that spells one.
//!
//! The merge keys everything by segments (`["tls", "ca"]`), never by
//! joined text, because a key may hold a `.`: the top-level key `tls.ca`
//! and the key `ca` inside `tls` are different leaves and must stay so.
//! The text spelling exists for the two places a path crosses as text -- an
//! error message and the C override table -- and is `guatiao-intake`'s path
//! grammar (`tls.ca`, `peers[0].name`, `["a.b"]`), so a path printed here
//! parses there.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

/// A segment that reads as a list position or a number-like key:
/// ASCII digits only.
fn is_digits(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
}

/// A segment that cannot be written bare after a `.`.
fn needs_quotes(segment: &str) -> bool {
    segment.is_empty() || segment.contains(['.', '[', ']'])
}

/// `segments` as one line of text: `tls.ca`, `peers[0].name`,
/// `["a.b"].c`. The root is the empty string.
pub(crate) fn render(segments: &[String]) -> String {
    let mut out = String::new();
    for (at, segment) in segments.iter().enumerate() {
        if is_digits(segment) {
            out.push('[');
            out.push_str(segment);
            out.push(']');
        } else if needs_quotes(segment) {
            out.push_str("[\"");
            for c in segment.chars() {
                if c == '"' || c == '\\' {
                    out.push('\\');
                }
                out.push(c);
            }
            out.push_str("\"]");
        } else {
            if at > 0 {
                out.push('.');
            }
            out.push_str(segment);
        }
    }
    out
}

/// The segments a spelling names, or `None` when it is not one. The empty
/// string is the root, which has none.
pub(crate) fn parse(text: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '[' => {
                at += 1;
                let mut segment = String::new();
                if chars.get(at) == Some(&'"') {
                    at += 1;
                    loop {
                        match chars.get(at)? {
                            '"' => break,
                            '\\' => {
                                at += 1;
                                segment.push(*chars.get(at)?);
                            }
                            c => segment.push(*c),
                        }
                        at += 1;
                    }
                    at += 1;
                } else {
                    while let Some(c) = chars.get(at) {
                        match c {
                            ']' => break,
                            '[' => return None,
                            c => segment.push(*c),
                        }
                        at += 1;
                    }
                    if segment.is_empty() {
                        return None;
                    }
                }
                if chars.get(at) != Some(&']') {
                    return None;
                }
                at += 1;
                out.push(segment);
            }
            '.' if at > 0 => {
                at += 1;
                out.push(name(&chars, &mut at)?);
            }
            // A bare name opens a path and nothing else: after a segment
            // comes a `.`, a `[` or the end.
            _ if at == 0 => out.push(name(&chars, &mut at)?),
            _ => return None,
        }
    }
    Some(out)
}

/// A bare name: one or more characters, none of `.`, `[`, `]`.
fn name(chars: &[char], at: &mut usize) -> Option<String> {
    let start = *at;
    while let Some(c) = chars.get(*at) {
        if matches!(c, '.' | '[' | ']') {
            break;
        }
        *at += 1;
    }
    (*at > start).then(|| chars[start..*at].iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_path_prints_the_way_the_intake_grammar_reads_it() {
        assert_eq!(render(&segs(&[])), "");
        assert_eq!(render(&segs(&["tls", "ca"])), "tls.ca");
        assert_eq!(render(&segs(&["peers", "0", "name"])), "peers[0].name");
        assert_eq!(render(&segs(&["tls.ca"])), "[\"tls.ca\"]");
        assert_eq!(render(&segs(&["a", "b.c", "d"])), "a[\"b.c\"].d");
        assert_eq!(render(&segs(&["say \"hi\""])), "say \"hi\"");
        assert_eq!(render(&segs(&["a]"])), "[\"a]\"]");
    }

    #[test]
    fn every_path_parses_back_to_its_segments() {
        for path in [
            segs(&[]),
            segs(&["tls", "ca"]),
            segs(&["peers", "0", "name"]),
            segs(&["tls.ca"]),
            segs(&["a", "b.c", "d"]),
            segs(&["a[0]"]),
            segs(&["quote\"d", "back\\slash"]),
            segs(&["", "x"]),
            segs(&["0"]),
        ] {
            assert_eq!(parse(&render(&path)), Some(path.clone()), "{path:?}");
        }
    }

    #[test]
    fn off_grammar_text_is_not_a_path() {
        for bad in [".a", "a.", "a..b", "a[0", "a]", "[a[b]", "a[0]b", "a.[0]"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
