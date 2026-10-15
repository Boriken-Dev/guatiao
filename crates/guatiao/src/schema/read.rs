// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a schema out of a value.
//!
//! A schema is a map written with the keys in [`super::vocab`] — JSON
//! Schema's. These are typed views over one: they borrow, allocate
//! nothing, and hand a Rust caller an exhaustive [`Kind`] to match on
//! instead of a pile of string comparisons.
//!
//! # A field is a name AND a schema
//!
//! JSON Schema keeps a field's name in the `properties` map key and its
//! requiredness in the owner's `required` list, so neither is inside the
//! field. A [`FieldRef`] therefore carries all three, and is made by
//! whatever walked the object — which is the only thing that knows them.
//!
//! # Skipping is the rule, not an error path
//!
//! A reader that meets something it does not understand skips that one
//! thing and carries on. A property whose name is not text is not a field,
//! so [`SchemaRef::fields`] passes over it; a kind whose `type` this build
//! does not know arrives as [`Kind::Unknown`] so the field's name, title
//! and description still render.
//!
//! Refusing the whole schema instead would hide every field that was
//! perfectly readable, which is the opposite of what forward
//! compatibility is for.

#![forbid(unsafe_code)]

use super::vocab;
use crate::value::convert::TryAsRef;
use crate::value::read::{bool_or, float_or};
use crate::value::types::{List, Map, Number, Tag, Value};

/// A schema: what a value is, and what a valid one looks like.
#[derive(Clone, Copy, Debug)]
pub struct SchemaRef<'a>(&'a Value);

/// One field a provider declares: its name, its schema, and whether the
/// object it belongs to requires it.
#[derive(Clone, Copy, Debug)]
pub struct FieldRef<'a> {
    key: &'a str,
    schema: &'a Value,
    required: bool,
}

/// One alternative of an enumeration.
///
/// A value and the label keyed against it, resolved here so a caller never
/// has to know that the two live in different keys.
#[derive(Clone, Copy, Debug)]
pub struct ChoiceRef<'a> {
    value: &'a str,
    label: &'a str,
}

/// One arm of a tagged kind.
#[derive(Clone, Copy, Debug)]
pub struct ArmRef<'a> {
    tag: &'a str,
    schema: &'a Value,
}

/// The text under `key`, or `""`.
///
/// Presentation is optional, so an absent key is empty rather than an
/// error. A caller that needs to tell "absent" from "set to empty" asks
/// the value itself.
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    TryAsRef::<Map>::try_as_ref(v)
        .and_then(|m| m.get(key))
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or("")
}

/// Whether `v` is a map, which every part of a schema this crate writes is.
fn is_map(v: &Value) -> bool {
    TryAsRef::<Map>::try_as_ref(v).is_some()
}

/// Whether `v` is the boolean schema `true`, which accepts any value: what
/// a generator writes for a field of any JSON value.
pub(crate) fn is_true_schema(v: &Value) -> bool {
    v.tag() == Ok(Tag::GUATIAO_BOOL) && bool_or(Some(v), false)
}

/// Whether `v` can be read as a field's schema: a map, or `true`.
fn is_schema(v: &Value) -> bool {
    is_map(v) || is_true_schema(v)
}

/// Whether `owner`'s `required` list names `key`.
///
/// A linear scan, on purpose: these readers allocate nothing, and building
/// a set would. An object's field count is a handful, so the scan is
/// cheaper than the allocation would be.
fn is_required_in(owner: &Value, key: &str) -> bool {
    TryAsRef::<Map>::try_as_ref(owner)
        .and_then(|m| m.get(vocab::REQUIRED))
        .and_then(TryAsRef::<List>::try_as_ref)
        .map(|list| &list[..])
        .unwrap_or(&[])
        .iter()
        .filter_map(TryAsRef::<str>::try_as_ref)
        .any(|n| n == key)
}

/// Every field an object declares, in declaration order.
///
/// The one place that joins the two halves JSON Schema keeps apart: a name
/// from `properties`, a requiredness from `required`.
///
/// Anything whose name is not text, or whose schema is neither a map nor
/// `true`, is skipped — dropping the one is better than refusing the rest.
/// A property whose schema is `false` accepts no value, which is what
/// skipping it says: a value under that key is undeclared.
fn fields_of(owner: &Value) -> impl Iterator<Item = FieldRef<'_>> {
    TryAsRef::<Map>::try_as_ref(owner)
        .and_then(|m| m.get(vocab::PROPERTIES))
        .and_then(TryAsRef::<Map>::try_as_ref)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .filter_map(move |e| {
            let key = e.key();
            let schema = e.value();
            is_schema(schema).then(|| FieldRef {
                key,
                schema,
                required: is_required_in(owner, key),
            })
        })
}

