// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a host files a library and a provider under.
//!
//! # The key is the host's policy, not the loader's
//!
//! Both halves get one. A library key of `%id` means one build of a
//! library at a time and a second is skipped; `%id@%version` means two
//! builds coexist. A provider key of `%id` is enough because an id is
//! unique on its own; `%id@%version` keeps two versions of one provider
//! apart.
//!
//! Neither answer is the loader's to pick: the same set of files is a
//! conflict for one host and a deliberate arrangement for another, and the
//! only one that can tell them apart is the host.
//!
//! # Which fields exist depends on what is being named
//!
//! A library has no display name and does not belong to another library,
//! so `%name` and `%library` are not fields of a library key — and naming
//! one is refused where it is written rather than rendering as empty.

#![forbid(unsafe_code)]

use std::fmt;

/// Which descriptor a template names the fields of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// A library: `%id`, `%version`.
    Library,
    /// A provider: `%id`, `%name`, `%library`, `%version`.
    Provider,
}

impl Subject {
    /// What a template for this may name, for a diagnostic.
    fn fields(self) -> &'static str {
        match self {
            Subject::Library => "%id, %version",
            Subject::Provider => "%id, %name, %library, %version",
        }
    }
}

/// A field of the descriptor a key can be built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Id,
    Name,
    Library,
    Version,
}

impl Field {
    fn parse(name: &str, subject: Subject) -> Option<Field> {
        let field = match name {
            "id" => Field::Id,
            "name" => Field::Name,
            "library" => Field::Library,
            "version" => Field::Version,
            _ => return None,
        };
        let allowed = match subject {
            Subject::Library => matches!(field, Field::Id | Field::Version),
            Subject::Provider => true,
        };
        allowed.then_some(field)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(Field),
}

/// What a template can be built from.
///
/// A library fills `id` and `version` and leaves the rest empty; a
/// template for a library cannot name the rest, so they are never read.
#[derive(Debug, Clone, Copy)]
pub struct KeyFields<'a> {
    /// `%id` — the library's or the provider's own identifier.
    pub id: &'a str,
    /// `%name` — a provider's display name, which may be empty.
    pub name: &'a str,
    /// `%library` — the id of the library offering this provider.
    pub library: &'a str,
    /// `%version` — its version.
    pub version: &'a str,
}

impl<'a> KeyFields<'a> {
    /// The two fields a library has.
    pub fn library(id: &'a str, version: &'a str) -> KeyFields<'a> {
        KeyFields {
            id,
            name: "",
            library: id,
            version,
        }
    }
}

/// Why a template could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyError {
    /// It names something this subject has no field for.
    UnknownField {
        /// What it named.
        name: String,
        /// What it was naming a field of.
        subject: Subject,
    },
    /// A `%` that begins nothing. `%%` is how a literal one is written.
    DanglingPercent,
    /// It names no field at all, so everything would land on one key and
    /// the second one loaded would look like a repeat of the first.
    NoFields,
    /// Two already loaded render the same key under it.
    Collides {
        /// The key they both render.
        key: String,
    },
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::UnknownField { name, subject } => write!(
                f,
                "`%{name}` is not a field of a {}; known ones are {}",
                match subject {
                    Subject::Library => "library",
                    Subject::Provider => "provider",
                },
                subject.fields()
            ),
            KeyError::DanglingPercent => {
                write!(f, "a `%` names no field; write `%%` for a literal one")
            }
            KeyError::NoFields => write!(
                f,
                "a key template naming no field gives everything the same key"
            ),
            KeyError::Collides { key } => {
                write!(f, "two already loaded both render `{key}`")
            }
        }
    }
}

impl std::error::Error for KeyError {}

/// How a host names what it loads.
///
/// | field | library | provider |
/// |---|---|---|
/// | `%id` | yes | yes |
/// | `%version` | yes | yes |
/// | `%name` | — | its display name, which may be empty |
/// | `%library` | — | the id of the library offering it |
///
/// `%%` is a literal `%`. Anything else is text and appears as written.
///
/// ```
/// use guatiao::library::KeyTemplate;
///
/// let default: KeyTemplate = Default::default();
/// assert_eq!(default.as_str(), "%id");
///
/// let side_by_side = KeyTemplate::library("%id@%version").unwrap();
/// assert_eq!(side_by_side.as_str(), "%id@%version");
///
/// // A library has no display name, so naming one is refused here and
/// // accepted for a provider.
/// assert!(KeyTemplate::library("%id-%name").is_err());
/// assert!(KeyTemplate::provider("%id-%name").is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyTemplate {
    subject: Subject,
    source: String,
    parts: Vec<Part>,
}

