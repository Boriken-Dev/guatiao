// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The keys a schema is written with: **JSON Schema's**.
//!
//! **A schema IS a value, and the value IS a JSON Schema.** It is a map,
//! holding maps, carried by the same containers as everything else this
//! crate moves — and the keys in it are the ones the 2020-12 specification
//! already assigns. There is no second set of C structs for it, no second
//! tagged union, no emitter and no parser.
//!
//! # Why these keys and not ours
//!
//! A vocabulary of our own would have to be translated by every consumer
//! that wanted to hand a schema to anything else — a form generator, a
//! validator, an editor's completion. Writing the specification's keys from
//! the start means that translation does not exist: a schema this crate
//! builds, written out by `guatiao-serde` in any format, is a document
//! those tools already read.
//!
//! What we add, we add in the space the specification reserves for exactly
//! that: a key outside its vocabulary is an annotation, carried and not
//! interpreted, and `x-` prefixed by convention. Every key this crate
//! invented is in [`EXTENSIONS`] and every one of them is prefixed.
//!
//! # The dialect
//!
//! [`DIALECT`], declared under [`SCHEMA`] on the document a
//! [`SchemaBuilder`](super::SchemaBuilder) finishes — and only there,
//! because a subschema is part of the same document and does not get to
//! change dialect midway.
//!
//! 2020-12 rather than draft-07 or 2019-09 because it costs nothing today:
//! every keyword below is identical in all three. The differences are in
//! tuple `items`, `definitions` vs `$defs`, `$ref` siblings, `dependencies`
//! and `format`, and this vocabulary emits none of them.
//!
//! # The shape
//!
//! ```text
//! schema  := { "$schema", "type": "object", "title", "description",
//!              "properties": { key: schema, … }, "required": [key, …] }
//! schema  := { "type": "boolean" }
//!          | { "type": "integer", "minimum", "maximum" }
//!          | { "type": "number",  "minimum", "maximum" }
//!          | { "type": "string" }
//!          | { "type": "string", "enum": [value, …], "x-enum-labels": {…} }
//!          | { "type": "bytes" }                        (ours)
//!          | { "type": "array",  "items": schema }
//!          | { "type": "object", "properties", "required" }
//!          | { "type": "object", "x-variant-tag": key, "oneOf": [arm, …] }
//!          | { "anyOf": [schema, …] }
//! arm     := { "title", "description",
//!              "properties": { <tag>: {"const": value}, … },
//!              "required": [<tag>, …] }
//! ```
//!
//! Plus, on any of them: `default`, `title`, `description` and
//! [`X_SENSITIVE`]. Presentation keys — `x-section`, `x-order`,
//! `x-advanced` — are named by `guatiao-intake`, not here.
//!
//! # An undeclared key is refused, and the document says so
//!
//! Every object the builders seal carries [`ADDITIONAL_PROPERTIES`]
//! `: false` — a struct's document and each arm's subschema — so a general
//! JSON Schema validator refuses exactly what [`validate`](super::validate)
//! refuses. A key nobody declared is a misspelling worth reporting, and
//! silently dropping one is how somebody ends up convinced a setting does
//! nothing.
//!
//! # Rules a reader must follow
//!
//! **An unrecognised `"type"` means skip that one field**, not reject the
//! schema. Refusing the whole document because one field came from a
//! newer producer hides every field that would have rendered fine. This
//! is the same rule as an unknown value tag, and it is the whole
//! forward-compatibility story.
//!
//! **Presentation keys are optional; substance is not.** `title`,
//! `description` and every `x-` key another crate hangs on a field may be
//! missing and the schema is still correct and still useful. A consumer
//! with no user interface ignores them. Never make a validation or type
//! behaviour depend on one.
//!
//! **A missing `default` and a `default` of null are different things.**
//! The first means the field has no default; the second means its default
//! is nothing. Absence is the query answer, which is why the value model
//! separates absent from null in the first place.
//!
//! **A name lives in one place.** A field's name is its key in
//! [`PROPERTIES`] and appears nowhere inside the field, and whether it is
//! required is a name in the owner's [`REQUIRED`] list and appears nowhere
//! inside the field either. Both are JSON Schema's arrangement, and both
//! are why a builder collects its fields and writes them at `finish`
//! rather than appending as it goes.
//!
//! **A choice's label is keyed, never parallel.** [`X_ENUM_LABELS`] is a map
//! from the value in [`ENUM`] to what a person is shown. The shape this
//! refuses — a labels list beside the values list — lets the two drift in
//! length or order, which shows a person one alternative while storing
//! another. A map cannot drift, because the value is the key.

#![forbid(unsafe_code)]

// --- the document -----------------------------------------------------

/// The dialect declaration, on the root document only.
pub const SCHEMA: &str = "$schema";

/// The dialect this crate writes: JSON Schema 2020-12.
///
/// The value of [`SCHEMA`], and the only line that would change to emit an
/// older draft — every keyword here is spelled the same in draft-07 and
/// 2019-09.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