impl<'a> SchemaRef<'a> {
    /// Views `value` as a schema, or `None` when it is not a map.
    pub fn new(value: &'a Value) -> Option<SchemaRef<'a>> {
        is_map(value).then_some(SchemaRef(value))
    }

    /// The value this is a view of.
    pub fn as_value(&self) -> &'a Value {
        self.0
    }

    /// The dialect this document declares, or `""`.
    ///
    /// [`vocab::DIALECT`] for anything this crate built. A schema that
    /// arrived from somewhere else may say something different or say
    /// nothing, and this crate reads it the same way either way — the
    /// keywords it uses are spelled identically in every dialect it could
    /// name.
    pub fn dialect(&self) -> &'a str {
        text(self.0, vocab::SCHEMA)
    }

    /// The schema's own `title`: a short line naming what it describes.
    pub fn title(&self) -> &'a str {
        text(self.0, vocab::TITLE)
    }

    /// The schema's own `description`: the longer prose under the title.
    pub fn description(&self) -> &'a str {
        text(self.0, vocab::DESCRIPTION)
    }

    /// Every field, in declaration order.
    pub fn fields(&self) -> impl Iterator<Item = FieldRef<'a>> {
        fields_of(self.0)
    }

    /// The field declared under `key`.
    ///
    /// Walks the entries rather than calling `get`, for the lifetime
    /// rather than the speed: a [`FieldRef`] borrows its name from the
    /// schema, and the caller's `key` does not live long enough to lend
    /// it. The cost is the same either way — a map here is an
    /// insertion-ordered association list, so `get` is a scan too.
    pub fn find(&self, key: &str) -> Option<FieldRef<'a>> {
        TryAsRef::<Map>::try_as_ref(self.0)
            .and_then(|m| m.get(vocab::PROPERTIES))
            .and_then(TryAsRef::<Map>::try_as_ref)
            .map(Map::entries)
            .unwrap_or(&[])
            .iter()
            .find(|e| e.key() == key)
            .and_then(|e| {
                let name = e.key();
                is_schema(e.value()).then(|| FieldRef {
                    key: name,
                    schema: e.value(),
                    required: is_required_in(self.0, name),
                })
            })
    }

    /// An annotation on the schema as a whole.
    ///
    /// Anything [`vocab::known`] does not claim is an annotation: carried,
    /// and never interpreted by this crate.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        TryAsRef::<Map>::try_as_ref(self.0)
            .and_then(|m| m.get(key))
            .filter(|_| !vocab::known(key))
    }

    /// Every annotation on the schema as a whole, in document order:
    /// each key [`vocab::known`] does not claim, with its value.
    pub fn extras(&self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        extras_of(self.0)
    }
}

/// The entries of a schema map that are annotations rather than
/// vocabulary, which is what [`SchemaRef::extras`] and
/// [`FieldRef::extras`] both walk.
fn extras_of(schema: &Value) -> impl Iterator<Item = (&str, &Value)> {
    TryAsRef::<Map>::try_as_ref(schema)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
        .filter(|(k, _)| !vocab::known(k))
}

