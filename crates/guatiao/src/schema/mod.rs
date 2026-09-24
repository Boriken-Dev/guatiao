// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a value is, and what a valid one looks like.
//!
//! A schema lists the fields something accepts: a name, what it accepts,
//! an optional default, presentation hints, and any number of
//! annotations. It is what lets a consumer build or understand a value it
//! has never heard of — a provider's configuration, a record, a set of
//! capabilities, metadata one library hands another.
//!
//! # A schema IS a value, and the value IS a JSON Schema
//!
//! It is a map, carried by the same containers as everything else, and it
//! crosses a boundary as a `Value` with no schema-shaped C type
//! anywhere. The keys it is written with are **JSON Schema's own**, listed
//! in [`vocab`], and that vocabulary is the contract; the Rust types here
//! are a typed way to write and read one, never a second representation of
//! it.
//!
//! That is what makes a consumer in another language cheap twice over. It
//! walks a map it already knows how to walk — a Dart or Python reader
//! needs the key names and nothing else, no generated structs and no
//! second ABI to keep in step with the first. And the key names are ones
//! its ecosystem probably already has a library for.
//!
//! - [`build`] writes one: [`SchemaBuilder`], [`FieldBuilder`],
//!   [`KindBuilder`], [`ArmBuilder`].
//!
//! ```
//! use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
//!
//! // Building names no allocator, the same as `Map::new()`.
//! let schema = SchemaBuilder::new()
//!     .field(
//!         FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
//!             .title("Port")
//!             .required(),
//!     )
//!     .finish()
//!     .expect("a schema this small does not exhaust an allocator");
//!
//! let read = guatiao::schema::read::SchemaRef::new(&schema).expect("a schema is a map");
//! assert!(read.find("port").expect("it declares `port`").is_required());
//! ```
//! - [`read`] reads one back: [`SchemaRef`], [`FieldRef`], [`KindRef`].
//! - [`validate`] answers whether a value is one the schema accepts.
//!
//! # A schema is a description, and nothing else
//!
//! Naming a place inside a value (`agent[1].name`), projecting one onto
//! flat `key -> text` storage, and checking a store of text are **not
//! here**: they live in `guatiao-intake`, as its `path`, `flat` and
//! `validate_texts`. Each is a decision about how a consumer reads,
//! enters or stores a value, and a schema describes the struct exactly
//! as it is — which is why a field key may hold a `.` and nothing here
//! objects. `validate_text` stays, because "is this text acceptable for
//! this field" is a question about the schema alone.
//!
//! # Anything outside the vocabulary is an annotation
//!
//! Carried, and never interpreted. A newer producer's keyword survives a
//! round trip through an older reader, and a consumer can hang its own
//! hints on a field without asking for a vocabulary change. What gives
//! one of those a meaning is whoever reads it — `guatiao::schema::merge`
//! is an example, reading `x-merge` from over in the merge module because
//! a schema must not have to know that merging exists.
//!
//! # No serialisation lives here
//!
//! There is no JSON Schema emitter and no parser, and writing the
//! specification's keys is what removes the need for either: the value
//! already IS the document, so `guatiao-serde` writes it in JSON, TOML or
//! YAML with no knowledge of schemas at all. How a schema is written down
//! stays the consumer's decision, including the consumers that want no
//! text form.
//!
//! # Presentation is optional; substance is not
//!
//! `title` and `description` may be empty and the schema is still
//! correct and still useful; a consumer with no user interface ignores
//! them entirely. Never make a validation or type behaviour depend on
//! one. They are JSON Schema's own keywords, written here by
//! [`SchemaBuilder::title`] and its siblings.
//!
//! **Every other presentation key belongs to whoever draws the thing.**
//! Such a key reaches a schema through [`Extras`] or [`option`
//! ](SchemaBuilder::option), is carried as an annotation, and is read
//! back with [`FieldRef::extra`]. This crate names none of them, writes
//! none of them and reads none of them by name — which is what lets the
//! crate that does hold that opinion change its mind without touching
//! anything here.

#![forbid(unsafe_code)]

pub mod build;
// What a Rust type says its schema is. A `//` comment, never a `///`.
pub mod describe;
// How a schema declares the way one of its fields combines across
// layers: the `x-merge` annotation. Here rather than under the merge
// because the merge touches values only — it is this side that
// knows both halves. A `//` comment, never a `///`.
pub mod merge;
pub mod read;
pub mod validate;
pub mod vocab;

pub use build::{ArmBuilder, Extras, FieldBuilder, KindBuilder, SchemaBuilder};
pub use describe::Schema;
pub use read::{ArmRef, ChoiceRef, FieldRef, Kind as KindRef, SchemaRef};
pub use validate::{validate_map, validate_text, validate_value};

/// Why a value was rejected by [`validate_value`] or [`validate_map`].
///
/// Deliberately carries the field's **key** and a description of what
/// *would* have been accepted, but **never the offending value**. Fields
/// are not secrets today, but a field may be marked
/// [`FieldRef::is_sensitive`], and an error type that quotes the input is
/// an error type that eventually logs a passphrase. Rejecting a value
/// without echoing it costs nothing here — the caller still has the value
/// it just passed in, and can decide for itself whether showing it is
/// safe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// No field is declared under this key. Carries the declared keys so
    /// a caller can suggest the one that was meant — silently dropping a
    /// misspelled field is how a user ends up convinced a setting does
    /// nothing.
    UnknownOption {
        /// The key that was not found.
        key: String,
        /// Every key this schema does declare, in declaration order.
        known: Vec<String>,
    },
    /// A value was given for a declared field, but the field does not
    /// accept it.
    BadValue {
        /// The field's key.
        key: String,
        /// What the field would have accepted, phrased for a person:
        /// `"a number between 0 and 9"`.
        expected: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::UnknownOption { key, known } => {
                write!(f, "unknown field '{key}'")?;
                if !known.is_empty() {
                    write!(f, "; known fields are: {}", known.join(", "))?;
                }
                Ok(())
            }
            ValidationError::BadValue { key, expected } => {
                write!(f, "field '{key}' expects {expected}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}
