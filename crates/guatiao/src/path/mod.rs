// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Naming one place inside a value: `agent[1].name[home].host`.
//!
//! A path is text, and text is what a flat store, a command line, a query
//! string and a form's hints all carry. One grammar, parsed in one place,
//! so none of them can disagree about what a path means.
//!
//! ```
//! use guatiao::path;
//!
//! let mut inner = guatiao::Map::new();
//! inner.set("host", "10.0.0.1")?;
//! let mut list = guatiao::List::new();
//! list.push(inner)?;
//! let mut root = guatiao::Map::new();
//! root.set("agent", list)?;
//! let value = guatiao::Value::from(root);
//!
//! let at = path::parse("agent[0].host").expect("a path");
//! assert_eq!(path::get(&value, at).and_then(|v| v.try_into().ok()), Some("10.0.0.1"));
//! # Ok::<(), guatiao::ValueError>(())
//! ```
//!
//! # The grammar
//!
//! ```text
//! path    := segment ( '.' name | '[' index ']' )*
//! segment := name | '[' index ']'
//! name    := one or more characters, none of them '.', '[' or ']'
//! index   := digits | quoted | one or more characters, none of them '[' or ']'
//! quoted  := '"' ( any character, with \" and \\ escaped ) '"'
//! ```
//!
//! **Rootless**, so there is no leading `.` and no `$`: jq's leading dot
//! is there because a jq path is an expression over one input, and these
//! are keys in a store.
//!
//! # What a bracket means is decided where it is resolved
//!
//! `[1]` is a list index **and** the key `"1"`; which one it is depends on
//! what it is applied to, and nothing else. That is what lets `env[PATH]`
//! be written without ceremony — the common spelling, and the one this
//! grammar was asked for.
//!
//! Quoting is the **escape**, not the rule. `["1"]` is only ever the key
//! `"1"`, and `["a[b]"]` is the only way to write a key holding a bracket.
//! A `.` needs no quoting inside brackets, because the `]` already says
//! where the segment ends.
//!
//! # A declaration may not contain a delimiter; data may
//!
//! [`schema::flat::check_keys`](crate::schema::flat::check_keys) refuses a
//! declared field key containing `.`, `[` or `]`, at the moment the schema
//! is finished. A schema's keys are ours to spell. The keys **inside** a
//! value are not — they come from whoever wrote the data — which is the
//! whole reason quoting exists.
//!
//! # Walking is iterative
//!
//! [`get`] and [`get_mut`] loop; they do not recurse. A path is as deep as
//! its text says, which a sender controls, and a stack that a sender
//! controls is not a stack.

#![forbid(unsafe_code)]

mod walk;

pub use walk::{get, get_mut};

use std::borrow::Cow;
use std::fmt;

/// One step of a path.
///
/// `Index` and `Key` are both bracket segments and differ only in what
/// they were written as: `[0]` could mean either the first element of a
/// list or the entry under `"0"`, and `["0"]` could only ever mean the
/// second. See the module note.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Segment<'a> {
    /// `name`, or `.name` after another segment: an entry of a map.
    Field(&'a str),
    /// `[0]`: the element at this position, or the entry under its
    /// decimal text.
    Index(usize),
    /// `[name]` or `["a.b"]`: an entry of a map, and never a position.
    Key(Cow<'a, str>),
}

impl fmt::Display for Segment<'_> {
    /// The spelling this segment parses back from.
    ///
    /// A [`Key`](Segment::Key) is quoted when leaving it bare would parse
    /// as something else: all digits would read as an [`Index`](
    /// Segment::Index), and a bracket or a leading quote would not parse
    /// at all.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Segment::Field(name) => f.write_str(name),
            Segment::Index(at) => write!(f, "[{at}]"),
            Segment::Key(key) if needs_quoting(key) => {
                f.write_str("[\"")?;
                for c in key.chars() {
                    if c == '"' || c == '\\' {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                f.write_str("\"]")
            }
            Segment::Key(key) => write!(f, "[{key}]"),
        }
    }
}

/// Whether writing this key bare would parse back as something else.
///
/// **Only a leading** `"` matters: a quoted segment is recognised by the
/// character right after the `[`, so `[say "hi"]` reads back as the key it
/// looks like. Quoting only what has to be quoted is what keeps the
/// common spelling readable.
fn needs_quoting(key: &str) -> bool {
    key.is_empty()
        || key.starts_with('"')
        || key.contains(['[', ']'])
        || key.bytes().all(|b| b.is_ascii_digit())
}

/// A parsed path: valid, and borrowed from the text it was parsed from.
///
/// Parsing checks the whole string once, so iterating cannot fail. The
/// segments are produced on demand rather than collected, which is what
/// keeps a path free of allocation — a quoted key holding an escape is the
/// one exception, and it allocates only that key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path<'a> {
    text: &'a str,
}