impl<'a> FieldRef<'a> {
    /// Views `schema` as the field declared under `key`, or `None` when it
    /// is neither a map nor the schema `true`.
    ///
    /// **Says nothing about requiredness**, which belongs to the object
    /// this field sits in and is not readable from the field alone. A field
    /// made this way answers `false`; one from [`SchemaRef::fields`] or
    /// [`SchemaRef::find`] answers what the owner declared.
    pub fn new(key: &'a str, schema: &'a Value) -> Option<FieldRef<'a>> {
        is_schema(schema).then_some(FieldRef {
            key,
            schema,
            required: false,
        })
    }

    /// The field's schema, which is the value this is a view of.
    pub fn as_value(&self) -> &'a Value {
        self.schema
    }

    /// The key this field's value is stored under.
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// What this field accepts.
    ///
    /// The field's own schema, because JSON Schema has no separate place to
    /// put a kind.
    pub fn kind(&self) -> Kind<'a> {
        Kind::read(Some(self.schema))
    }

    /// Whether a null is a value this field accepts: the schema says
    /// `type: [T, "null"]` or `anyOf: [X, null]`, the shapes a generator
    /// writes for an optional value. [`kind`](FieldRef::kind) is the `T`
    /// either way. Beside an `enum` or a `const`, a `type` admitting null
    /// is not enough: null must also be one of the members.
    pub fn is_nullable(&self) -> bool {
        is_nullable(self.schema)
    }

    /// The field's `title`: what a person sees it called. May be empty.
    pub fn title(&self) -> &'a str {
        text(self.schema, vocab::TITLE)
    }

    /// The field's `description`: the longer prose. May be empty.
    pub fn description(&self) -> &'a str {
        text(self.schema, vocab::DESCRIPTION)
    }

    /// The default value, or `None` when the field has no default.
    ///
    /// **`None` and a default of null are different things**: the first
    /// means there is no default, the second that the default is nothing.
    pub fn default(&self) -> Option<&'a Value> {
        TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(vocab::DEFAULT))
    }

    /// A secret: never print this value.
    ///
    /// **The one `x-` key this crate reads by name**, and it earns that by
    /// not being a drawing instruction: a log, a crash dump and a debug
    /// print all obey it with no screen in sight. A hint that only a
    /// renderer acts on is read with [`extra`](FieldRef::extra) by
    /// whoever named it.
    pub fn is_sensitive(&self) -> bool {
        self.flag(vocab::X_SENSITIVE)
    }

    /// JSON Schema's `readOnly`: shown, never edited. Validation does not
    /// refuse a value for it; a renderer disables the control.
    pub fn is_read_only(&self) -> bool {
        self.flag(vocab::READ_ONLY)
    }

    /// JSON Schema's `deprecated`: kept for old documents, not offered for
    /// new ones.
    pub fn is_deprecated(&self) -> bool {
        self.flag(vocab::DEPRECATED)
    }

    /// JSON Schema's `examples`, in document order; empty when there are
    /// none. Not checked against the field's kind -- a renderer offering
    /// one should validate it like anything a person typed.
    pub fn examples(&self) -> &'a [Value] {
        TryAsRef::<Map>::try_as_ref(self.schema)
            .and_then(|m| m.get(vocab::EXAMPLES))
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .unwrap_or(&[])
    }

    /// The `examples` that are text, each with its label: the names an
    /// open enumeration offers without requiring one of them. Empty for
    /// a field with no text examples.
    pub fn suggestions(&self) -> impl Iterator<Item = ChoiceRef<'a>> {
        let labels = TryAsRef::<Map>::try_as_ref(self.schema)
            .and_then(|m| m.get(vocab::X_ENUM_LABELS))
            .and_then(TryAsRef::<Map>::try_as_ref);
        self.examples()
            .iter()
            .filter_map(TryAsRef::<str>::try_as_ref)
            .map(move |value| {
                let label = labels
                    .and_then(|m| m.get(value))
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or("");
                ChoiceRef { value, label }
            })
    }

    /// A boolean annotation: `true` only when written `true`.
    fn flag(&self, key: &str) -> bool {
        bool_or(
            TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(key)),
            false,
        )
    }

    /// Must be given.
    ///
    /// Read off the **owner's** `required` list when this view was made, so
    /// a field built by [`FieldRef::new`] alone answers `false`.
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// An annotation on this field. Carried, never interpreted.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        TryAsRef::<Map>::try_as_ref(self.schema)
            .and_then(|m| m.get(key))
            .filter(|_| !vocab::known(key))
    }

    /// Every annotation on this field, in document order -- what a
    /// consumer keeping its own mirror of a field copies across without
    /// having to know each key by name.
    pub fn extras(&self) -> impl Iterator<Item = (&'a str, &'a Value)> {
        extras_of(self.schema)
    }
}

impl<'a> ChoiceRef<'a> {
    /// What is stored when this alternative is chosen.
    pub fn value(&self) -> &'a str {
        self.value
    }
    /// What is shown. Falls back to the value, so a choice with no label
    /// still renders as something a person can read.
    pub fn label(&self) -> &'a str {
        if self.label.is_empty() {
            self.value
        } else {
            self.label
        }
    }
}

impl<'a> ArmRef<'a> {
    /// The discriminant stored when this arm is selected: the `const` its
    /// tag property pins.
    pub fn value(&self) -> &'a str {
        TryAsRef::<Map>::try_as_ref(self.schema)
            .and_then(|m| m.get(vocab::PROPERTIES))
            .and_then(TryAsRef::<Map>::try_as_ref)
            .and_then(|p| p.get(self.tag))
            .and_then(TryAsRef::<Map>::try_as_ref)
            .and_then(|t| t.get(vocab::CONST))
            .and_then(TryAsRef::<str>::try_as_ref)
            .unwrap_or("")
    }
    /// The arm's own schema.
    pub(crate) fn schema(&self) -> &'a Value {
        self.schema
    }
    /// The arm's `title`, falling back to its discriminant.
    pub fn title(&self) -> &'a str {
        let l = text(self.schema, vocab::TITLE);
        if l.is_empty() { self.value() } else { l }
    }
    /// The arm's `description`. May be empty.
    pub fn description(&self) -> &'a str {
        text(self.schema, vocab::DESCRIPTION)
    }
    /// The fields this arm adds when selected.
    ///
    /// **Without the discriminant**, which is a property of the arm in the
    /// document but is not a field somebody fills in — it is what selecting
    /// the arm means.
    ///
    /// **An empty result is ordinary and complete**, not missing data: "use
    /// the ambient credential" is the common case, and treating it as an
    /// error would make the common case the exception.
    pub fn fields(&self) -> impl Iterator<Item = FieldRef<'a>> {
        let tag = self.tag;
        fields_of(self.schema).filter(move |f| f.key() != tag)
    }
}

