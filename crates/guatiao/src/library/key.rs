// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a host files a provider under.
//!
//! # The key is the host's policy, not the loader's
//!
//! A provider id is unique, so `%id` is enough for a host that loads one
//! build of each. A host that wants two builds of one provider in one
//! process says `%id@%version` and gets two entries where the default
//! would have given it a duplicate.
//!
//! Neither answer is the loader's to pick: the same set of libraries is a
//! conflict for one host and a deliberate arrangement for another, and the
//! only one that can tell them apart is the host.

#![forbid(unsafe_code)]

use std::fmt;
use std::str::FromStr;

/// A field of the descriptor a key can be built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Id,
    Kind,
    Name,
    Library,
    Version,
}

impl Field {
    fn parse(name: &str) -> Option<Field> {
        match name {
            "id" => Some(Field::Id),
            "kind" => Some(Field::Kind),
            "name" => Some(Field::Name),
            "library" => Some(Field::Library),
            "version" => Some(Field::Version),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(Field),
}

/// What a template can be built from: one provider, and the library that
/// offered it.
#[derive(Debug, Clone, Copy)]
pub struct KeyFields<'a> {
    /// `%id` — the provider's own identifier.
    pub id: &'a str,
    /// `%kind` — what it speaks.
    pub kind: &'a str,
    /// `%name` — its display name, which may be empty.
    pub name: &'a str,
    /// `%library` — the id of the library that offered it.
    pub library: &'a str,
    /// `%version` — that library's version.
    pub version: &'a str,
}

/// Why a template could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyError {
    /// It names something no descriptor has.
    UnknownField {
        /// What it named.
        name: String,
    },
    /// A `%` that begins nothing. `%%` is how a literal one is written.
    DanglingPercent,
    /// It names no field at all, so every provider would land on one key
    /// and the second one loaded would be a duplicate of the first.
    NoFields,
    /// Two providers already loaded render the same key under it.
    Collides {
        /// The key they both render.
        key: String,
    },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::UnknownField { name } => write!(
                f,
                "`%{name}` is not a field; known ones are \
                 %id, %kind, %name, %library, %version"
            ),
            KeyError::DanglingPercent => {
                write!(f, "a `%` names no field; write `%%` for a literal one")
            }
            KeyError::NoFields => write!(
                f,
                "a key template naming no field keys every provider the same"
            ),
            KeyError::Collides { key } => {
                write!(f, "two providers already loaded both render `{key}`")
            }
        }
    }
}

impl std::error::Error for KeyError {}

/// How a host names the providers it loads.
///
/// | field | what it is |
/// |---|---|
/// | `%id` | the provider's own identifier |
/// | `%kind` | what it speaks |
/// | `%name` | its display name, which may be empty |
/// | `%library` | the id of the library that offered it |
/// | `%version` | that library's version |
///
/// `%%` is a literal `%`. Anything else is text and appears as written.
///
/// ```
/// use guatiao::library::KeyTemplate;
///
/// let default: KeyTemplate = Default::default();
/// assert_eq!(default.as_str(), "%id");
///
/// let side_by_side: KeyTemplate = "%id@%version".parse().unwrap();
/// assert_eq!(side_by_side.as_str(), "%id@%version");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyTemplate {
    source: String,
    parts: Vec<Part>,
}

