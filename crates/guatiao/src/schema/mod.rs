// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What a provider needs to be initialised.
//!
//! A schema lists the options something accepts: a key, a kind, an
//! optional default, presentation fields, and any number of annotations.
//! It is what a library hands a consumer so the consumer can build a
//! configuration without having heard of it before.
//!
//! # A schema IS a value
//!
//! It is a map, carried by the same containers as everything else, and it
//! crosses a boundary as a `Value` with no schema-shaped C type
//! anywhere. The keys it is written with are in [`vocab`], and **that
//! vocabulary is the contract**; the Rust types here are a typed way to
//! write and read one, never a second representation of it.
//!
//! That is what makes a consumer in another language cheap: it walks a map
//! it already knows how to walk. A Dart or Python reader needs the key
//! names and nothing else — no generated structs, no second ABI to keep in
//! step with the first.
//!
//! - [`build`] writes one: [`SchemaBuilder`], [`OptionBuilder`],
//!   [`KindBuilder`], [`ArmBuilder`].
//! - [`read`] reads one back: [`SchemaRef`], [`OptionRef`], [`KindRef`].
//! - [`validate`] answers whether a value is one the schema accepts.
//! - [`flat`] projects a tagged option onto flat `key -> text` storage,
//!   which is what a command line or a query string can carry.
//!
//! # Anything outside the vocabulary is an annotation
//!
//! Carried, and never interpreted. A newer producer's keyword survives a
//! round trip through an older reader, and a consumer can hang its own
//! hints on an option without asking for a vocabulary change. What gives
//! one of those a meaning is whoever reads it — `guatiao::schema::merge`
//! is an example, reading `x-merge` from over in the merge module because
//! a schema must not have to know that merging exists.
//!
//! # No serialisation lives here
//!
//! There is no JSON Schema emitter and no parser. How a schema is written
//! down is the consumer's decision, and a crate that shipped one would be
//! tracking somebody else's specification on behalf of every consumer,
//! including the ones that wanted a different spelling or no text form at
//! all. The format crate beside this one does that job and can carry more
//! than one format.
//!
//! # Presentation is optional; substance is not
//!
//! `label`, `help`, `section`, `advanced` and `order` may all be empty or
//! default and the schema is still correct and still useful. A consumer
//! with no user interface ignores them entirely. Never make a validation
//! or type behaviour depend on one.

#![forbid(unsafe_code)]

pub mod build;
pub mod describe;
pub mod flat;
// How a schema declares the way one of its options combines across
// layers: the `x-merge` annotation. Here rather than under the merge
// because the merge touches values only — it is this side that
// knows both halves. A `//` comment, never a `///`.
pub mod merge;
pub mod read;
pub mod validate;
pub mod vocab;

pub use build::{ArmBuilder, KindBuilder, OptionBuilder, SchemaBuilder};
pub use describe::Schema;
pub use flat::{SEPARATOR, flatten, is_sensitive, resolve, unflatten};
pub use read::{ArmRef, ChoiceRef, Kind as KindRef, OptionRef, SchemaRef, SectionRef};
pub use validate::{validate_map, validate_text, validate_texts, validate_value};

/// Why a value was rejected by [`validate_value`] or [`validate_map`].
///
/// Deliberately carries the option's **key** and a description of what
/// *would* have been accepted, but **never the offending value**. Options
/// are not secrets today, but an option may be marked
/// [`OptionRef::is_sensitive`], and an error type that quotes the input is
/// an error type that eventually logs a passphrase. Rejecting a value
/// without echoing it costs nothing here — the caller still has the value
/// it just passed in, and can decide for itself whether showing it is
/// safe.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// No option is declared under this key. Carries the declared keys so
    /// a caller can suggest the one that was meant — silently dropping a
    /// misspelled option is how a user ends up convinced a setting does
    /// nothing.
    UnknownOption {
        /// The key that was not found.
        key: String,
        /// Every key this schema does declare, in declaration order.
        known: Vec<String>,
    },
    /// A value was given for a declared option, but the option does not
    /// accept it.
    BadValue {
        /// The option's key.
        key: String,
        /// What the option would have accepted, phrased for a person:
        /// `"a number between 0 and 9"`.
        expected: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::UnknownOption { key, known } => {
                write!(f, "unknown option '{key}'")?;
                if !known.is_empty() {
                    write!(f, "; known options are: {}", known.join(", "))?;
                }
                Ok(())
            }
            ValidationError::BadValue { key, expected } => {
                write!(f, "option '{key}' expects {expected}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}
