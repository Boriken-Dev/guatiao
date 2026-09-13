// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The judgement: whether a form fits its schema, how its fields are
//! grouped and ordered, and whether one is showing.
//!
//! Code rather than data, for the reason the schema's validation is code:
//! two renderers that each worked these rules out for themselves would
//! disagree about the same form, and a person would see two screens for
//! one configuration.
//!
//! # Errors never quote a value
//!
//! [`FormError`] names paths, section ids and what would have been
//! accepted, and never the value a condition compares with. That value is
//! something the referenced field could hold, and a field can be a secret.

#![forbid(unsafe_code)]

use std::fmt;

use guatiao::schema::ValidationError;
use guatiao::schema::flat;
use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::schema::validate::{validate_text, validate_value};
use guatiao::schema::vocab as schema_vocab;
use guatiao::value::read::equal;
use guatiao::value::types::{Tag, Value};

use crate::read::{FormRef, HintsRef, SectionRef};
use crate::vocab;

/// Why a form does not fit its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FormError {
    /// Something in the form is not the shape the vocabulary gives it.
    Malformed {
        /// Where, as a path into the form: `sections[1].id`,
        /// `fields["port"].widget`. Empty for the form itself.
        at: String,
        /// What belongs there, phrased for a person.
        expected: &'static str,
    },
    /// A path names no field the schema declares.
    UnknownField {
        /// Where the path was written.
        at: String,
        /// The path itself.
        path: String,
    },
    /// Two sections share an id, so a field's `x-section` could not say
    /// which one it meant.
    DuplicateSection {
        /// The id declared twice.
        id: String,
    },
    /// A condition compares a field with a value that field would never
    /// hold, so the field it guards could never be shown.
    ConditionRefused {
        /// Where the condition was written.
        at: String,
        /// The field it reads.
        field: String,
        /// What that field would accept.
        expected: String,
    },
    /// A field is shown only when a chain of conditions leads back to
    /// itself, so it could never be shown.
    CyclicCondition {
        /// A field on the cycle.
        path: String,
    },
}

impl fmt::Display for FormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormError::Malformed { at, expected } if at.is_empty() => {
                write!(f, "the form should be {expected}")
            }
            FormError::Malformed { at, expected } => write!(f, "{at} should be {expected}"),
            FormError::UnknownField { at, path } => {
                write!(f, "{at} names '{path}', which the schema does not declare")
            }
            FormError::DuplicateSection { id } if id.is_empty() => {
                write!(f, "the default section is declared twice")
            }
            FormError::DuplicateSection { id } => write!(f, "section '{id}' is declared twice"),
            FormError::ConditionRefused {
                at,
                field,
                expected,
            } => write!(
                f,
                "{at} waits for '{field}' to hold a value it never could: it accepts {expected}"
            ),
            FormError::CyclicCondition { path } => write!(
                f,
                "'{path}' is shown only when a chain of conditions leads back to itself"
            ),
        }
    }
}

impl std::error::Error for FormError {}

fn malformed(at: impl Into<String>, expected: &'static str) -> FormError {
    FormError::Malformed {
        at: at.into(),
        expected,
    }
}

fn is_map(v: &Value) -> bool {
    v.tag() == Ok(Tag::GUATIAO_MAP)
}

/// A key that must be text when it is there at all.
fn text_if_present(v: &Value, key: &str, at: &str) -> Result<(), FormError> {
    match v.get(key) {
        Some(x) if x.as_str().is_none() => Err(malformed(format!("{at}.{key}"), "text")),
        _ => Ok(()),
    }
}

/// Where a field's hints are, written so a dotted path stays one step.
fn field_at(path: &str) -> String {
    format!("{}[\"{path}\"]", vocab::FIELDS)
}