impl<'a> Path<'a> {
    /// The text this was parsed from, unchanged.
    pub fn as_str(&self) -> &'a str {
        self.text
    }

    /// The segments, in order.
    pub fn segments(&self) -> Segments<'a> {
        Segments {
            rest: self.text,
            at: 0,
        }
    }

    /// The first segment and the rest of the path, or `None` when there is
    /// no rest.
    ///
    /// For a caller that descends one level and hands the remainder to
    /// somebody else — a sub-form over one field's members is the case
    /// this exists for.
    pub fn split_first(&self) -> Option<(Segment<'a>, Option<Path<'a>>)> {
        let mut segments = self.segments();
        let first = segments.next()?;
        let rest = segments.remainder();
        Some((first, rest))
    }
}

impl fmt::Display for Path<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text)
    }
}

impl<'a> IntoIterator for Path<'a> {
    type Item = Segment<'a>;
    type IntoIter = Segments<'a>;

    fn into_iter(self) -> Segments<'a> {
        self.segments()
    }
}

/// The segments of a [`Path`], in order.
#[derive(Debug, Clone)]
pub struct Segments<'a> {
    /// What is left to read, starting at a segment boundary.
    rest: &'a str,
    /// Where `rest` starts in the original text, for the remainder.
    at: usize,
}

impl<'a> Segments<'a> {
    /// What is left, as a path of its own. `None` when nothing is.
    fn remainder(&self) -> Option<Path<'a>> {
        let rest = self.rest.strip_prefix('.').unwrap_or(self.rest);
        (!rest.is_empty()).then_some(Path { text: rest })
    }
}

impl<'a> Iterator for Segments<'a> {
    type Item = Segment<'a>;

    fn next(&mut self) -> Option<Segment<'a>> {
        // Parsing already accepted this text, so every step here is
        // reading what was checked rather than checking it again. A
        // malformed remainder is unreachable, and it answers `None`
        // rather than panicking, because a panic in an iterator is a
        // denial of service dressed as an invariant.
        let (segment, read) = step(self.rest).ok()??;
        self.rest = &self.rest[read..];
        self.at += read;
        Some(segment)
    }
}

/// Why a path is not one. Every variant says where, as a byte offset into
/// the text that was given.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathError {
    /// The text is empty. A path names at least one step.
    Empty,
    /// A field name with no characters: a leading or doubled `.`, or a
    /// trailing one.
    EmptyField {
        /// Where the name would have started.
        at: usize,
    },
    /// `[]`, which names neither a position nor a key.
    EmptyIndex {
        /// The `[`.
        at: usize,
    },
    /// A `[` with no `]` after it.
    UnclosedIndex {
        /// The `[`.
        at: usize,
    },
    /// A `"` with no closing `"`.
    UnclosedQuote {
        /// The opening `"`.
        at: usize,
    },
    /// A `\` at the end of a quoted key, or before a character that is
    /// neither `"` nor `\`.
    BadEscape {
        /// The `\`.
        at: usize,
    },
    /// A `]` where no `[` is open.
    UnexpectedClose {
        /// The `]`.
        at: usize,
    },
    /// Something after a closing `"` other than `]`.
    TrailingQuote {
        /// The first character after the closing `"`.
        at: usize,
    },
}

impl PathError {
    /// Where in the text, as a byte offset. 0 for an empty path.
    pub fn offset(&self) -> usize {
        match self {
            PathError::Empty => 0,
            PathError::EmptyField { at }
            | PathError::EmptyIndex { at }
            | PathError::UnclosedIndex { at }
            | PathError::UnclosedQuote { at }
            | PathError::BadEscape { at }
            | PathError::UnexpectedClose { at }
            | PathError::TrailingQuote { at } => *at,
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => f.write_str("an empty path names nothing"),
            PathError::EmptyField { at } => write!(f, "an empty field name at {at}"),
            PathError::EmptyIndex { at } => write!(f, "an empty `[]` at {at}"),
            PathError::UnclosedIndex { at } => write!(f, "a `[` at {at} with no `]`"),
            PathError::UnclosedQuote { at } => write!(f, "a `\"` at {at} with no closing `\"`"),
            PathError::BadEscape { at } => {
                write!(f, "a `\\` at {at} before something other than `\"` or `\\`")
            }
            PathError::UnexpectedClose { at } => write!(f, "a `]` at {at} with no `[`"),
            PathError::TrailingQuote { at } => write!(f, "text after a quoted key at {at}"),
        }
    }
}

impl std::error::Error for PathError {}

/// Reads a path, checking the whole of it.
///
/// The returned [`Path`] borrows `text`, and iterating it cannot fail.
pub fn parse(text: &str) -> Result<Path<'_>, PathError> {
    if text.is_empty() {
        return Err(PathError::Empty);
    }
    // A path is rootless, so the first step names a field outright. jq's
    // leading dot is there because a jq path is an expression over one
    // input; these are keys in a store, and `.host` would be a key with
    // an empty first segment.
    if text.starts_with('.') {
        return Err(PathError::EmptyField { at: 0 });
    }
    let mut rest = text;
    let mut at = 0;
    while !rest.is_empty() {
        let read = match step(rest) {
            Ok(Some((_, read))) => read,
            // `step` answers `None` only for an empty remainder, which the
            // loop condition already excluded.
            Ok(None) => return Err(PathError::EmptyField { at }),
            Err(e) => return Err(offset_by(e, at)),
        };
        rest = &rest[read..];
        at += read;
    }
    Ok(Path { text })
}

