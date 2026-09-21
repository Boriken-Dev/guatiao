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
use crate::value::read::{bool_or, float_or, int_or};
use crate::value::types::{List, Map, Number, Value};

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

/// Whether `v` is a map, which every part of a schema is.
fn is_map(v: &Value) -> bool {
    TryAsRef::<Map>::try_as_ref(v).is_some()
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
/// Anything whose name is not text, or whose schema is not a map, is
/// skipped — dropping the one is better than refusing the rest.
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
            is_map(schema).then(|| FieldRef {
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

    /// A short human label for the schema as a whole: its `title`.
    pub fn label(&self) -> &'a str {
        text(self.0, vocab::TITLE)
    }

    /// Longer human help for the schema as a whole: its `description`.
    pub fn help(&self) -> &'a str {
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
                is_map(e.value()).then(|| FieldRef {
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
    /// is not a map.
    ///
    /// **Says nothing about requiredness**, which belongs to the object
    /// this field sits in and is not readable from the field alone. A field
    /// made this way answers `false`; one from [`SchemaRef::fields`] or
    /// [`SchemaRef::find`] answers what the owner declared.
    pub fn new(key: &'a str, schema: &'a Value) -> Option<FieldRef<'a>> {
        is_map(schema).then_some(FieldRef {
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

    /// A short human label: the field's `title`. May be empty.
    pub fn label(&self) -> &'a str {
        text(self.schema, vocab::TITLE)
    }

    /// Longer human help: the field's `description`. May be empty.
    pub fn help(&self) -> &'a str {
        text(self.schema, vocab::DESCRIPTION)
    }

    /// Which section this belongs to. Empty means the default one.
    pub fn section(&self) -> &'a str {
        text(self.schema, vocab::X_SECTION)
    }

    /// The default value, or `None` when the field has no default.
    ///
    /// **`None` and a default of null are different things**: the first
    /// means there is no default, the second that the default is nothing.
    pub fn default(&self) -> Option<&'a Value> {
        TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(vocab::DEFAULT))
    }

    /// Declaration position. 0 when unset, which a consumer should read as
    /// "no ordering information" rather than "first".
    pub fn order(&self) -> i64 {
        int_or(
            TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(vocab::X_ORDER)),
            0,
        )
    }

    /// Hidden behind a disclosure by default.
    pub fn is_advanced(&self) -> bool {
        bool_or(
            TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(vocab::X_ADVANCED)),
            false,
        )
    }

    /// A secret: masked in a form, and not somewhere to put in a log.
    pub fn is_sensitive(&self) -> bool {
        bool_or(
            TryAsRef::<Map>::try_as_ref(self.schema).and_then(|m| m.get(vocab::X_SENSITIVE)),
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
    /// What is shown. Falls back to the value.
    pub fn label(&self) -> &'a str {
        let l = text(self.schema, vocab::TITLE);
        if l.is_empty() { self.value() } else { l }
    }
    /// Longer human help. May be empty.
    pub fn help(&self) -> &'a str {
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
    /// True or false.
    Bool,
    /// A whole number, with optional inclusive bounds.
    Int {
        /// Inclusive lower bound.
        min: Option<i64>,
        /// Inclusive upper bound.
        max: Option<i64>,
    },
    /// A real number, with optional inclusive bounds.
    Float {
        /// Inclusive lower bound.
        min: Option<f64>,
        /// Inclusive upper bound.
        max: Option<f64>,
    },
    /// Free text.
    Str,
    /// Opaque bytes: a certificate, a ticket, a key.
    Bytes,
    /// A sequence, every element of one kind. Carries the element schema.
    List(&'a Value),
    /// A nested object, with its own fields. Carries the **object's**
    /// schema, not its `properties`, because `required` sits beside them.
    Map(&'a Value),
    /// Exactly one of a fixed set of alternatives. Carries the schema
    /// holding both `enum` and `x-labels`.
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
    /// A kind this build does not know, by name. **Skip the field; do not
    /// reject the schema.**
    Unknown(&'a str),
    /// No kind was declared at all, which is a malformed field rather
    /// than a new one.
    Missing,
}

impl<'a> Kind<'a> {
    fn read(schema: Option<&'a Value>) -> Kind<'a> {
        let Some(k) = schema.filter(|v| is_map(v)) else {
            return Kind::Missing;
        };
        let Some(ty) = TryAsRef::<Map>::try_as_ref(k)
            .and_then(|m| m.get(vocab::TYPE))
            .and_then(TryAsRef::<str>::try_as_ref)
        else {
            // No `type` is not automatically malformed: `anyOf` alone is
            // what an untagged union is, and a bare `enum` is a schema
            // somebody else wrote that still says exactly what it accepts.
            return if let Some(any_of) =
                TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(vocab::ANY_OF))
            {
                Kind::Union(any_of)
            } else if TryAsRef::<Map>::try_as_ref(k)
                .and_then(|m| m.get(vocab::ENUM))
                .is_some()
            {
                Kind::Enum(k)
            } else {
                Kind::Missing
            };
        };
        match ty {
            vocab::TYPE_BOOLEAN => Kind::Bool,
            // An `enum` narrows the type it sits on, so it is read first:
            // `{"type":"string","enum":[…]}` is a choice, not free text.
            vocab::TYPE_STRING
                if TryAsRef::<Map>::try_as_ref(k)
                    .and_then(|m| m.get(vocab::ENUM))
                    .is_some() =>
            {
                Kind::Enum(k)
            }
            vocab::TYPE_STRING => Kind::Str,
            vocab::TYPE_BYTES => Kind::Bytes,
            // An array with no element schema says nothing about what it
            // holds, which is a kind this build cannot use rather than an
            // array of anything.
            vocab::TYPE_ARRAY => {
                match TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(vocab::ITEMS)) {
                    Some(i) => Kind::List(i),
                    None => Kind::Unknown(ty),
                }
            }
            // An object with no properties is ordinary and complete, the
            // same way an arm with no fields is: it is an object nothing
            // further is declared about. With a tag it is a variant, and a
            // variant with no arms cannot be selected from.
            vocab::TYPE_OBJECT => match TryAsRef::<Map>::try_as_ref(k)
                .and_then(|m| m.get(vocab::X_VARIANT_TAG))
                .and_then(TryAsRef::<str>::try_as_ref)
            {
                Some(tag) => {
                    match TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(vocab::ONE_OF)) {
                        Some(arms) => Kind::Variant { tag, arms },
                        None => Kind::Unknown(ty),
                    }
                }
                None => Kind::Map(k),
            },
            vocab::TYPE_INTEGER => Kind::Int {
                min: opt_int(k, vocab::MINIMUM),
                max: opt_int(k, vocab::MAXIMUM),
            },
            vocab::TYPE_NUMBER => Kind::Float {
                min: opt_float(k, vocab::MINIMUM),
                max: opt_float(k, vocab::MAXIMUM),
            },
            other => Kind::Unknown(other),
        }
    }

    /// The alternatives of an enumeration, or nothing for any other kind.
    pub fn choices(self) -> impl Iterator<Item = ChoiceRef<'a>> {
        let (values, labels): (&[Value], Option<&Map>) = match self {
            Kind::Enum(k) => (
                TryAsRef::<Map>::try_as_ref(k)
                    .and_then(|m| m.get(vocab::ENUM))
                    .and_then(TryAsRef::<List>::try_as_ref)
                    .map(|list| &list[..])
                    .unwrap_or(&[]),
                TryAsRef::<Map>::try_as_ref(k)
                    .and_then(|m| m.get(vocab::X_ENUM_LABELS))
                    .and_then(TryAsRef::<Map>::try_as_ref),
            ),
            _ => (&[][..], None),
        };
        values
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
            Kind::List(i) => Kind::read(Some(i)),
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
            Kind::Bool => vocab::TYPE_BOOLEAN,
            Kind::Int { .. } => vocab::TYPE_INTEGER,
            Kind::Float { .. } => vocab::TYPE_NUMBER,
            Kind::Str | Kind::Enum(_) => vocab::TYPE_STRING,
            Kind::Bytes => vocab::TYPE_BYTES,
            Kind::List(_) => vocab::TYPE_ARRAY,
            Kind::Map(_) | Kind::Variant { .. } => vocab::TYPE_OBJECT,
            Kind::Union(_) => vocab::ANY_OF,
            Kind::Unknown(name) => name,
            Kind::Missing => "",
        }
    }
}

fn opt_int(k: &Value, key: &str) -> Option<i64> {
    let v = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(key))?;
    // A bound written outside `i64` is no bound this build can apply, and
    // silently clamping it would enforce a limit nobody declared.
    TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)?;
    let got = int_or(Some(v), i64::MIN);
    (got != i64::MIN
        || TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)
            == Some("-9223372036854775808"))
    .then_some(got)
}

fn opt_float(k: &Value, key: &str) -> Option<f64> {
    let v = TryAsRef::<Map>::try_as_ref(k).and_then(|m| m.get(key))?;
    TryAsRef::<Number>::try_as_ref(v).map(AsRef::<str>::as_ref)?;
    let got = float_or(Some(v), f64::NAN);
    got.is_finite().then_some(got)
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