/// Whether `form` fits `schema`: every path names a declared field, every
/// section id is declared once, every hint has its shape, every condition
/// could be met, and no field waits on itself.
///
/// A section a schema field names but the form does not declare is **not**
/// an error: that field joins the default section, which is what the
/// schema's own `FormBuilder::section` promises.
pub fn check(schema: SchemaRef<'_>, form: FormRef<'_>) -> Result<(), FormError> {
    let doc = form.as_value();

    if let Some(sections) = doc.get(vocab::SECTIONS) {
        let items = sections
            .items()
            .ok_or_else(|| malformed(vocab::SECTIONS, "a list"))?;
        let mut seen: Vec<&str> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let at = format!("{}[{i}]", vocab::SECTIONS);
            if !is_map(item) {
                return Err(malformed(at, "a map"));
            }
            let Some(id) = item.get(vocab::ID).and_then(Value::as_str) else {
                return Err(malformed(format!("{at}.{}", vocab::ID), "text"));
            };
            text_if_present(item, vocab::TITLE, &at)?;
            text_if_present(item, vocab::DESCRIPTION, &at)?;
            if seen.contains(&id) {
                return Err(FormError::DuplicateSection { id: id.to_string() });
            }
            seen.push(id);
        }
    }

    if let Some(fields) = doc.get(vocab::FIELDS) {
        let entries = fields
            .entries()
            .ok_or_else(|| malformed(vocab::FIELDS, "a map"))?;
        for entry in entries {
            let Some(path) = entry.key_str() else {
                return Err(malformed(vocab::FIELDS, "a map whose keys are text"));
            };
            if flat::resolve(schema, path).is_none() {
                return Err(FormError::UnknownField {
                    at: vocab::FIELDS.to_string(),
                    path: path.to_string(),
                });
            }
            let at = field_at(path);
            let hints = entry.value();
            if !is_map(hints) {
                return Err(malformed(at, "a map"));
            }
            text_if_present(hints, vocab::WIDGET, &at)?;
            text_if_present(hints, vocab::PLACEHOLDER, &at)?;
            if let Some(condition) = hints.get(vocab::VISIBLE_WHEN) {
                check_condition(schema, condition, format!("{at}.{}", vocab::VISIBLE_WHEN))?;
            }
        }
        // After every condition is known to be well formed, so a cycle is
        // reported as a cycle rather than as the malformed link in it.
        for entry in entries {
            if let Some(path) = entry.key_str()
                && let Some(on_cycle) = cycle_from(form, path)
            {
                return Err(FormError::CyclicCondition { path: on_cycle });
            }
        }
    }
    Ok(())
}

fn check_condition(schema: SchemaRef<'_>, condition: &Value, at: String) -> Result<(), FormError> {
    if !is_map(condition) {
        return Err(malformed(at, "a map holding `field` and `equals`"));
    }
    let Some(field) = condition.get(vocab::FIELD).and_then(Value::as_str) else {
        return Err(malformed(format!("{at}.{}", vocab::FIELD), "text"));
    };
    let Some(equals) = condition.get(vocab::EQUALS) else {
        return Err(malformed(
            format!("{at}.{}", vocab::EQUALS),
            "a value to compare with",
        ));
    };
    let Some(referenced) = flat::resolve(schema, field) else {
        return Err(FormError::UnknownField {
            at,
            path: field.to_string(),
        });
    };

    // A variant named by its own key is compared with its DISCRIMINANT,
    // so what must be acceptable is the text form, which for a variant is
    // exactly the name of an arm.
    let accepted = match (referenced.kind(), flat::split(field)) {
        (Kind::Variant { .. }, None) => match equals.as_str() {
            Some(arm) => validate_text(referenced, arm),
            None => {
                return Err(FormError::ConditionRefused {
                    at,
                    field: field.to_string(),
                    expected: "the name of one of its arms".to_string(),
                });
            }
        },
        _ => validate_value(referenced, equals),
    };
    accepted.map_err(|e| FormError::ConditionRefused {
        at,
        field: field.to_string(),
        expected: expected_of(e),
    })
}

/// What a validation error says would have been accepted.
fn expected_of(e: ValidationError) -> String {
    match e {
        ValidationError::BadValue { expected, .. } => expected,
        other => other.to_string(),
    }
}

/// A field on a cycle reachable from `start`, if there is one.
///
/// Each field has at most one condition, so a chain has one successor per
/// step and walking it either ends or repeats. The field that repeats is
/// the one on the cycle; `start` may only lead into it.
fn cycle_from(form: FormRef<'_>, start: &str) -> Option<String> {
    let mut walked: Vec<String> = Vec::new();
    let mut current = start.to_string();
    loop {
        if walked.contains(&current) {
            return Some(current);
        }
        let next = form.hints(&current).visible_when()?.field().to_string();
        walked.push(current);
        current = next;
    }
}

/// A group of fields a person sees together.
#[derive(Clone, Debug)]
pub struct Group<'a> {
    /// The section declared for this group, or `None` for the default
    /// group when the form does not declare it.
    pub section: Option<SectionRef<'a>>,
    /// The fields, in the order they are shown.
    pub fields: Vec<Placed<'a>>,
}