/// Moves an error's offset from "inside this step" to "inside the path".
fn offset_by(error: PathError, by: usize) -> PathError {
    match error {
        PathError::Empty => PathError::Empty,
        PathError::EmptyField { at } => PathError::EmptyField { at: at + by },
        PathError::EmptyIndex { at } => PathError::EmptyIndex { at: at + by },
        PathError::UnclosedIndex { at } => PathError::UnclosedIndex { at: at + by },
        PathError::UnclosedQuote { at } => PathError::UnclosedQuote { at: at + by },
        PathError::BadEscape { at } => PathError::BadEscape { at: at + by },
        PathError::UnexpectedClose { at } => PathError::UnexpectedClose { at: at + by },
        PathError::TrailingQuote { at } => PathError::TrailingQuote { at: at + by },
    }
}

/// One segment off the front of `rest`, and how many bytes it took.
///
/// `rest` starts at a segment boundary: either at the very beginning, or
/// just after a segment, where a `.` may still be waiting.
fn step(rest: &str) -> Result<Option<(Segment<'_>, usize)>, PathError> {
    if rest.is_empty() {
        return Ok(None);
    }
    // The separator belongs to the segment that follows it, so the byte
    // count below covers it and a caller never has to remember.
    let (body, skipped) = match rest.strip_prefix('.') {
        Some(body) => (body, 1),
        None => (rest, 0),
    };
    if body.starts_with('[') {
        let (segment, read) = bracket(body).map_err(|e| offset_by(e, skipped))?;
        return Ok(Some((segment, skipped + read)));
    }
    let end = body.find(['.', '[', ']']).unwrap_or(body.len());
    if let Some(']') = body[end..].chars().next() {
        return Err(PathError::UnexpectedClose { at: skipped + end });
    }
    if end == 0 {
        return Err(PathError::EmptyField { at: skipped });
    }
    Ok(Some((Segment::Field(&body[..end]), skipped + end)))
}

/// A bracket segment, `body` starting AT the `[` so every offset below is
/// relative to it. Answers how many bytes it took, closing `]` included.
fn bracket(body: &str) -> Result<(Segment<'_>, usize), PathError> {
    let inside = &body[1..];
    if let Some(quoted) = inside.strip_prefix('"') {
        // `quoted_key` counts from the opening `"`, which is one past the
        // `[` this function counts from.
        let (key, read) = quoted_key(quoted).map_err(|e| offset_by(e, 1))?;
        return match inside[1 + read..].strip_prefix(']') {
            Some(_) => Ok((Segment::Key(key), 1 + 1 + read + 1)),
            None => Err(PathError::TrailingQuote { at: 1 + 1 + read }),
        };
    }
    let Some(end) = inside.find(']') else {
        return Err(PathError::UnclosedIndex { at: 0 });
    };
    if end == 0 {
        return Err(PathError::EmptyIndex { at: 0 });
    }
    let text = &inside[..end];
    let segment = match text.parse::<usize>() {
        // Only a bare run of digits. `parse` would also take `+1` and
        // whitespace, and a key called `+1` must stay a key.
        Ok(at) if text.bytes().all(|b| b.is_ascii_digit()) => Segment::Index(at),
        _ => Segment::Key(Cow::Borrowed(text)),
    };
    Ok((segment, 1 + end + 1))
}

/// A quoted key, `quoted` starting just after the opening `"`; offsets are
/// relative to that. Answers how many bytes it took, closing `"` included.
fn quoted_key(quoted: &str) -> Result<(Cow<'_, str>, usize), PathError> {
    // Borrowed while nothing is escaped, which is the common case; the
    // first `\` is what makes a copy necessary.
    let mut escaped = false;
    for (i, c) in quoted.char_indices() {
        match c {
            '\\' => {
                escaped = true;
                break;
            }
            '"' => return Ok((Cow::Borrowed(&quoted[..i]), i + 1)),
            _ => {}
        }
    }
    if !escaped {
        // The offset a caller wants is the opening `"`, one before this.
        return Err(PathError::UnclosedQuote { at: 0 });
    }

    let mut out = String::new();
    let mut chars = quoted.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((Cow::Owned(out), i + 1)),
            '\\' => match chars.next() {
                Some((_, next @ ('"' | '\\'))) => out.push(next),
                // A `\` before anything else, or at the end: refused
                // rather than passed through, so one spelling means one
                // key.
                _ => return Err(PathError::BadEscape { at: i + 1 }),
            },
            other => out.push(other),
        }
    }
    Err(PathError::UnclosedQuote { at: 0 })
}