/// What a field accepts.
///
/// An exhaustive Rust enum over the value form, so a reader matches rather
/// than comparing strings. [`Kind::Unknown`] carries the name this build
/// did not recognise, which is what makes skipping one field possible
/// while every other field still renders.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Kind<'a> {
    /// Nothing: the only value is null. Reached as an arm of a
    /// [`Union`](Kind::Union); a field that is merely optional reads as
    /// the kind it holds, see [`FieldRef::is_nullable`].
    Null,
    /// True or false.
    Bool,
    /// A whole number, with optional inclusive bounds.
    Int {
        /// Inclusive lower bound.
        min: Option<i64>,
        /// Inclusive upper bound.
        max: Option<i64>,
        /// The number must be a multiple of this: `multipleOf`, greater
        /// than zero when present.
        multiple_of: Option<f64>,
    },
    /// A real number, with optional inclusive bounds.
    Float {
        /// Inclusive lower bound.
        min: Option<f64>,
        /// Inclusive upper bound.
        max: Option<f64>,
        /// The number must be a multiple of this: `multipleOf`, greater
        /// than zero when present.
        multiple_of: Option<f64>,
    },
    /// Free text, with what JSON Schema can say about its shape.
    Str {
        /// `minLength`: fewest characters, in Unicode code points.
        min_length: Option<u64>,
        /// `maxLength`: most characters, in Unicode code points.
        max_length: Option<u64>,
        /// `pattern`: a regular expression the text must match,
        /// unanchored.
        pattern: Option<&'a str>,
        /// `format`: what the text holds (`email`, `uri`, ...). An
        /// annotation -- never checked.
        format: Option<&'a str>,
    },
    /// Opaque bytes: a certificate, a ticket, a key.
    Bytes,
    /// A sequence, every element of one kind.
    List {
        /// The element schema; [`Kind::items`] reads it as a kind.
        items: &'a Value,
        /// `minItems`: fewest elements.
        min: Option<u64>,
        /// `maxItems`: most elements.
        max: Option<u64>,
    },
    /// A nested object, with its own fields. Carries the **object's**
    /// schema, not its `properties`, because `required` sits beside them.
    Map(&'a Value),
    /// An object whose **keys are data**: any number of them, every value
    /// of one kind. Carries the object's schema, whose
    /// `additionalProperties` is the value schema.
    ///
    /// Told apart from [`Map`](Kind::Map) by having no `properties`: an
    /// object that declares some keys is a declaration, and one that
    /// declares none but says what its values look like is this. A
    /// document carrying both reads as a `Map`, because the declared half
    /// is the half a reader can act on.
    MapOf(&'a Value),
    /// Exactly one of a fixed set of values, of any type. Carries the
    /// schema listing them: an `enum`, a single `const`, or a `oneOf` whose
    /// every arm is a `const` (each arm's `title` is that choice's label),
    /// with the labels of an `enum` in `x-enum-labels`.
    ///
    /// A value is accepted when it equals a member, and
    /// [`choices`](Kind::choices) lists the members that have a text
    /// spelling: the strings and the numbers.
    Enum(&'a Value),
    /// Any one of several kinds. **Untagged**: validation succeeds if any
    /// arm accepts, and which arm took the value is not the point. Carries
    /// the `anyOf` list.
    Union(&'a Value),
    /// One of several alternatives, each with its own fields.
    /// **Tagged**: the discriminant is stored, so knowing the arm is the
    /// point.
    Variant {
        /// The key the discriminant is stored under.
        tag: &'a str,
        /// The arms, as the `oneOf` list holding them.
        arms: &'a Value,
    },
    /// A kind this build does not know, by name, or a schema that
    /// constrains nothing it reads: `{}`, `true`, a `description` alone,
    /// an `allOf` of several schemas, a `$ref` not yet inlined. **Skip the
    /// field; do not reject the schema.** Validation accepts any value.
    Unknown(&'a str),
    /// Not a schema at all: no value, or one that is neither a map nor
    /// `true` -- also what [`items`](Kind::items) and
    /// [`values`](Kind::values) answer for another kind. Never the
    /// reading of a map.
    Missing,
}

impl<'a> Kind<'a> {
    /// The kind `schema` declares, or [`Kind::Missing`] when it is neither
    /// a map nor `true`.
    pub(crate) fn read(schema: Option<&'a Value>) -> Kind<'a> {
        let Some(k) = schema else {
            return Kind::Missing;
        };
        // An optional value reads as the value it holds, and a lone
        // `allOf` member as itself; whether null is accepted too is
        // `is_nullable`'s question.
        let k = peel(k);
        if is_true_schema(k) {
            return Kind::Unknown(TRUE_SCHEMA);
        }
        let Some(m) = TryAsRef::<Map>::try_as_ref(k) else {
            return Kind::Missing;
        };
        // A reference this view cannot follow: skip the field, never
        // refuse the schema.
        if m.get(vocab::REF).is_some() {
            return Kind::Unknown(vocab::REF);
        }
        let declared = declared_type(k);
        // An enumeration narrows whatever type it sits on, so it is read
        // first: `{"type":"string","enum":[...]}` is a choice, not free
        // text. A tagged variant's `oneOf` is arms, not choices.
        if is_enumeration(k) {
            return Kind::Enum(k);
        }
        let ty = match declared {
            Declared::Named(ty) => ty,
            Declared::Several => return Kind::Unknown(vocab::TYPE),
            Declared::Absent => {
                // No `type` is not automatically malformed: `anyOf` alone
                // is what an untagged union is.
                return if let Some(any_of) = m.get(vocab::ANY_OF) {
                    Kind::Union(any_of)
                } else if m.get(vocab::ONE_OF).is_some() {
                    // Arms that are not all constants are a shape this
                    // build does not read.
                    Kind::Unknown(vocab::ONE_OF)
                } else if m.get(vocab::ALL_OF).is_some() {
                    Kind::Unknown(vocab::ALL_OF)
                } else {
                    // Nothing that constrains a value: `{}` or prose
                    // alone, which JSON Schema reads as accepting any.
                    Kind::Unknown(vocab::TYPE)
                };
            }
        };
        match ty {
            vocab::TYPE_NULL => Kind::Null,
            vocab::TYPE_BOOLEAN => Kind::Bool,
            vocab::TYPE_STRING => Kind::Str {
                min_length: opt_count(k, vocab::MIN_LENGTH),
                max_length: opt_count(k, vocab::MAX_LENGTH),
                pattern: opt_str(k, vocab::PATTERN),
                format: opt_str(k, vocab::FORMAT),
            },
            vocab::TYPE_BYTES => Kind::Bytes,
            // An array with no element schema says nothing about what it
            // holds, which is a kind this build cannot use rather than an
            // array of anything.
            vocab::TYPE_ARRAY => match m.get(vocab::ITEMS) {
                Some(items) => Kind::List {
                    items,
                    min: opt_count(k, vocab::MIN_ITEMS),
                    max: opt_count(k, vocab::MAX_ITEMS),
                },
                None => Kind::Unknown(ty),
            },
            // An object with no properties is ordinary and complete, the
            // same way an arm with no fields is: it is an object nothing
            // further is declared about. With a tag it is a variant, and a
            // variant with no arms cannot be selected from.
            vocab::TYPE_OBJECT => match m
                .get(vocab::X_VARIANT_TAG)
                .and_then(TryAsRef::<str>::try_as_ref)
            {
                Some(tag) => match m.get(vocab::ONE_OF) {
                    Some(arms) => Kind::Variant { tag, arms },
                    None => Kind::Unknown(ty),
                },
                None => {
                    let declares_keys = m.get(vocab::PROPERTIES).is_some();
                    let values_declared = m.get(vocab::ADDITIONAL_PROPERTIES).is_some_and(is_map);
                    if !declares_keys && values_declared {
                        Kind::MapOf(k)
                    } else {
                        Kind::Map(k)
                    }
                }
            },
            vocab::TYPE_INTEGER => Kind::Int {
                min: opt_bound(k, vocab::MINIMUM, Round::Up),
                max: opt_bound(k, vocab::MAXIMUM, Round::Down),
                multiple_of: opt_step(k),
            },
            vocab::TYPE_NUMBER => Kind::Float {
                min: opt_float(k, vocab::MINIMUM),
                max: opt_float(k, vocab::MAXIMUM),
                multiple_of: opt_step(k),
            },
            other => Kind::Unknown(other),
        }
    }

    /// The alternatives of an enumeration, or nothing for any other kind.
    ///
    /// Only the members with a text spelling appear: strings, and numbers
    /// as the text that declared them. A member of another type is still
    /// accepted by validation and has no choice to show.
    pub fn choices(self) -> impl Iterator<Item = ChoiceRef<'a>> {
        let (owner, labels) = match self {
            Kind::Enum(k) => (
                Some(k),
                TryAsRef::<Map>::try_as_ref(k)
                    .and_then(|m| m.get(vocab::X_ENUM_LABELS))
                    .and_then(TryAsRef::<Map>::try_as_ref),
            ),
            _ => (None, None),
        };
        owner
            .into_iter()
            .flat_map(members)
            .filter_map(move |(member, title)| {
                let value = member_text(member)?;
                let label = labels
                    .and_then(|m| m.get(value))
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .unwrap_or(title);
                Some(ChoiceRef { value, label })
            })
    }

    /// The arms of a **union**, as kinds.
    ///
    /// Empty for a variant, deliberately. The two are different features
    /// and a reader that treated a variant's arms as a union's would draw
    /// a selector whose selection stores a bare discriminant with no
    /// payload.
    pub fn alternatives(self) -> impl Iterator<Item = Kind<'a>> {
        let list = match self {
            Kind::Union(a) => TryAsRef::<List>::try_as_ref(a)
                .map(|list| &list[..])
                .unwrap_or(&[]),
            _ => &[],
        };
        list.iter().map(|v| Kind::read(Some(v)))
    }

    /// The arms of a **variant**, each with its own fields.
    pub fn arms(self) -> impl Iterator<Item = ArmRef<'a>> {
        let (tag, list) = match self {
            Kind::Variant { tag, arms } => (
                tag,
                TryAsRef::<List>::try_as_ref(arms)
                    .map(|list| &list[..])
                    .unwrap_or(&[]),
            ),
            _ => ("", &[][..]),
        };
        list.iter()
            .filter(|v| is_map(v))
            .map(move |schema| ArmRef { tag, schema })
    }

    /// The kind every element of a list has, or [`Kind::Missing`] for
    /// any other kind.
    pub fn items(self) -> Kind<'a> {
        match self {
            Kind::List { items, .. } => Kind::read(Some(items)),
            _ => Kind::Missing,
        }
    }

    /// The kind every value of an open object has, or [`Kind::Missing`]
    /// for any other kind.
    ///
    /// The map counterpart of [`items`](Kind::items): a
    /// [`MapOf`](Kind::MapOf) says what its values are and says nothing
    /// about its keys, because its keys are data.
    pub fn values(self) -> Kind<'a> {
        match self {
            Kind::MapOf(k) => Kind::read(
                TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(vocab::ADDITIONAL_PROPERTIES)),
            ),
            _ => Kind::Missing,
        }
    }

    /// The fields of a nested object, or nothing for any other kind.
    ///
    /// Reads exactly like [`ArmRef::fields`], because it is the same idea:
    /// here are more fields, keyed under something.
    pub fn fields(self) -> impl Iterator<Item = FieldRef<'a>> {
        let owner = match self {
            Kind::Map(k) => Some(k),
            _ => None,
        };
        owner.into_iter().flat_map(fields_of)
    }

    /// The name this kind is written with, for a diagnostic.
    pub fn name(self) -> &'a str {
        match self {
            Kind::Null => vocab::TYPE_NULL,
            Kind::Bool => vocab::TYPE_BOOLEAN,
            Kind::Int { .. } => vocab::TYPE_INTEGER,
            Kind::Float { .. } => vocab::TYPE_NUMBER,
            Kind::Str { .. } => vocab::TYPE_STRING,
            Kind::Enum(k) => match declared_type(k) {
                Declared::Named(ty) => ty,
                _ => vocab::TYPE_STRING,
            },
            Kind::Bytes => vocab::TYPE_BYTES,
            Kind::List { .. } => vocab::TYPE_ARRAY,
            Kind::Map(_) | Kind::MapOf(_) | Kind::Variant { .. } => vocab::TYPE_OBJECT,
            Kind::Union(_) => vocab::ANY_OF,
            Kind::Unknown(name) => name,
            Kind::Missing => "",
        }
    }
}

