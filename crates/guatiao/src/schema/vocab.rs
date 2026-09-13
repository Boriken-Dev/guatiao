// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The keys a schema is written with.
//!
//! **A schema IS a value.** It is a map, holding lists of maps, carried by
//! the same containers as everything else this crate moves. There is no
//! second set of C structs for it, no second tagged union, and no second
//! versioning scheme.
//!
//! # Why, stated once
//!
//! A schema is metadata about configuration, which is exactly what the
//! value model exists to carry. Giving it typed structs of its own meant
//! reinventing containers that already existed, and the bill came in fast:
//! five list types duplicating `List`, five view types duplicating
//! `Values`, a second tagged union with its own frozen tag space,
//! `struct_size` on four more types, a by-value cycle no header generator
//! could order, and every one of those types dropped **silently** from the
//! generated header because a schema crosses as data and no function
//! signature names one.
//!
//! As a value, all of that is somebody else's solved problem. A new
//! keyword is a new key rather than an appended field with a versioning
//! ceremony. A format crate serialises a schema for free, because it
//! already serialises values. A builder is `map_set` and `list_push`.
//!
//! The line this draws is worth keeping: **data crosses as a value, and
//! only code crosses as a typed struct.** An allocator's callbacks, a
//! provider's vtable and the library entry point are function pointers, so
//! they stay structs. Everything a consumer reads is a value.
//!
//! # The shape
//!
//! ```text
//! schema  := { "options": [option…], "sections": [section…], … }
//! section := { "id", "label", "help" }
//! option  := { "key", "kind": kind, "label", "help", "section",
//!              "default": <any>, "order": <number>,
//!              "advanced": <bool>, "sensitive": <bool>, "required": <bool>, … }
//! kind    := { "type": "bool"|"int"|"float"|"string"|"enum"|"union"|"variant", … }
//!              int, float  ->  "min", "max"       (each optional)
//!              enum        ->  "choices": [ {"value","label"}, … ]
//!              union       ->  "arms": [kind, …]
//!              variant     ->  "tag", "arms": [ {"value","label","help","fields":[option…]} ]
//! ```
//!
//! # Rules a reader must follow
//!
//! **An unrecognised `"type"` means skip that one option**, not reject the
//! schema. Refusing the whole document because one option came from a
//! newer producer hides every option that would have rendered fine. This
//! is the same rule as an unknown value tag, and it is the whole
//! forward-compatibility story.
//!
//! **Presentation keys are optional; substance is not.** `label`, `help`,
//! `section`, `order` and `advanced` may all be missing and the schema is
//! still correct and still useful. A consumer with no user interface
//! ignores them. Never make a validation or type behaviour depend on one.
//!
//! **A missing `default` and a `default` of null are different things.**
//! The first means the option has no default; the second means its default
//! is nothing. Absence is the query answer, which is why the value model
//! separates absent from null in the first place.
//!
//! **Any key not listed here is an annotation and is not interpreted.**
//! That is the point of an open vocabulary: a hint one consumer
//! understands is invisible to every other party rather than a feature
//! they all must implement. By convention an annotation is prefixed `x-`,
//! but nothing enforces it, because a key nobody reads cannot collide with
//! a meaning nobody assigned.
//!
//! **A choice is a row, never a pair of arrays.** `{"value","label"}` per
//! alternative. The shape this refuses — a values list beside a labels
//! list — lets the two drift in length or order, which shows a person one
//! option while setting another.

#![forbid(unsafe_code)]

// --- the schema -------------------------------------------------------

/// The options a provider declares, as a list of maps.
pub const OPTIONS: &str = "options";
/// The sections a consumer may group options into, as a list of maps.
pub const SECTIONS: &str = "sections";

// --- a section --------------------------------------------------------

/// A section's identifier, which an option's [`SECTION`] refers to.
pub const ID: &str = "id";

// --- an option --------------------------------------------------------

/// The key this option's value is stored under. The one required key.
pub const KEY: &str = "key";
/// What the option accepts: a nested map, keyed by [`TYPE`].
pub const KIND: &str = "kind";
/// A short human label. Optional.
pub const LABEL: &str = "label";
/// Longer human help. Optional.
pub const HELP: &str = "help";
/// Which section this belongs to. Optional; empty means the default one.
pub const SECTION: &str = "section";
/// The default value, of whatever kind the option accepts.
///
/// **Absent and null are different.** No `default` key means the option
/// has no default; a `default` of null means its default is nothing.
pub const DEFAULT: &str = "default";
/// Declaration position, as a number.
///
/// Not a preference. If every option omits it, a consumer has no ordering
/// information and falls back to something arbitrary — alphabetical,
/// usually — which silently rearranges a carefully grouped form.
pub const ORDER: &str = "order";
/// True when the option is advanced: hidden behind a disclosure by
/// default.
pub const ADVANCED: &str = "advanced";
/// True when the value is a secret: masked in a form, encrypted in
/// storage.
pub const SENSITIVE: &str = "sensitive";
/// True when the option must be given.
///
/// It exists because without it a required field and an optional one
/// produce identical schemas, so a generated schema could not express the
/// distinction its own source made.
pub const REQUIRED: &str = "required";

