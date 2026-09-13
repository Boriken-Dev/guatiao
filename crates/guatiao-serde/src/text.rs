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

use std::fmt;

/// Why a document could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The parser or writer refused it, and said why.
    Format {
        /// Which format.
        format: &'static str,
        /// Its own message, which names the real cause better than any
        /// wording here could.
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
            Error::Format { format, .. } | Error::Unsupported { format, .. } => format,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Format { format, detail } => write!(f, "{format}: {detail}"),
            Error::Unsupported { format, detail } => {
                write!(f, "{format} cannot carry this: {detail}")
            }
        }
    }
}

impl std::error::Error for Error {}

#[cfg(feature = "json")]
pub mod json {
    //! JSON, through `serde_json`.
    //!
    //! **`arbitrary_precision` is on**, so a number arrives as its own
    //! text: `1.10` reads back `1.10` rather than `1.1`, and a 200-digit
    //! integer survives. That reaches `serde_json::Value` everywhere in a
    //! consumer's graph, because cargo unifies features — a consumer who
    //! does not want it turns off this crate's `json` feature and hands a
    //! `serde_json::Deserializer` to [`ValueSeed`](crate::ValueSeed)
    //! themselves.

    use serde::de::DeserializeSeed;

    use guatiao::value::alloc::Alloc;
    use guatiao::value::types::Value;

    use super::Error;
    use crate::{Numbers, Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "json";

    /// JSON is the one format that can take a number as raw text, so
    /// this module sets that policy rather than leaving it to a caller
    /// who would have to know the token exists.
    fn exact(how: Presentation) -> Presentation {
        how.numbers(Numbers::RawText)
    }

    /// A value as compact JSON.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_json::to_string(&Serializable::new(value, exact(how))).map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })
    }

    /// The same, indented for a person to read.
    pub fn to_string_pretty(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_json::to_string_pretty(&Serializable::new(value, exact(how))).map_err(|e| {
            Error::Format {
                format: NAME,
                detail: e.to_string(),
            }
        })
    }

    /// One value out of a JSON document, built through `alloc`.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        let mut de = serde_json::Deserializer::from_str(text);
        let value = ValueSeed::with(alloc, how)
            .deserialize(&mut de)
            .map_err(|e| Error::Format {
                format: NAME,
                detail: e.to_string(),
            })?;
        // Trailing rubbish is a malformed document, not a value with a
        // tail: a reader that stopped at the first complete value would
        // accept `{} garbage` and report success.
        de.end().map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })?;
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

    use super::Error;
    use crate::{Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "toml";

    /// A value as TOML. The value must be a map.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        if value.tag() != Ok(Tag::GUATIAO_MAP) {
            return Err(Error::Unsupported {
                format: NAME,
                detail: "a TOML document is a table, so only a map can be one".to_string(),
            });
        }
        ::toml::to_string(&Serializable::new(value, how)).map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })
    }

    /// One value out of a TOML document, built through `alloc`.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        let de = ::toml::Deserializer::parse(text).map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })?;
        ValueSeed::with(alloc, how)
            .deserialize(de)
            .map_err(|e| Error::Format {
                format: NAME,
                detail: e.to_string(),
            })
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

    use super::Error;
    use crate::{Presentation, Serializable, ValueSeed};

    /// The format's name, for a diagnostic.
    pub const NAME: &str = "yaml";

    /// A value as YAML.
    pub fn to_string(value: &Value, how: Presentation) -> Result<String, Error> {
        serde_saphyr::to_string(&Serializable::new(value, how)).map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })
    }

    /// One value out of a YAML document, built through `alloc`.
    ///
    /// The deserialiser arrives through a closure rather than a
    /// constructor — it borrows the parser's own state, so the crate hands
    /// it out only for as long as that state lives. The seed goes in;
    /// the value comes out.
    pub fn from_str(text: &str, alloc: Alloc, how: Presentation) -> Result<Value, Error> {
        serde_saphyr::with_deserializer_from_str(text, |de| {
            ValueSeed::with(alloc, how).deserialize(de)
        })
        .map_err(|e| Error::Format {
            format: NAME,
            detail: e.to_string(),
        })
    }
}
