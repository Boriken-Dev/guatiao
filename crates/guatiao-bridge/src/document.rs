// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Configuration files, read through `anycfg` into a guatiao value.
//!
//! `.toml`, `.json`, `.yaml` and `.yml` choose their format by extension. A
//! `.cfg` file names its format on its first line (`#!toml`, `#!json`,
//! `#!yaml`, the line's last word), which is removed before parsing, and is
//! TOML when it has no such line. Any other extension is refused with the
//! list of the accepted ones. A failure names the file and, when the parser
//! knows it, the position; it never quotes the document.

use std::path::Path;
use std::sync::OnceLock;

use anycfg::{Callbacks, CodecError, Formats, Options};
use guatiao::{Alloc, Value};
use guatiao_serde::{Presentation, text};

fn failure(error: text::Error) -> CodecError {
    let at = error.position().map_or_else(CodecError::new, |position| {
        CodecError::new()
            .line(position.line)
            .column(position.column)
    });
    at.message(error.to_string())
}

fn utf8(bytes: &[u8]) -> Result<&str, CodecError> {
    core::str::from_utf8(bytes).map_err(|error| {
        CodecError::at_offset(bytes, error.valid_up_to()).message("not valid UTF-8")
    })
}

fn read_only(_: &Value) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::new().message("documents are only read here"))
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
    formats.register(
        text::json::NAME,
        ["json"],
        Callbacks::new(
            |bytes: &[u8]| {
                text::json::from_str(utf8(bytes)?, Alloc::rust(), Presentation::new())
                    .map_err(failure)
            },
            read_only,
        ),
    )?;
    formats.register(
        text::yaml::NAME,
        ["yaml", "yml"],
        Callbacks::new(
            |bytes: &[u8]| {
                text::yaml::from_str(utf8(bytes)?, Alloc::rust(), Presentation::new())
                    .map_err(failure)
            },
            read_only,
        ),
    )?;
    formats.register_auto(["cfg"], text::toml::NAME)?;
    Ok(formats)
}

/// The three formats over one value, `.cfg` choosing by its first line and
/// then TOML.
pub fn formats() -> &'static Formats<Value> {
    static FORMATS: OnceLock<Formats<Value>> = OnceLock::new();
    FORMATS.get_or_init(|| build().expect("the three formats register"))
}

/// The document `bytes` of a file named `name`, as a value: only the
/// extension of `name` is used.
pub fn parse(name: &str, bytes: &[u8]) -> Result<Value, anycfg::Error> {
    formats().from_slice(bytes, Options::new().name(name))
}

/// The document in the file at `path`, as a value. The error names the file.
pub fn read(path: &Path) -> Result<Value, String> {
    formats()
        .load(path, Options::new())
        .map_err(|e| format!("{}: {e}", path.display()))
}