impl KeyTemplate {
    /// Parses one, refusing a field this subject does not have and a
    /// template that names no field at all.
    pub fn parse(subject: Subject, source: &str) -> Result<KeyTemplate, KeyError> {
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
            let field = Field::parse(name, subject).ok_or_else(|| KeyError::UnknownField {
                name: name.to_string(),
                subject,
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
            subject,
            source: source.to_string(),
            parts,
        })
    }

    /// A template naming a library.
    pub fn library(source: &str) -> Result<KeyTemplate, KeyError> {
        KeyTemplate::parse(Subject::Library, source)
    }

    /// A template naming a provider.
    pub fn provider(source: &str) -> Result<KeyTemplate, KeyError> {
        KeyTemplate::parse(Subject::Provider, source)
    }

    /// What it names the fields of.
    pub fn subject(&self) -> Subject {
        self.subject
    }

    /// The template as it was written.
    pub fn as_str(&self) -> &str {
        &self.source
    }

    /// The key one library or provider lands on.
    pub fn render(&self, fields: KeyFields<'_>) -> String {
        let mut out = String::with_capacity(self.source.len() + 16);
        for part in &self.parts {
            match part {
                Part::Text(text) => out.push_str(text),
                Part::Field(Field::Id) => out.push_str(fields.id),
                Part::Field(Field::Name) => out.push_str(fields.name),
                Part::Field(Field::Library) => out.push_str(fields.library),
                Part::Field(Field::Version) => out.push_str(fields.version),
            }
        }
        out
    }
}

/// `%id` over a provider: an id is unique on its own, so a host that
/// loads one build of each needs nothing more.
impl Default for KeyTemplate {
    fn default() -> KeyTemplate {
        KeyTemplate::provider("%id").expect("`%id` is one field and nothing else")
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
            id: "hello_greeter",
            name: "Hello",
            library: "hello_library",
            version: "0.1.0",
        }
    }

    #[test]
    fn the_default_is_the_id() {
        assert_eq!(KeyTemplate::default().render(fields()), "hello_greeter");
        assert_eq!(KeyTemplate::default().subject(), Subject::Provider);
    }

    #[test]
    fn every_provider_field_renders() {
        let t = KeyTemplate::provider("%id %name %library %version").unwrap();
        assert_eq!(
            t.render(fields()),
            "hello_greeter Hello hello_library 0.1.0"
        );
    }

    #[test]
    fn a_library_names_its_two_fields() {
        let t = KeyTemplate::library("%id@%version").unwrap();
        assert_eq!(
            t.render(KeyFields::library("hello_library", "0.1.0")),
            "hello_library@0.1.0"
        );
    }

    /// A library has no display name and belongs to no library, so naming
    /// either is refused where it is written rather than rendering empty.
    #[test]
    fn a_library_template_cannot_name_a_providers_fields() {
        for source in ["%id-%name", "%library"] {
            match KeyTemplate::library(source) {
                Err(KeyError::UnknownField { subject, .. }) => {
                    assert_eq!(subject, Subject::Library);
                }
                other => panic!("expected an unknown field for {source}, got {other:?}"),
            }
        }
        // And the same spellings are fine for a provider.
        assert!(KeyTemplate::provider("%id-%name").is_ok());
        assert!(KeyTemplate::provider("%library").is_ok());
    }

    #[test]
    fn text_around_a_field_survives() {
        let t = KeyTemplate::provider("provider:%id/v%version!").unwrap();
        assert_eq!(t.render(fields()), "provider:hello_greeter/v0.1.0!");
    }

    #[test]
    fn a_doubled_percent_is_a_literal_one() {
        let t = KeyTemplate::provider("%%%id%%").unwrap();
        assert_eq!(t.render(fields()), "%hello_greeter%");
    }

    #[test]
    fn a_field_nobody_has_is_refused() {
        assert_eq!(
            KeyTemplate::provider("%id@%revision"),
            Err(KeyError::UnknownField {
                name: "revision".to_string(),
                subject: Subject::Provider,
            })
        );
    }

    #[test]
    fn a_percent_that_begins_nothing_is_refused() {
        assert_eq!(
            KeyTemplate::provider("%id-%"),
            Err(KeyError::DanglingPercent)
        );
        assert_eq!(
            KeyTemplate::provider("%id %2"),
            Err(KeyError::DanglingPercent)
        );
    }

    /// A template of pure text gives everything one key, so the second
    /// thing loaded looks like a repeat of the first. Refused here rather
    /// than reported later as a conflict between two unrelated files.
    #[test]
    fn a_template_with_no_field_is_refused() {
        assert_eq!(KeyTemplate::provider("provider"), Err(KeyError::NoFields));
        assert_eq!(KeyTemplate::library("100%%"), Err(KeyError::NoFields));
    }
}