// --- any schema -------------------------------------------------------

/// Which kind this is: one of the `TYPE_*` constants below.
pub const TYPE: &str = "type";
/// A short human name. Optional.
///
/// What the Rust builders call `label`.
pub const TITLE: &str = "title";
/// Longer human prose. Optional.
///
/// What the Rust builders call `help`.
pub const DESCRIPTION: &str = "description";
/// The default value, of whatever kind this accepts.
///
/// **Absent and null are different.** No `default` key means there is no
/// default; a `default` of null means the default is nothing.
pub const DEFAULT: &str = "default";

// --- an object --------------------------------------------------------

/// The fields an object declares: a **map** from name to schema.
///
/// A map rather than a list, which is what JSON Schema says. **Declaration
/// order survives**, because this crate's maps are insertion-ordered by
/// contract — worth stating, since it is the one thing a JSON Schema
/// consumer would not normally be able to rely on.
///
/// It does not make a lookup cheaper: a map here is an insertion-ordered
/// association list, so finding a key is a scan either way.
pub const PROPERTIES: &str = "properties";
/// The names that must be given: a list of strings, on the **object**.
///
/// Not a flag on the field. A field and its requiredness are declared in
/// two different places, which is JSON Schema's arrangement; the builders
/// hide it by collecting `required()` calls and writing the list at
/// `finish`.
pub const REQUIRED: &str = "required";
/// `additionalProperties`: written as `false` on every object the
/// builders seal, because [`validate`](super::validate) refuses an
/// undeclared key and the document should say so.
pub const ADDITIONAL_PROPERTIES: &str = "additionalProperties";

// --- an array ---------------------------------------------------------

/// The schema every element of an array satisfies.
pub const ITEMS: &str = "items";

// --- a number ---------------------------------------------------------

/// Inclusive lower bound of a numeric kind. Optional.
pub const MINIMUM: &str = "minimum";
/// Inclusive upper bound of a numeric kind. Optional.
pub const MAXIMUM: &str = "maximum";

// --- alternatives -----------------------------------------------------

/// The permitted values of an enumeration: a list.
pub const ENUM: &str = "enum";
/// The one value a schema accepts, which is how an arm carries its
/// discriminant.
pub const CONST: &str = "const";
/// **Any** of these schemas accepting is enough: an untagged union.
///
/// `anyOf` and not `oneOf`, deliberately. A union asks whether the value is
/// acceptable at all — a port number *or* the word `auto` — and knowing
/// which arm took it is explicitly not the point. `oneOf` would reject a
/// value two arms both accept, which is a rule nobody declaring a union
/// meant to state.
pub const ANY_OF: &str = "anyOf";
/// **Exactly one** of these schemas: a tagged variant's arms.
///
/// Exact here because the arms are told apart by a discriminant, so a value
/// matching two of them is a contradiction rather than a convenience.
pub const ONE_OF: &str = "oneOf";

// --- what this crate adds ---------------------------------------------
//
// Three keys that used to be here — `x-section`, `x-order` and
// `x-advanced` — are `guatiao-intake`'s, in its own `vocab`. They name how
// to organise controls on a screen, which is an opinion this crate does
// not hold. They stay perfectly legal in a document written here: unknown
// to `known()`, so carried as annotations and interpreted by whoever
// draws the form.

/// True when the value is a secret: never print it.
///
/// **Not presentation**, which is why it stayed when the rest went. A
/// form masks it, but so does a log, a crash dump and anything else that
/// renders a value into text, none of which have a screen. What each of
/// them does about it is its own decision; this says only that somebody
/// declared the field one.
pub const X_SENSITIVE: &str = "x-sensitive";
/// What a person is shown for each value in [`ENUM`]: a map from the value
/// to its label.
///
/// Keyed rather than parallel, so a label cannot come adrift from the value
/// it belongs to. A value with no entry shows as itself.
///
/// **A map, where the earlier emitter in this family wrote a parallel
/// array** under the same name. That emitter's own documentation called
/// the array form out as one that "cannot mispair an enum value with its
/// label, which this text form structurally can" — so the shape is the
/// fix and the name is kept. An old reader meets a map where it wanted an
/// array, which fails loudly or yields no labels; it never yields the
/// wrong ones.
pub const X_ENUM_LABELS: &str = "x-enum-labels";
/// The key a variant's discriminant is stored under.
///
/// JSON Schema has no discriminator keyword — OpenAPI's is not JSON
/// Schema — and the alternative is for a reader to infer it by finding the
/// property that is [`CONST`] in every arm, which stops working the moment
/// two properties are. So it is carried.
///
/// Spelled out rather than `x-tag`: an annotation space is shared with
/// every vendor, and "tag" is the most overloaded word in the
/// neighbourhood — a label on a field, a marker on a resource. The
/// earlier emitter in this family chose the same long name.
///
/// **Written even for a variant with no arms**, which is what lets a
/// reader tell an empty variant from an empty union.
///
/// Flattened onto `key -> text` storage — a config map, a URI query — the
/// discriminant lands at the field's own key and each payload field at
/// `<key>.<field>`.
pub const X_VARIANT_TAG: &str = "x-variant-tag";

