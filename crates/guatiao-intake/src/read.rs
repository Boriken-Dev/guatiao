// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a form out of a value.
//!
//! Views that borrow and allocate nothing, the same arrangement as
//! `guatiao::schema::read`. **Skipping is the rule**: a section with no
//! text `id` is not a section, so [`FormRef::sections`] passes over it,
//! and a field's hints that are not a map read as no hints. Refusing is
//! [`crate::check`]'s job, which says what was wrong and where.

#![forbid(unsafe_code)]

use guatiao::value::convert::TryAsRef;
use guatiao::value::types::{List, Map, Tag, Value};

use crate::vocab;

/// A form: how a schema's fields are shown.
#[derive(Clone, Copy, Debug)]
pub struct FormRef<'a>(&'a Value);

/// One section: what a group of fields is called.
#[derive(Clone, Copy, Debug)]
pub struct SectionRef<'a>(&'a Value);

/// The hints for one field. Empty when the form says nothing about it,
/// which is the ordinary case rather than a missing one.
#[derive(Clone, Copy, Debug)]
pub struct HintsRef<'a>(Option<&'a Value>);

/// The one condition under which a field is shown.
#[derive(Clone, Copy, Debug)]
pub struct Condition<'a> {
    field: &'a str,
    equals: &'a Value,
}

fn is_map(v: &Value) -> bool {
    v.tag() == Ok(Tag::GUATIAO_MAP)
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    TryAsRef::<Map>::try_as_ref(v)
        .and_then(|m| m.get(key))
        .and_then(TryAsRef::<str>::try_as_ref)
        .unwrap_or("")
}

/// The value under `key`, unless this vocabulary gives that key a meaning.
fn annotation<'a>(v: &'a Value, key: &str, known: &[&str]) -> Option<&'a Value> {
    TryAsRef::<Map>::try_as_ref(v)
        .and_then(|m| m.get(key))
        .filter(|_| !known.contains(&key))
}

impl<'a> FormRef<'a> {
    /// Views `value` as a form, or `None` when it is not a map.
    pub fn new(value: &'a Value) -> Option<FormRef<'a>> {
        is_map(value).then_some(FormRef(value))
    }

    /// The value this is a view of.
    pub fn as_value(&self) -> &'a Value {
        self.0
    }

    /// Every section, in the order a person sees them.
    pub fn sections(&self) -> impl Iterator<Item = SectionRef<'a>> {
        TryAsRef::<Map>::try_as_ref(self.0)
            .and_then(|m| m.get(vocab::SECTIONS))
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .unwrap_or(&[])
            .iter()
            .filter(|s| {
                TryAsRef::<Map>::try_as_ref(*s)
                    .and_then(|m| m.get(vocab::ID))
                    .and_then(TryAsRef::<str>::try_as_ref)
                    .is_some()
            })
            .map(SectionRef)
    }

    /// The section declared with `id`, if any. The first, when a form
    /// declares one twice — which `check` refuses.
    pub fn section(&self, id: &str) -> Option<SectionRef<'a>> {
        self.sections().find(|s| s.id() == id)
    }

    /// The hints for the field at `path`. Empty when there are none.
    pub fn hints(&self, path: &str) -> HintsRef<'a> {
        HintsRef(
            TryAsRef::<Map>::try_as_ref(self.0)
                .and_then(|m| m.get(vocab::FIELDS))
                .and_then(TryAsRef::<Map>::try_as_ref)
                .and_then(|fields| fields.get(path))
                .filter(|h| is_map(h)),
        )
    }

    /// Every path the form gives hints to, with its hints, in the order
    /// they were written.
    pub fn fields(&self) -> impl Iterator<Item = (&'a str, HintsRef<'a>)> {
        TryAsRef::<Map>::try_as_ref(self.0)
            .and_then(|m| m.get(vocab::FIELDS))
            .and_then(TryAsRef::<Map>::try_as_ref)
            .map(Map::entries)
            .unwrap_or(&[])
            .iter()
            .map(|e| (e.key(), HintsRef(Some(e.value()).filter(|h| is_map(h)))))
    }

    /// An annotation on the form as a whole. Carried, never interpreted.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        annotation(self.0, key, vocab::FORM_KEYS)
    }
}

impl<'a> SectionRef<'a> {
    /// The id a field's `x-section` names. `""` is the default section.
    pub fn id(&self) -> &'a str {
        text(self.0, vocab::ID)
    }

    /// What a person sees the section called. May be empty.
    pub fn label(&self) -> &'a str {
        text(self.0, vocab::TITLE)
    }

    /// Longer prose under the title. May be empty.
    pub fn help(&self) -> &'a str {
        text(self.0, vocab::DESCRIPTION)
    }

    /// The value this is a view of.
    pub fn as_value(&self) -> &'a Value {
        self.0
    }

    /// An annotation on this section. Carried, never interpreted.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        annotation(self.0, key, vocab::SECTION_KEYS)
    }
}

impl<'a> HintsRef<'a> {
    /// Whether the form says anything at all about this field.
    pub fn is_empty(&self) -> bool {
        self.0.is_none_or(|h| {
            TryAsRef::<Map>::try_as_ref(h)
                .map(Map::entries)
                .is_none_or(<[_]>::is_empty)
        })
    }

    /// Which control to draw, or `""` for the kind's default.
    pub fn widget(&self) -> &'a str {
        self.0.map_or("", |h| text(h, vocab::WIDGET))
    }

    /// Text shown in the empty control, or `""`.
    pub fn placeholder(&self) -> &'a str {
        self.0.map_or("", |h| text(h, vocab::PLACEHOLDER))
    }

    /// The condition under which the field is shown, if it has one.
    ///
    /// `None` for a condition with no text `field` or no `equals` as well
    /// as for no condition at all; `check` tells those apart.
    pub fn visible_when(&self) -> Option<Condition<'a>> {
        let condition =
            TryAsRef::<Map>::try_as_ref(self.0?).and_then(|m| m.get(vocab::VISIBLE_WHEN))?;
        Some(Condition {
            field: TryAsRef::<Map>::try_as_ref(condition)
                .and_then(|m| m.get(vocab::FIELD))
                .and_then(TryAsRef::<str>::try_as_ref)?,
            equals: TryAsRef::<Map>::try_as_ref(condition).and_then(|m| m.get(vocab::EQUALS))?,
        })
    }

    /// The value these hints are a view of, if the form has any.
    pub fn as_value(&self) -> Option<&'a Value> {
        self.0
    }

    /// An annotation on these hints. Carried, never interpreted.
    pub fn extra(&self, key: &str) -> Option<&'a Value> {
        self.0.and_then(|h| annotation(h, key, vocab::HINT_KEYS))
    }
}

impl<'a> Condition<'a> {
    /// The path of the field the condition reads.
    pub fn field(&self) -> &'a str {
        self.field
    }

    /// The value that field must hold.
    pub fn equals(&self) -> &'a Value {
        self.equals
    }
}
