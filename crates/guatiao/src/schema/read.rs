// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a schema out of a value.
//!
//! A schema is a map written with the keys in [`super::vocab`]. These are
//! typed views over one: they borrow, allocate nothing, and hand a Rust
//! caller an exhaustive [`Kind`] to match on instead of a pile of string
//! comparisons.
//!
//! # Skipping is the rule, not an error path
//!
//! A reader that meets something it does not understand skips that one
//! thing and carries on. An option with no key is not an option, so
//! [`SchemaRef::options`] passes over it; a kind whose `type` this build
//! does not know arrives as [`Kind::Unknown`] so the option's key, label
//! and help still render.
//!
//! Refusing the whole schema instead would hide every option that was
//! perfectly readable, which is the opposite of what forward
//! compatibility is for.

#![forbid(unsafe_code)]

use super::vocab;
use crate::value::read::{bool_or, float_or, int_or};
use crate::value::types::{Tag, Value};

/// A schema: what a provider needs to be initialised.
#[derive(Clone, Copy, Debug)]
pub struct SchemaRef<'a>(&'a Value);

/// One option a provider declares.
#[derive(Clone, Copy, Debug)]
pub struct OptionRef<'a>(&'a Value);

/// A named group of options, for a consumer that draws them.
#[derive(Clone, Copy, Debug)]
pub struct SectionRef<'a>(&'a Value);

/// One alternative of an enum.
#[derive(Clone, Copy, Debug)]
pub struct ChoiceRef<'a>(&'a Value);

/// One arm of a tagged kind.
#[derive(Clone, Copy, Debug)]
pub struct ArmRef<'a>(&'a Value);

/// The text under `key`, or `""`.
///
/// Presentation fields are optional, so an absent one is empty rather than
/// an error. A caller that needs to tell "absent" from "set to empty" asks
/// the value itself.
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Whether `v` is a map, which every part of a schema is.
fn is_map(v: &Value) -> bool {
    v.tag() == Ok(Tag::GUATIAO_MAP)
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

    /// Every option, in declaration order.
    ///
    /// Anything in the list that is not a map with a string key is skipped:
    /// an option with no key is not an option, and dropping the one is
    /// better than refusing the rest.
    pub fn options(&self) -> impl Iterator<Item = OptionRef<'a>> {
        self.0
            .get(vocab::OPTIONS)
            .and_then(Value::items)
            .unwrap_or(&[])
            .iter()
            .filter_map(OptionRef::new)
    }

    /// Every section, in declaration order.
    pub fn sections(&self) -> impl Iterator<Item = SectionRef<'a>> {
        self.0
            .get(vocab::SECTIONS)
            .and_then(Value::items)
            .unwrap_or(&[])
            .iter()
            .filter(|v| is_map(v))
            .map(SectionRef)
    }

    /// The option declared under `key`.
    pub fn find(&self, key: &str) -> Option<OptionRef<'a>> {
        self.options().find(|o| o.key() == key)
    }

    /// An annotation on the schema as a whole.
    ///
    /// Anything outside [`vocab::KEYWORDS`] is an annotation: carried, and
    /// never interpreted by this crate.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|_| !vocab::KEYWORDS.contains(&key))
    }
}

impl<'a> SectionRef<'a> {
    /// The identifier an option's `section` refers to.
    pub fn id(&self) -> &'a str {
        text(self.0, vocab::ID)
    }
    /// A short human label. May be empty.
    pub fn label(&self) -> &'a str {
        text(self.0, vocab::LABEL)
    }
    /// Longer human help. May be empty.
    pub fn help(&self) -> &'a str {
        text(self.0, vocab::HELP)
    }
}

impl<'a> OptionRef<'a> {
    /// Views `value` as an option, or `None` when it is not a map with a
    /// string key.
    pub fn new(value: &'a Value) -> Option<OptionRef<'a>> {
        if !is_map(value) {
            return None;
        }
        value.get(vocab::KEY).and_then(Value::as_str)?;
        Some(OptionRef(value))
    }

    /// The value this is a view of.
    pub fn as_value(&self) -> &'a Value {
        self.0
    }