// --- the types --------------------------------------------------------

/// True or false.
pub const TYPE_BOOLEAN: &str = "boolean";
/// A whole number, optionally bounded by [`MINIMUM`] and [`MAXIMUM`].
pub const TYPE_INTEGER: &str = "integer";
/// A real number, optionally bounded by [`MINIMUM`] and [`MAXIMUM`].
pub const TYPE_NUMBER: &str = "number";
/// Free text, or — with [`ENUM`] — one of a fixed set of alternatives.
pub const TYPE_STRING: &str = "string";
/// A sequence of values, every one satisfying [`ITEMS`].
pub const TYPE_ARRAY: &str = "array";
/// A nested object, whose fields are under [`PROPERTIES`].
///
/// Also what a tagged variant is: an object carrying [`X_VARIANT_TAG`] and
/// [`ONE_OF`].
pub const TYPE_OBJECT: &str = "object";
/// Opaque bytes. **Ours, and not a JSON Schema type.**
///
/// A value kind the schema can name, so a field carrying a certificate or a
/// ticket is described as what it is rather than as text that happens to
/// survive. It has no text form and therefore no bounds.
///
/// The consequence, stated so nobody rediscovers it: JSON Schema's
/// meta-schema fixes `type` to a closed set, so a document saying
/// `"type": "bytes"` is readable by anything and **fails a strict
/// schema-of-the-schema check**. Data is unaffected. The escape hatch, if
/// it is ever wanted, is `"type": "string"` with a `contentEncoding` of our
/// choosing, which carries the same information in a valid document — and
/// says the bytes are text, which they are not.
pub const TYPE_BYTES: &str = "bytes";

/// Every key JSON Schema assigns a meaning that this crate writes.
pub const KEYWORDS: &[&str] = &[
    SCHEMA,
    TYPE,
    TITLE,
    DESCRIPTION,
    DEFAULT,
    PROPERTIES,
    REQUIRED,
    ITEMS,
    MINIMUM,
    MAXIMUM,
    ENUM,
    CONST,
    ANY_OF,
    ONE_OF,
    ADDITIONAL_PROPERTIES,
];

/// Every key **this crate** invented, all of them `x-` prefixed.
pub const EXTENSIONS: &[&str] = &[X_SENSITIVE, X_ENUM_LABELS, X_VARIANT_TAG];

/// Every key that has a meaning at all: [`KEYWORDS`] and [`EXTENSIONS`].
///
/// A reader uses it to tell an annotation from a key it merely does not
/// know: anything outside this list is an annotation, and annotations are
/// carried, never interpreted. `x-merge` is one of those — this crate's
/// merge gives it a meaning from over in its own module, and the schema
/// itself still does not know that merging exists.
pub fn known(key: &str) -> bool {
    KEYWORDS.contains(&key) || EXTENSIONS.contains(&key)
}

/// Every type name this vocabulary assigns a meaning.
pub const TYPES: &[&str] = &[
    TYPE_BOOLEAN,
    TYPE_INTEGER,
    TYPE_NUMBER,
    TYPE_STRING,
    TYPE_ARRAY,
    TYPE_OBJECT,
    TYPE_BYTES,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is flat and every name is distinct. A duplicate
    /// would mean two concepts sharing a key, which a reader cannot undo.
    #[test]
    fn every_keyword_is_distinct() {
        let mut seen: Vec<&str> = KEYWORDS.iter().chain(EXTENSIONS).copied().collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "a key means one thing: {seen:?}");

        let mut kinds = TYPES.to_vec();
        kinds.sort_unstable();
        let before = kinds.len();
        kinds.dedup();
        assert_eq!(kinds.len(), before);
    }

    /// The prefix is the whole boundary between what the specification
    /// says and what we added, so it has to hold in both directions.
    #[test]
    fn the_prefix_separates_ours_from_theirs() {
        for k in EXTENSIONS {
            assert!(k.starts_with("x-"), "{k} is ours and must say so");
        }
        for k in KEYWORDS {
            assert!(
                !k.starts_with("x-"),
                "{k} is JSON Schema's and must not be prefixed"
            );
        }
    }

    /// Exactly one type is not JSON Schema's, and it is the one the
    /// meta-schema consequence is recorded against.
    #[test]
    fn bytes_is_the_only_type_we_invented() {
        const JSON_SCHEMA_TYPES: [&str; 7] = [
            "null", "boolean", "object", "array", "number", "string", "integer",
        ];
        let ours: Vec<&&str> = TYPES
            .iter()
            .filter(|t| !JSON_SCHEMA_TYPES.contains(t))
            .collect();
        assert_eq!(ours, [&TYPE_BYTES], "extending the type set is a decision");
    }
}