/// What [`Kind::Unknown`] names for the schema `true`.
const TRUE_SCHEMA: &str = "true";

/// Which way a fractional bound moves to land on a whole number that
/// accepts exactly the integers the bound does.
#[derive(Clone, Copy)]
enum Round {
    /// A lower bound: `minimum: 1.5` admits 2 and up.
    Up,
    /// An upper bound: `maximum: 9.5` admits 9 and down.
    Down,
}

/// A bound on an integer, rounded inward when it is not whole.
///
/// Exact for a plain integer, whatever its width up to `i64`; a
/// fractional or exponent spelling (`1.5`, `1e1`) goes through `f64`.
/// A bound outside `i64` is no bound this build can apply, and silently
/// clamping it would enforce a limit nobody declared.
fn opt_bound(k: &Value, key: &str, round: Round) -> Option<i64> {
    let v = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(key))?;
    let text: &str = TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)?;
    if let Ok(n) = text.parse::<i64>() {
        return Some(n);
    }
    let x: f64 = text.parse().ok().filter(|x: &f64| x.is_finite())?;
    let x = match round {
        Round::Up => x.ceil(),
        Round::Down => x.floor(),
    };
    // `i64::MAX as f64` is 2^63, which is itself out of range.
    (x >= i64::MIN as f64 && x < i64::MAX as f64).then_some(x as i64)
}

