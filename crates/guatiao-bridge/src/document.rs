// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A manifest's text, read through `anycfg` into a guatiao value.
//!
//! The file's name never chooses the format: a first line `#!toml`, `#!json`
//! or `#!yaml` does, and is removed before parsing, and a document without
//! one is TOML. JSON and YAML are read only when the bridge is built with
//! its `json` and `yaml` features; a document that names a format not built
//! in is refused, saying so. A failure carries the position the parser gave
//! and never quotes the document.

use std::sync::OnceLock;

use anycfg::{Callbacks, CodecError, Formats, Options};
use guatiao::{Alloc, Value};
use guatiao_serde::{Presentation, text};

/// What a codec may pass on: the parser's words and the position it gave,
/// without the format name or a position of its own (anycfg adds the
/// position, counted from the document's first line).
fn failure(error: text::Error) -> CodecError {
    match error {
        text::Error::Syntax {
            position, detail, ..
        } => {
            let at = match position {
                Some(p) if p.line > 0 => {
                    let at = CodecError::new().line(p.line);
                    // Anycfg counts columns from 1; a parser reporting the end of
                    // its input as column 0 says no column.
                    if p.column > 0 {
                        at.column(p.column)
                    } else {
                        at
                    }
                }
                _ => CodecError::new(),
            };
            at.message(detail)
        }
        text::Error::Format { detail, .. } | text::Error::Unsupported { detail, .. } => {
            CodecError::new().message(detail)
        }
        other => CodecError::new().message(other.to_string()),
    }
}

fn utf8(bytes: &[u8]) -> Result<&str, CodecError> {
    core::str::from_utf8(bytes).map_err(|error| {
        CodecError::at_offset(bytes, error.valid_up_to()).message("not valid UTF-8")
    })
}

fn read_only(_: &Value) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::new().message("documents are only read here"))
}

#[cfg(not(all(feature = "json", feature = "yaml")))]
fn not_built_in(feature: &'static str) -> impl Fn(&[u8]) -> Result<Value, CodecError> {
    move |_| {
        Err(CodecError::new().message(format!(
            "this format is not built in (guatiao-bridge's `{feature}` feature)"
        )))
    }
}

fn build() -> Result<Formats<Value>, anycfg::Error> {
    let mut formats = Formats::new();
    formats.register(
        text::toml::NAME,
        ["toml"],
        Callbacks::new(
            |bytes: &[u8]| {
                text::toml::from_str(utf8(bytes)?, Alloc::rust(), Presentation::new())
                    .map_err(failure)
            },
            read_only,
        ),
    )?;
    #[cfg(feature = "json")]
    let json = Callbacks::new(
        |bytes: &[u8]| {
            text::json::from_str(utf8(bytes)?, Alloc::rust(), Presentation::new()).map_err(failure)
        },
        read_only,
    );
    #[cfg(not(feature = "json"))]
    let json = Callbacks::new(not_built_in("json"), read_only);
    formats.register("json", ["json"], json)?;
    #[cfg(feature = "yaml")]
    let yaml = Callbacks::new(
        |bytes: &[u8]| {
            text::yaml::from_str(utf8(bytes)?, Alloc::rust(), Presentation::new()).map_err(failure)
        },
        read_only,
    );
    #[cfg(not(feature = "yaml"))]
    let yaml = Callbacks::new(not_built_in("yaml"), read_only);
    formats.register("yaml", ["yaml", "yml"], yaml)?;
    formats.set_default(text::toml::NAME)?;
    Ok(formats)
}

fn formats() -> &'static Formats<Value> {
    static FORMATS: OnceLock<Formats<Value>> = OnceLock::new();
    FORMATS.get_or_init(|| build().expect("the formats register"))
}

/// A manifest's bytes as a value, in the format their first line names,
/// else TOML.
pub(crate) fn parse(bytes: &[u8]) -> Result<Value, anycfg::Error> {
    formats().from_slice(bytes, Options::new())
}