    /// The key this option's value is stored under. Never empty: an option
    /// without one is not constructible through [`OptionRef::new`].
    pub fn key(&self) -> &'a str {
        text(self.0, vocab::KEY)
    }

    /// What this option accepts.
    pub fn kind(&self) -> Kind<'a> {
        Kind::read(self.0.get(vocab::KIND))
    }

    /// A short human label. May be empty.
    pub fn label(&self) -> &'a str {
        text(self.0, vocab::LABEL)
    }

    /// Longer human help. May be empty.
    pub fn help(&self) -> &'a str {
        text(self.0, vocab::HELP)
    }

    /// Which section this belongs to. Empty means the default one.
    pub fn section(&self) -> &'a str {
        text(self.0, vocab::SECTION)
    }

    /// The default value, or `None` when the option has no default.
    ///
    /// **`None` and a default of null are different things**: the first
    /// means there is no default, the second that the default is nothing.
    pub fn default(&self) -> Option<&'a Value> {
        self.0.get(vocab::DEFAULT)
    }

    /// Declaration position. 0 when unset, which a consumer should read as
    /// "no ordering information" rather than "first".
    pub fn order(&self) -> i64 {
        int_or(self.0.get(vocab::ORDER), 0)
    }

    /// Hidden behind a disclosure by default.
    pub fn is_advanced(&self) -> bool {
        bool_or(self.0.get(vocab::ADVANCED), false)
    }

    /// A secret: masked in a form, encrypted in storage.
    pub fn is_sensitive(&self) -> bool {
        bool_or(self.0.get(vocab::SENSITIVE), false)
    }

    /// Must be given.
    pub fn is_required(&self) -> bool {
        bool_or(self.0.get(vocab::REQUIRED), false)
    }

    /// An annotation on this option. Carried, never interpreted.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        self.0.get(key).filter(|_| !vocab::KEYWORDS.contains(&key))
    }
}

impl<'a> ChoiceRef<'a> {
    /// What is stored when this alternative is chosen.
    pub fn value(&self) -> &'a str {
        text(self.0, vocab::VALUE)
    }
    /// What is shown. Falls back to the value, so a choice with no label
    /// still renders as something a person can read.
    pub fn label(&self) -> &'a str {
        let l = text(self.0, vocab::LABEL);
        if l.is_empty() { self.value() } else { l }
    }
}

impl<'a> ArmRef<'a> {
    /// The discriminant stored when this arm is selected.
    pub fn value(&self) -> &'a str {
        text(self.0, vocab::VALUE)
    }
    /// What is shown. Falls back to the value.
    pub fn label(&self) -> &'a str {
        let l = text(self.0, vocab::LABEL);
        if l.is_empty() { self.value() } else { l }
    }
    /// Longer human help. May be empty.
    pub fn help(&self) -> &'a str {
        text(self.0, vocab::HELP)
    }
    /// The options this arm adds when selected.
    ///
    /// **An empty list is ordinary and complete**, not missing data: "use
    /// the ambient credential" is the common case, and treating it as an
    /// error would make the common case the exception.
    pub fn fields(&self) -> impl Iterator<Item = OptionRef<'a>> {
        self.0
            .get(vocab::FIELDS)
            .and_then(Value::items)
            .unwrap_or(&[])
            .iter()
            .filter_map(OptionRef::new)
    }
}