/// A count JSON Schema requires to be a non-negative integer. Anything
/// else -- negative, fractional, past `i64` -- is no count this build can
/// apply, and is read as absent rather than guessed at.
fn opt_count(k: &Value, key: &str) -> Option<u64> {
    let v = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(key))?;
    let text: &str = TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)?;
    text.parse::<u64>().ok()
}

/// `multipleOf`, which JSON Schema requires to be greater than zero. A
/// step of zero or less divides nothing, so it is read as absent.
fn opt_step(k: &Value) -> Option<f64> {
    opt_float(k, vocab::MULTIPLE_OF).filter(|s| *s > 0.0)
}

fn opt_str<'a>(k: &'a Value, key: &str) -> Option<&'a str> {
    TryAsRef::<Map>::try_as_ref(k)
        .and_then(|m| m.get(key))
        .and_then(TryAsRef::<str>::try_as_ref)
}

fn opt_float(k: &Value, key: &str) -> Option<f64> {
    let v = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(key))?;
    TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)?;
    let got = float_or(Some(v), f64::NAN);
    got.is_finite().then_some(got)
}

/// What a schema's `type` says.
enum Declared<'a> {
    /// One type, possibly beside `"null"` in a list.
    Named(&'a str),
    /// More than one non-null type, or something that is not a type.
    Several,
    /// No `type` at all.
    Absent,
}