impl KeyTemplate {
    /// Parses one, refusing a field nobody has and a template that names
    /// no field at all.
    pub fn parse(source: &str) -> Result<KeyTemplate, KeyError> {
        let mut parts: Vec<Part> = Vec::new();
        let mut text = String::new();
        let mut rest = source;
        let mut named_a_field = false;

        while let Some(at) = rest.find('%') {
            text.push_str(&rest[..at]);
            rest = &rest[at + 1..];

            if let Some(after) = rest.strip_prefix('%') {
                text.push('%');
                rest = after;
                continue;
            }

            let end = rest
                .find(|c: char| !c.is_ascii_lowercase() && c != '_')
                .unwrap_or(rest.len());
            if end == 0 {
                return Err(KeyError::DanglingPercent);
            }
            let name = &rest[..end];
            let field = Field::parse(name).ok_or_else(|| KeyError::UnknownField {
                name: name.to_string(),
            })?;
            rest = &rest[end..];

            if !text.is_empty() {
                parts.push(Part::Text(std::mem::take(&mut text)));
            }
            parts.push(Part::Field(field));
            named_a_field = true;
        }

        text.push_str(rest);
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        if !named_a_field {
            return Err(KeyError::NoFields);
        }

        Ok(KeyTemplate {
            source: source.to_string(),
            parts,
        })
    }

    /// The template as it was written.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// The key one provider lands on.
    pub fn render(&self, fields: KeyFields<'_>) -> String {
        let mut out = String::with_capacity(self.source.len() + 16);
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(Field::Id) => out.push_str(fields.id),
                Part::Field(Field::Kind) => out.push_str(fields.kind),
                Part::Field(Field::Name) => out.push_str(fields.name),
                Part::Field(Field::Library) => out.push_str(fields.library),
                Part::Field(Field::Version) => out.push_str(fields.version),
            }
        }
        out
    }
}

/// `%id`: a provider id is unique on its own, so a host that loads one
/// build of each needs nothing more.
impl Default for KeyTemplate {
    fn default() -> KeyTemplate {
        KeyTemplate::parse("%id").expect("`%id` is one field and nothing else")
    }
}

impl FromStr for KeyTemplate {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<KeyTemplate, KeyError> {
        KeyTemplate::parse(s)
    }
}

impl fmt::Display for KeyTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> KeyFields<'static> {
        KeyFields {
            id: "hello",
            kind: "greeter",
            name: "Hello",
            library: "hello_library",
            version: "0.1.0",
        }
    }

    #[test]
    fn the_default_is_the_provider_id() {
        assert_eq!(KeyTemplate::default().render(fields()), "hello");
    }

    #[test]
    fn every_field_renders() {
        let t: KeyTemplate = "%id %kind %name %library %version".parse().unwrap();
        assert_eq!(
            t.render(fields()),
            "hello greeter Hello hello_library 0.1.0"
        );
    }

    #[test]
    fn text_around_a_field_survives() {
        let t: KeyTemplate = "%id@%version".parse().unwrap();
        assert_eq!(t.render(fields()), "hello@0.1.0");
        let t: KeyTemplate = "provider:%id/v%version!".parse().unwrap();
        assert_eq!(t.render(fields()), "provider:hello/v0.1.0!");
    }

    #[test]
    fn a_doubled_percent_is_a_literal_one() {
        let t: KeyTemplate = "%%%id%%".parse().unwrap();
        assert_eq!(t.render(fields()), "%hello%");
    }

    #[test]
    fn a_field_nobody_has_is_refused() {
        assert_eq!(
            "%id@%revision".parse::<KeyTemplate>(),
            Err(KeyError::UnknownField {
                name: "revision".to_string()
            })
        );
    }

    #[test]
    fn a_percent_that_begins_nothing_is_refused() {
        assert_eq!(
            "%id-%".parse::<KeyTemplate>(),
            Err(KeyError::DanglingPercent)
        );
        assert_eq!(
            "%id %2".parse::<KeyTemplate>(),
            Err(KeyError::DanglingPercent)
        );
    }

    /// A template of pure text keys every provider the same, so the second
    /// one loaded would be a duplicate of the first. Refused here rather
    /// than reported as a conflict between two unrelated libraries.
    #[test]
    fn a_template_with_no_field_is_refused() {
        assert_eq!("provider".parse::<KeyTemplate>(), Err(KeyError::NoFields));
        assert_eq!("100%%".parse::<KeyTemplate>(), Err(KeyError::NoFields));
    }
}