/// What an option accepts.
///
/// An exhaustive Rust enum over the value form, so a reader matches rather
/// than comparing strings. [`Kind::Unknown`] carries the name this build
/// did not recognise, which is what makes skipping one option possible
/// while every other option still renders.
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
    /// A sequence, every element of one kind.
    List(&'a Value),
    /// A nested object, with its own options.
    Map(&'a Value),
    /// Exactly one of a fixed set of alternatives.
    Enum(&'a Value),
    /// Any one of several kinds. **Untagged**: validation succeeds if any
    /// arm accepts, and which arm took the value is not the point.
    Union(&'a Value),
    /// One of several alternatives, each with its own fields.
    /// **Tagged**: the discriminant is stored, so knowing the arm is the
    /// point.
    Variant {
        /// The key the discriminant is stored under.
        tag: &'a str,
        /// The arms, as the value holding them.
        arms: &'a Value,
    },
    /// A kind this build does not know, by name. **Skip the option; do not
    /// reject the schema.**
    Unknown(&'a str),
    /// No kind was declared at all, which is a malformed option rather
    /// than a new one.
    Missing,
}

impl<'a> Kind<'a> {
    fn read(kind: Option<&'a Value>) -> Kind<'a> {
        let Some(k) = kind.filter(|v| is_map(v)) else {
            return Kind::Missing;
        };
        let Some(ty) = k.get(vocab::TYPE).and_then(Value::as_str) else {
            return Kind::Missing;
        };
        match ty {
            vocab::TYPE_BOOL => Kind::Bool,
            vocab::TYPE_STRING => Kind::Str,
            vocab::TYPE_BYTES => Kind::Bytes,
            // A list with no element kind says nothing about what it
            // holds, which is a kind this build cannot use rather than a
            // list of anything.
            vocab::TYPE_LIST => match k.get(vocab::ITEMS) {
                Some(i) => Kind::List(i),
                None => Kind::Unknown(ty),
            },
            // A map with no fields is ordinary and complete, the same way
            // an arm with no fields is: it is an object nothing further is
            // declared about.
            vocab::TYPE_MAP => Kind::Map(k.get(vocab::FIELDS).unwrap_or(k)),
            vocab::TYPE_INT => Kind::Int {
                min: opt_int(k, vocab::MIN),
                max: opt_int(k, vocab::MAX),
            },
            vocab::TYPE_FLOAT => Kind::Float {
                min: opt_float(k, vocab::MIN),
                max: opt_float(k, vocab::MAX),
            },
            vocab::TYPE_ENUM => match k.get(vocab::CHOICES) {
                Some(c) => Kind::Enum(c),
                None => Kind::Unknown(ty),
            },
            vocab::TYPE_UNION => match k.get(vocab::ARMS) {
                Some(a) => Kind::Union(a),
                None => Kind::Unknown(ty),
            },
            vocab::TYPE_VARIANT => match (
                k.get(vocab::TAG).and_then(Value::as_str),
                k.get(vocab::ARMS),
            ) {
                (Some(tag), Some(arms)) => Kind::Variant { tag, arms },
                // A variant with no discriminant key cannot be stored, so
                // it is unreadable rather than merely unusual.
                _ => Kind::Unknown(ty),
            },
            other => Kind::Unknown(other),
        }
    }

    /// The alternatives of an enum, or nothing for any other kind.
    pub fn choices(self) -> impl Iterator<Item = ChoiceRef<'a>> {
        let list = match self {
            Kind::Enum(c) => c.items().unwrap_or(&[]),
            _ => &[],
        };
        list.iter().filter(|v| is_map(v)).map(ChoiceRef)
    }

    /// The arms of a **union**, as kinds.
    ///
    /// Empty for a variant, deliberately. The two are different features
    /// and a reader that treated a variant's arms as a union's would draw
    /// a selector whose selection stores a bare discriminant with no
    /// payload.
    pub fn alternatives(self) -> impl Iterator<Item = Kind<'a>> {
        let list = match self {
            Kind::Union(a) => a.items().unwrap_or(&[]),
            _ => &[],
        };
        list.iter().map(|v| Kind::read(Some(v)))
    }

    /// The arms of a **variant**, each with its own fields.
    pub fn arms(self) -> impl Iterator<Item = ArmRef<'a>> {
        let list = match self {
            Kind::Variant { arms, .. } => arms.items().unwrap_or(&[]),
            _ => &[],
        };
        list.iter().filter(|v| is_map(v)).map(ArmRef)
    }

    /// The kind every element of a list has, or [`Kind::Missing`] for
    /// any other kind.
    pub fn items(self) -> Kind<'a> {
        match self {
            Kind::List(i) => Kind::read(Some(i)),
            _ => Kind::Missing,
        }
    }

    /// The options of a nested object, or nothing for any other kind.
    ///
    /// Reads exactly like [`ArmRef::fields`], because it is the same idea:
    /// here are more options, keyed under something.
    pub fn fields(self) -> impl Iterator<Item = OptionRef<'a>> {
        let list = match self {
            Kind::Map(f) => f.items().unwrap_or(&[]),
            _ => &[],
        };
        list.iter().filter_map(OptionRef::new)
    }

    /// The name this kind is written with, for a diagnostic.
    pub fn name(self) -> &'a str {
        match self {
            Kind::Bool => vocab::TYPE_BOOL,
            Kind::Int { .. } => vocab::TYPE_INT,
            Kind::Float { .. } => vocab::TYPE_FLOAT,
            Kind::Str => vocab::TYPE_STRING,
            Kind::Bytes => vocab::TYPE_BYTES,
            Kind::List(_) => vocab::TYPE_LIST,
            Kind::Map(_) => vocab::TYPE_MAP,
            Kind::Enum(_) => vocab::TYPE_ENUM,
            Kind::Union(_) => vocab::TYPE_UNION,
            Kind::Variant { .. } => vocab::TYPE_VARIANT,
            Kind::Unknown(name) => name,
            Kind::Missing => "",
        }
    }
}

fn opt_int(k: &Value, key: &str) -> Option<i64> {
    let v = k.get(key)?;
    // A bound written outside `i64` is no bound this build can apply, and
    // silently clamping it would enforce a limit nobody declared.
    v.as_number_str()?;
    let got = int_or(Some(v), i64::MIN);
    (got != i64::MIN || v.as_number_str() == Some("-9223372036854775808")).then_some(got)
}

fn opt_float(k: &Value, key: &str) -> Option<f64> {
    let v = k.get(key)?;
    v.as_number_str()?;
    let got = float_or(Some(v), f64::NAN);
    got.is_finite().then_some(got)
}

/// Every entry of a map, as `(key, value)`, for a caller walking
/// annotations.
pub fn annotations(v: &Value) -> impl Iterator<Item = (&[u8], &Value)> {
    v.entries()
        .unwrap_or(&[])
        .iter()
        .map(|e| (e.key(), e.value()))
        .filter(|(k, _)| std::str::from_utf8(k).is_ok_and(|s| !vocab::KEYWORDS.contains(&s)))
}