fn declared_type(k: &Value) -> Declared<'_> {
    let Some(v) = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(vocab::TYPE)) else {
        return Declared::Absent;
    };
    if let Some(name) = TryAsRef::<str>::try_as_ref(v) {
        return Declared::Named(name);
    }
    let Some(list) = TryAsRef::<List>::try_as_ref(v) else {
        return Declared::Several;
    };
    let mut only: Option<&str> = None;
    for entry in list.iter() {
        let Some(name) = TryAsRef::<str>::try_as_ref(entry) else {
            return Declared::Several;
        };
        if name == vocab::TYPE_NULL {
            continue;
        }
        if only.replace(name).is_some() {
            return Declared::Several;
        }
    }
    match only {
        Some(name) => Declared::Named(name),
        None if list.is_empty() => Declared::Several,
        None => Declared::Named(vocab::TYPE_NULL),
    }
}

/// Whether `v` is `{"type": "null"}` or `{"const": null}`.
fn is_null_schema(v: &Value) -> bool {
    let Some(m) = TryAsRef::<Map>::try_as_ref(v) else {
        return false;
    };
    m.get(vocab::TYPE)
        .and_then(TryAsRef::<str>::try_as_ref)
        .is_some_and(|t| t == vocab::TYPE_NULL)
        || m.get(vocab::CONST)
            .is_some_and(|c| c.tag() == Ok(Tag::GUATIAO_NULL))
}

/// The other arm of an `anyOf` that is exactly a schema and null: how a
/// generator spells an optional value.
fn nullable_pair(any_of: &Value) -> Option<&Value> {
    let list = TryAsRef::<List>::try_as_ref(any_of)?;
    let [a, b] = &list[..] else {
        return None;
    };
    match (is_null_schema(a), is_null_schema(b)) {
        (true, false) if is_map(b) => Some(b),
        (false, true) if is_map(a) => Some(a),
        _ => None,
    }
}

/// The one member of an `allOf` that is a wrapper and nothing more: a
/// single schema, on a map that names no `type`, `enum`, `const`, `anyOf`
/// or `oneOf` of its own beside it.
fn lone_all_of(m: &Map) -> Option<&Value> {
    if [
        vocab::TYPE,
        vocab::ENUM,
        vocab::CONST,
        vocab::ANY_OF,
        vocab::ONE_OF,
    ]
    .iter()
    .any(|key| m.get(key).is_some())
    {
        return None;
    }
    let list = m
        .get(vocab::ALL_OF)
        .and_then(TryAsRef::<List>::try_as_ref)?;
    match &list[..] {
        [only] if is_schema(only) => Some(only),
        _ => None,
    }
}

