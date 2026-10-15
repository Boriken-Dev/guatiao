// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The formats this crate names, one module each.
//!
//! Each is a feature, and each is the same two functions over the same
//! two types — the [`Serializable`](crate::Serializable) wrapper and the
//! [`ValueSeed`](crate::ValueSeed). Nothing format-specific happens in
//! the value model; these modules are the plumbing to somebody else's
//! parser and nothing more.
//!
//! A format this crate has never heard of still works: reach for
//! `to_serde` and `ValueSeed` directly and hand them that format's own
//! serialiser. These exist because a caller usually has a string and
//! wants a value.

#![forbid(unsafe_code)]

use alloc::fmt;

/// Where in a document a parser stopped: 1-based, a column counted in
/// characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The line, from 1.
    pub line: usize,
    /// The column, from 1.
    pub column: usize,
}

/// Why a document could not be read or written.
///
/// **No variant carries text from the document**: a document can hold a
/// secret, so a parser's own error that quotes the offending line is
/// translated into its message and a [`Position`], and the excerpt is
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The writer refused a value, and said why.
    Format {
        /// Which format.
        format: &'static str,
        /// Its own message, which names the real cause better than any
        /// wording here could.
        detail: String,
    },
    /// The text is not a document of the format, or is one nested deeper
    /// than a value can be.
    Syntax {
        /// Which format.
        format: &'static str,
        /// Where the parser stopped, when it said.
        position: Option<Position>,
        /// What it expected, in the parser's words and without an excerpt
        /// of the document.
        detail: String,
    },
    /// The document is fine and this format cannot carry that shape.
    Unsupported {
        /// Which format.
        format: &'static str,
        /// What it cannot carry.
        detail: String,
    },
}

impl Error {
    /// Which format refused.
    pub fn format(&self) -> &'static str {
        match self {
            Error::Format { format, .. }
            | Error::Syntax { format, .. }
            | Error::Unsupported { format, .. } => format,
        }
    }

    /// Where a parser stopped, for an [`Error::Syntax`] that knows.
    pub fn position(&self) -> Option<Position> {
        match self {
            Error::Syntax { position, .. } => *position,
            _ => None,
        }
    }

    /// A parse refusal at `position`, in the parser's own words.
    #[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
    fn syntax(format: &'static str, position: Option<Position>, detail: String) -> Error {
        Error::Syntax {
            format,
            position,
            detail,
        }
    }

    /// A refusal to write a value.
    #[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
    fn writing(format: &'static str, detail: impl fmt::Display) -> Error {
        Error::Format {
            format,
            detail: detail.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format { format, detail } => write!(f, "{format}: {detail}"),
            Error::Syntax {
                format,
                position: Some(Position { line, column }),
                detail,
            } => write!(f, "{format}: {detail} at line {line}, column {column}"),
            Error::Syntax {
                format,
                position: None,
                detail,
            } => write!(f, "{format}: {detail}"),
            Error::Unsupported { format, detail } => {
                write!(f, "{format} cannot carry this: {detail}")
            }
        }
    }
}

/// The 1-based line and column of byte `offset` in `text`.
#[cfg(feature = "toml")]
fn position_at(text: &str, offset: usize) -> Position {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |at| at + 1);
    Position {
        line,
        column: before[start..].chars().count() + 1,
    }
}

impl core::error::Error for Error {}

#[cfg(feature = "json")]
pub mod json {
    //! JSON, through `serde_json`.
    //!
    //! **Writing is exact; reading is exact with `exact-numbers`.** A
    //! number goes out as its own text (`1.10`, a 200-digit integer,
    //! `1e400`). It comes back as that text only when this crate's
    //! `exact-numbers` feature turns `serde_json/arbitrary_precision` on;
    //! without it, a number that fits no `i64`/`u64` reads as an `f64` and
    //! `1.10` reads back `1.1`. The feature reaches `serde_json::Value`
    //! everywhere in a consumer's graph, because cargo unifies features,
    //! which is why it is not part of `json`.

    use serde::de::DeserializeSeed;