/// One field in a group, with what it is shown with.
#[derive(Clone, Copy, Debug)]
pub struct Placed<'a> {
    /// The field, as the schema declares it.
    pub field: FieldRef<'a>,
    /// Its hints from the form. Empty when the form says nothing about it.
    pub hints: HintsRef<'a>,
}

/// The schema's fields grouped into sections and put in order.
///
/// - **Groups**: the sections the form declares, in its order. The default
///   section `""` comes first unless the form declares it somewhere else.
///   A field whose `x-section` the form does not declare joins the default
///   group. A group with no fields is left out.
/// - **Within a group**: fields with an explicit `x-order` first, lowest
///   first; then the rest in the order the schema declares them.
///
/// Top-level fields only. The fields an arm of a variant adds are shown
/// with that variant, and their hints are [`FormRef::hints`] under
/// `owner.member`.
///
/// Visibility is not applied, because it depends on values this does not
/// have; [`is_visible`] answers it per field.
pub fn layout<'a>(schema: SchemaRef<'a>, form: FormRef<'a>) -> Vec<Group<'a>> {
    let mut order: Vec<(&'a str, Option<SectionRef<'a>>)> = Vec::new();
    if form.section(vocab::DEFAULT_SECTION).is_none() {
        order.push((vocab::DEFAULT_SECTION, None));
    }
    for section in form.sections() {
        // The first declaration wins; `check` refuses the second.
        if !order.iter().any(|(id, _)| *id == section.id()) {
            order.push((section.id(), Some(section)));
        }
    }
    let default_group = order.iter().position(|(id, _)| id.is_empty()).unwrap_or(0);

    let mut groups: Vec<Group<'a>> = order
        .iter()
        .map(|(_, section)| Group {
            section: *section,
            fields: Vec::new(),
        })
        .collect();
    for field in schema.fields() {
        let group = order
            .iter()
            .position(|(id, _)| *id == field.section())
            .unwrap_or(default_group);
        groups[group].fields.push(Placed {
            field,
            hints: form.hints(field.key()),
        });
    }
    for group in &mut groups {
        // Stable, so fields with no order keep the schema's own order.
        group.fields.sort_by_key(|placed| order_key(placed.field));
    }
    groups.retain(|group| !group.fields.is_empty());
    groups
}

/// Explicit orders first and ascending, then everything without one.
fn order_key(field: FieldRef<'_>) -> (bool, i64) {
    match field.as_value().get(schema_vocab::X_ORDER) {
        Some(_) => (false, field.order()),
        None => (true, 0),
    }
}

/// Whether the field at `path` is shown, given the `values` a person has
/// entered so far.
///
/// - No condition: shown.
/// - A condition: shown when the field it reads is **itself shown** and
///   holds `equals`. A condition on a hidden field is not met, so hiding a
///   field hides everything that waits on it.
/// - A field that holds nothing yet reads as its schema `default`, and one
///   with no default does not meet the condition.
/// - A variant named by its own key reads as its discriminant.
/// - A cycle is not shown; [`check`] reports it.
///
/// `values` is a map shaped the way the schema's values are: a variant's
/// value is a map carrying its tag, an arm's field sits inside it.
pub fn is_visible(schema: SchemaRef<'_>, form: FormRef<'_>, path: &str, values: &Value) -> bool {
    let mut walked = Vec::new();
    visible(schema, form, path, values, &mut walked)
}

fn visible(
    schema: SchemaRef<'_>,
    form: FormRef<'_>,
    path: &str,
    values: &Value,
    walked: &mut Vec<String>,
) -> bool {
    if walked.iter().any(|w| w == path) {
        return false;
    }
    let Some(condition) = form.hints(path).visible_when() else {
        return true;
    };
    walked.push(path.to_string());
    visible(schema, form, condition.field(), values, walked)
        && current(schema, values, condition.field())
            .is_some_and(|held| equal(held, condition.equals()))
}

/// What the field at `path` holds now: its value, else its default, and
/// for a variant named by its own key, the discriminant of either.
fn current<'a>(schema: SchemaRef<'a>, values: &'a Value, path: &str) -> Option<&'a Value> {
    match flat::split(path) {
        None => {
            let field = schema.find(path)?;
            let held = values.get(path).or_else(|| field.default())?;
            match field.kind() {
                Kind::Variant { tag, .. } if is_map(held) => held.get(tag),
                _ => Some(held),
            }
        }
        Some((owner, member)) => values
            .get(owner)
            .and_then(|o| o.get(member))
            .or_else(|| flat::resolve(schema, path).and_then(|f| f.default())),
    }
}