/// `k` with every wrapper that only says "this, or null" or "this, with a
/// description" removed: `anyOf: [X, null]` and a lone `allOf: [X]`,
/// however deep. A loop, because the schema is not ours to bound.
pub(crate) fn peel(mut k: &Value) -> &Value {
    loop {
        let Some(m) = TryAsRef::<Map>::try_as_ref(k) else {
            return k;
        };
        let inner = if m.get(vocab::TYPE).is_none() {
            m.get(vocab::ANY_OF).and_then(nullable_pair)
        } else {
            None
        };
        match inner.or_else(|| lone_all_of(m)) {
            Some(inner) => k = inner,
            None => return k,
        }
    }
}

/// Whether a null is a value this schema accepts: `type: [T, "null"]`
/// (with null a member, when an `enum` or `const` sits beside it), or
/// `anyOf: [X, null]`, at any level of the wrappers [`peel`] removes.
pub(crate) fn is_nullable(schema: &Value) -> bool {
    let mut k = schema;
    loop {
        let Some(m) = TryAsRef::<Map>::try_as_ref(k) else {
            return false;
        };
        if let Some(ty) = m.get(vocab::TYPE) {
            let Some(list) = TryAsRef::<List>::try_as_ref(ty) else {
                return false;
            };
            let admits_null = list
                .iter()
                .filter_map(TryAsRef::<str>::try_as_ref)
                .any(|t| t == vocab::TYPE_NULL);
            return admits_null
                && (!is_enumeration(k)
                    || members(k).any(|(member, _)| member.tag() == Ok(Tag::GUATIAO_NULL)));
        }
        if m.get(vocab::ANY_OF).and_then(nullable_pair).is_some() {
            return true;
        }
        match lone_all_of(m) {
            Some(inner) => k = inner,
            None => return false,
        }
    }
}

/// Whether `k` lists a fixed set of values: an `enum`, a `const`, or a
/// `oneOf` of constants that is not a tagged variant's arms.
pub(crate) fn is_enumeration(k: &Value) -> bool {
    let Some(m) = TryAsRef::<Map>::try_as_ref(k) else {
        return false;
    };
    if m.get(vocab::ENUM)
        .is_some_and(|e| TryAsRef::<List>::try_as_ref(e).is_some())
        || m.get(vocab::CONST).is_some()
    {
        return true;
    }
    m.get(vocab::X_VARIANT_TAG).is_none()
        && m.get(vocab::ONE_OF)
            .and_then(TryAsRef::<List>::try_as_ref)
            .is_some_and(|arms| {
                !arms.is_empty()
                    && arms.iter().all(|arm| {
                        TryAsRef::<Map>::try_as_ref(arm)
                            .is_some_and(|a| a.get(vocab::CONST).is_some())
                    })
            })
}

/// The members of an enumeration, each with the `title` its `oneOf` arm
/// gave it (`""` for an `enum` entry or a `const`).
pub(crate) fn members(k: &Value) -> impl Iterator<Item = (&Value, &str)> {
    let m = TryAsRef::<Map>::try_as_ref(k);
    let listed = m
        .and_then(|m| m.get(vocab::ENUM))
        .and_then(TryAsRef::<List>::try_as_ref)
        .map(|list| &list[..]);
    let single = m
        .and_then(|m| m.get(vocab::CONST))
        .filter(|_| listed.is_none());
    let arms = m
        .and_then(|m| m.get(vocab::ONE_OF))
        .and_then(TryAsRef::<List>::try_as_ref)
        .map(|list| &list[..])
        .filter(|_| listed.is_none() && single.is_none());
    listed
        .into_iter()
        .flatten()
        .map(|v| (v, ""))
        .chain(single.map(|v| (v, "")))
        .chain(arms.into_iter().flatten().filter_map(|arm| {
            let constant = TryAsRef::<Map>::try_as_ref(arm).and_then(|a| a.get(vocab::CONST))?;
            Some((constant, text(arm, vocab::TITLE)))
        }))
}

/// A member's text spelling: a string, or a number's own text.
fn member_text(member: &Value) -> Option<&str> {
    TryAsRef::<str>::try_as_ref(member)
        .or_else(|| TryAsRef::<Number>::try_as_ref(member).map(AsRef::<str>::as_ref))
}

/// Every entry of a map, as `(key, value)`, for a caller walking
/// annotations.
pub fn annotations(v: &Value) -> impl Iterator<Item = (&str, &Value)> {
    TryAsRef::<Map>::try_as_ref(v)
        .map(Map::entries)
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
        .filter(|(k, _)| !vocab::known(k))
}