    use guatiao::value::alloc::Alloc;
    use guatiao::value::types::Value;

    use super::{Error, Position};
    use crate::{Numbers, Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "json";

    /// JSON is the one format that can take a number as raw text, so
    /// this module sets that policy rather than leaving it to a caller
    /// who would have to know the token exists.
    fn exact(how: Presentation) -> Presentation {
        how.numbers(Numbers::RawText)
    }

    /// A parse refusal as its message and position. serde_json writes the
    /// position into the message, so it is taken back out of it, and
    /// counts the column in bytes, so it is counted again in characters.
    fn refusal(text: &str, e: &serde_json::Error) -> Error {
        let (line, column) = (e.line(), e.column());
        let full = e.to_string();
        let detail = full
            .strip_suffix(&format!(" at line {line} column {column}"))
            .unwrap_or(&full)
            .to_string();
        let position = (line > 0).then(|| Position {
            line,
            column: in_characters(text, line, column),
        });
        Error::syntax(NAME, position, detail)
    }

    /// The column of byte `column` (from 1) of line `line`, counted in
    /// characters. A byte inside a character counts as that character.
    fn in_characters(text: &str, line: usize, column: usize) -> usize {
        let row = text.split('\n').nth(line - 1).unwrap_or("");
        let mut end = column.min(row.len());
        while !row.is_char_boundary(end) {
            end += 1;
        }
        row[..end].chars().count()
    }

    /// A value as compact JSON.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_json::to_string(&Serializable::new(value, exact(how)))
            .map_err(|e| Error::writing(NAME, e))
    }

    /// The same, indented for a person to read.
    pub fn to_string_pretty(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_json::to_string_pretty(&Serializable::new(value, exact(how)))
            .map_err(|e| Error::writing(NAME, e))
    }

    /// One value out of a JSON document, built through `alloc`.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        let mut de = serde_json::Deserializer::from_str(text);
        let value = ValueSeed::with(alloc, how)
            .deserialize(&mut de)
            .map_err(|e| refusal(text, &e))?;
        // Trailing rubbish is a malformed document, not a value with a
        // tail: a reader that stopped at the first complete value would
        // accept `{} garbage` and report success.
        de.end().map_err(|e| refusal(text, &e))?;
        Ok(value)
    }
}

#[cfg(feature = "toml")]
pub mod toml {
    //! TOML, through the `toml` crate.
    //!
    //! # A TOML document is a TABLE
    //!
    //! The format has no spelling for a bare string, number or array at
    //! the top level, so writing one is [`Error::Unsupported`] rather than
    //! a wrapper object invented here. Wrapping would put a key in the
    //! document that the value never had, and every reader would then have
    //! to know to strip it.

    use serde::de::DeserializeSeed;

    use guatiao::value::alloc::Alloc;
    use guatiao::value::types::{Tag, Value};

    use super::{Error, position_at};
    use crate::{Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "toml";

    /// A parse refusal as the parser's message and where it stopped. The
    /// parser's own `Display` quotes the offending line, so it is not used.
    fn refusal(text: &str, e: &::toml::de::Error) -> Error {
        let position = e.span().map(|span| position_at(text, span.start));
        Error::syntax(NAME, position, e.message().to_string())
    }

    /// A value as TOML. The value must be a map.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        if value.tag() != Ok(Tag::GUATIAO_MAP) {
            return Err(Error::Unsupported {
                format: NAME,
                detail: "a TOML document is a table, so only a map can be one".to_string(),
            });
        }
        ::toml::to_string(&Serializable::new(value, how)).map_err(|e| Error::writing(NAME, e))
    }

    /// One value out of a TOML document, built through `alloc`.
    ///
    /// A table keeps the order the document wrote it in, and a date or
    /// time reads as its text.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        let de = ::toml::Deserializer::parse(text).map_err(|e| refusal(text, &e))?;
        ValueSeed::with(alloc, how)
            .deserialize(de)
            .map_err(|e| refusal(text, &e))
    }
}