// --- a kind -----------------------------------------------------------

/// Which kind this is: one of the `TYPE_*` constants below.
pub const TYPE: &str = "type";

/// True or false.
pub const TYPE_BOOL: &str = "bool";
/// A whole number, optionally bounded by [`MIN`] and [`MAX`].
pub const TYPE_INT: &str = "int";
/// A real number, optionally bounded by [`MIN`] and [`MAX`].
pub const TYPE_FLOAT: &str = "float";
/// Free text.
pub const TYPE_STRING: &str = "string";
/// Exactly one of a fixed set of alternatives, in [`CHOICES`].
pub const TYPE_ENUM: &str = "enum";
/// Any one of several kinds, in [`ARMS`].
///
/// **Untagged.** Validation succeeds if any arm accepts, and knowing which
/// arm took the value is explicitly not the point — a port number *or* the
/// word `auto`.
pub const TYPE_UNION: &str = "union";
/// Opaque bytes.
///
/// A value kind the schema can name, so a field carrying a certificate or
/// a ticket is described as what it is rather than as text that happens to
/// survive. It has no text form and therefore no bounds.
pub const TYPE_BYTES: &str = "bytes";
/// A sequence of values, every one of the kind under [`ITEMS`].
pub const TYPE_LIST: &str = "list";
/// A nested object, whose options are under [`FIELDS`].
///
/// The same spelling a variant arm uses for the options it adds, and
/// deliberately so: "here are more options" is one idea, and a reader that
/// walks an arm's fields walks these with the same code.
pub const TYPE_MAP: &str = "map";
/// One of several alternatives, each carrying its own named fields.
///
/// **Tagged**, so knowing the arm IS the point: the discriminant is stored
/// under [`TAG`]. Do not treat this like a union. A reader that rendered a
/// union's arms as a selector would produce a control that stores a bare
/// discriminant with no payload — a control promising a capability it does
/// not have.
pub const TYPE_VARIANT: &str = "variant";

/// Inclusive lower bound of a numeric kind. Optional.
pub const MIN: &str = "min";
/// Inclusive upper bound of a numeric kind. Optional.
pub const MAX: &str = "max";
/// An enum's alternatives: a list of `{VALUE, LABEL}` maps.
pub const CHOICES: &str = "choices";
/// A union's or a variant's arms.
///
/// For a union, a list of kinds. For a variant, a list of arm maps keyed
/// by [`VALUE`], [`LABEL`], [`HELP`] and [`FIELDS`].
pub const ARMS: &str = "arms";
/// The key a variant's discriminant is stored under.
///
/// Flattened onto `key -> text` storage — a config map, a URI query — the
/// discriminant lands at the option's own key and each payload field at
/// `<key>.<field>`.
pub const TAG: &str = "tag";
/// A choice's or an arm's stored value.
pub const VALUE: &str = "value";
/// The kind every element of a list has.
pub const ITEMS: &str = "items";
/// The options an arm of a variant adds when it is selected.
///
/// **An arm with no fields is ordinary and complete.** It is the common
/// case — "use the ambient credential" — so treating an empty list as
/// missing data would make the common case the exception everyone has to
/// remember.
pub const FIELDS: &str = "fields";

/// Every key this vocabulary assigns a meaning.
///
/// A reader uses it to tell an annotation from a keyword it merely does
/// not know: anything outside this list is an annotation, and annotations
/// are carried, never interpreted.
pub const KEYWORDS: &[&str] = &[
    OPTIONS, SECTIONS, ID, KEY, KIND, LABEL, HELP, SECTION, DEFAULT, ORDER, ADVANCED, SENSITIVE,
    REQUIRED, TYPE, MIN, MAX, CHOICES, ARMS, TAG, VALUE, FIELDS, ITEMS,
];

/// Every kind name this vocabulary assigns a meaning.
pub const TYPES: &[&str] = &[
    TYPE_BOOL,
    TYPE_INT,
    TYPE_FLOAT,
    TYPE_STRING,
    TYPE_BYTES,
    TYPE_LIST,
    TYPE_MAP,
    TYPE_ENUM,
    TYPE_UNION,
    TYPE_VARIANT,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is flat and every name is distinct. A duplicate
    /// would mean two concepts sharing a key, which a reader cannot undo.
    #[test]
    fn every_keyword_is_distinct() {
        let mut seen = KEYWORDS.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "a key means one thing: {KEYWORDS:?}");

        let mut kinds = TYPES.to_vec();
        kinds.sort_unstable();
        let before = kinds.len();
        kinds.dedup();
        assert_eq!(kinds.len(), before);
    }

    /// Nothing in the vocabulary carries the annotation prefix, or a
    /// reader could not use the prefix to tell the two apart at a glance.
    #[test]
    fn no_keyword_looks_like_an_annotation() {
        for k in KEYWORDS {
            assert!(!k.starts_with("x-"), "{k} would read as an annotation");
        }
    }
}