#[cfg(feature = "yaml")]
pub mod yaml {
    //! YAML, through `serde-saphyr`.
    //!
    //! # Why that one
    //!
    //! The obvious crates are gone: `serde_yaml` is deprecated and
    //! `serde_yml` is deprecated too, its own description calling itself
    //! an unmaintained shim. Of what is maintained, this one **forbids
    //! unsafe and emphasises panic-free parsing** — which are the two
    //! properties that matter for a library whose whole job is reading a
    //! document somebody else wrote, at an FFI boundary where a panic is
    //! an abort.

    use serde::de::DeserializeSeed;

    use guatiao::value::alloc::Alloc;
    use guatiao::value::types::Value;

    use alloc::borrow::Cow;

    use serde_saphyr::{DefaultMessageFormatter, MessageFormatter, Options};

    use super::{Error, Position};
    use crate::{Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "yaml";

    /// The parser's message, but not where it names the document's own
    /// text: a key it saw twice, a value it would not accept, a tag or an
    /// anchor. Those are the cases whose message quotes the input, and a
    /// document can hold a secret.
    struct WithoutExcerpts;

    impl MessageFormatter for WithoutExcerpts {
        fn format_message<'a>(&self, err: &'a serde_saphyr::Error) -> Cow<'a, str> {
            use serde_saphyr::Error as E;
            Cow::Borrowed(match err {
                E::DuplicateMappingKey { .. } => "duplicate mapping key",
                E::NonFiniteFloat { .. } => "a non-finite float is not accepted",
                E::QuotingRequired { .. } => "a value that reads as another type needs quoting",
                E::SerdeInvalidType { .. } | E::SerdeInvalidValue { .. } => "an unexpected value",
                E::SerdeUnknownVariant { .. } | E::SerdeUnknownField { .. } => "an unknown name",
                E::TaggedEnumMismatch { .. } | E::UnsupportedTag { .. } => "an unsupported tag",
                E::UnknownAnchor { .. } => "an alias names no anchor",
                // The scanner's own message quotes the character.
                E::ExternalMessage { msg, .. } if msg.starts_with("unexpected character") => {
                    "an unexpected character"
                }
                _ => return DefaultMessageFormatter.format_message(err),
            })
        }
    }

    /// A parse refusal as the parser's message and where it stopped. The
    /// parser's own `Display` shows lines of the document around the
    /// error, so the message is asked for on its own.
    fn refusal(e: &serde_saphyr::Error) -> Error {
        let position = e.location().and_then(|at| {
            Some(Position {
                line: usize::try_from(at.line()).ok().filter(|line| *line > 0)?,
                column: usize::try_from(at.column()).ok()?,
            })
        });
        // The formatter's text is not neutralised the way the parser's own
        // rendering is, so control characters are made inert here, and
        // only those: a quote is the parser's wording, not an escape.
        let detail = WithoutExcerpts.format_message(e.without_snippet());
        let mut inert = String::with_capacity(detail.len());
        for c in detail.chars() {
            if c.is_control() {
                inert.extend(c.escape_default());
            } else {
                inert.push(c);
            }
        }
        Error::syntax(NAME, position, inert)
    }

    /// A value as YAML.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_saphyr::to_string(&Serializable::new(value, how)).map_err(|e| Error::writing(NAME, e))
    }

    /// One value out of a YAML document, built through `alloc`.
    ///
    /// A `!!binary` scalar reads as its base64 text: reading goes through
    /// `deserialize_any`, which tells the seed nothing of the tag, so
    /// bytes are out of reach, and the text is neither decoded behind the
    /// caller's back nor refused when it is not UTF-8.
    ///
    /// The deserialiser arrives through a closure rather than a
    /// constructor — it borrows the parser's own state, so the crate hands
    /// it out only for as long as that state lives. The seed goes in;
    /// the value comes out.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        let mut options = Options::default();
        options.with_snippet = false;
        options.ignore_binary_tag_for_string = true;
        serde_saphyr::with_deserializer_from_str_with_options(text, options, |de| {
            ValueSeed::with(alloc, how).deserialize(de)
        })
        .map_err(|e| refusal(&e))
    }
}
