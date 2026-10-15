// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hiding a value's secrets by what its schema says.
//!
//! A field marked `x-sensitive` (or JSON Schema's `writeOnly`) is one
//! nobody prints, and a value that is about to be logged, shown or sent to
//! a support request holds such fields at any depth: inside a nested
//! object, in every element of a list, in the payload of a variant's
//! selected arm. [`redact`] walks the value with its schema and returns a
//! copy with each of them replaced.
//!
//! **Where the schema stops describing the value, the value is hidden**,
//! because a secret may be there and nothing says it is not: a key the
//! schema does not declare, a container where the schema names a scalar, a
//! variant whose discriminant names no arm, a union no arm accepts, and
//! the contents of a kind this build does not read. A scalar of an
//! unreadable kind is kept, since only a mark makes a scalar secret.

#![forbid(unsafe_code)]

use super::read::{Kind, SchemaRef};
use super::validate::accepts;
use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::{ToValue, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::read::{bool_or, str_or};
use crate::value::types::{List, Map, Tag, Text, Value};

/// A copy of `value` with every secret replaced by `placeholder`.
///
/// A value is a secret when its field's schema sets `x-sensitive` or
/// `writeOnly`, at any depth -- inside a nested object, in every element
/// of a list, in the selected arm of a variant -- whatever it holds (a null
/// included, so the copy does not say whether one was set).
///
/// **Where the schema stops describing the value, the value is hidden**,
/// because a secret may be there and nothing says it is not: a key the
/// schema does not declare, a container where it names a scalar, a variant
/// whose discriminant names no arm, a union no arm accepts, and the
/// contents of a kind this build does not read. A scalar of an unreadable
/// kind is kept, since only a mark makes a scalar secret.
///
/// Built with the crate's allocator; [`redact_in`] names one.
///
/// # Errors
///
/// [`ValueError::TooDeep`] for a value nested past `MAX_DEPTH`, and the
/// allocator's refusal.
pub fn redact(
    schema: SchemaRef<'_>,
    value: &Value,
    placeholder: &str,
) -> Result<Value, ValueError> {
    redact_in(schema, value, placeholder, Alloc::rust())
}

/// [`redact`], building the copy through `alloc`.
pub fn redact_in(
    schema: SchemaRef<'_>,
    value: &Value,
    placeholder: &str,
    alloc: Alloc,
) -> Result<Value, ValueError> {
    Redactor { placeholder, alloc }.node(schema.as_value(), value, 0)
}

struct Redactor<'p> {
    placeholder: &'p str,
    alloc: Alloc,
}

/// What becomes of one entry of a map.
enum Part<'s> {
    /// It is not data (a variant's discriminant): copied as it is.
    Keep,
    /// It is described by this schema.
    Described(&'s Value),
    /// Nothing describes it.
    Hide,
}

/// Whether a schema marks its value as a secret.
fn is_secret(schema: &Value) -> bool {
    let flag = |key| {
        bool_or(
            TryAsRef::<Map>::try_as_ref(schema).and_then(|m| m.get(key)),
            false,
        )
    };
    flag(vocab::X_SENSITIVE) || flag(vocab::WRITE_ONLY)
}

fn is_container(value: &Value) -> bool {
    matches!(value.tag(), Ok(Tag::GUATIAO_MAP | Tag::GUATIAO_LIST))
}

impl Redactor<'_> {
    fn mask(&self) -> Result<Value, ValueError> {
        Ok(Text::new_in(self.alloc, self.placeholder)?.into())
    }

    fn keep(&self, value: &Value) -> Result<Value, ValueError> {
        value.to_value_in(self.alloc)
    }

    /// `value` as `schema` describes it, secrets replaced.
    fn node(&self, schema: &Value, value: &Value, depth: u32) -> Result<Value, ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        if is_secret(schema) {
            return self.mask();
        }
        let kind = Kind::read(Some(schema));
        match kind {
            Kind::List { items, .. } => {
                let Some(elements) = TryAsRef::<List>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let mut out = List::new_in(self.alloc);
                for element in elements.iter() {
                    out.push_in(self.node(items, element, depth + 1)?, self.alloc)?;
                }
                Ok(out.into())
            }
            Kind::MapOf(owner) => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let element = TryAsRef::<Map>::try_as_ref(owner)
                    .and_then(|m| m.get(vocab::ADDITIONAL_PROPERTIES));
                self.entries(map, depth, |_| match element {
                    Some(schema) => Part::Described(schema),
                    None => Part::Hide,
                })
            }
            Kind::Map(_) => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                self.entries(map, depth, |key| {
                    match kind.fields().find(|f| f.key() == key) {
                        Some(field) => Part::Described(field.as_value()),
                        None => Part::Hide,
                    }
                })
            }
            Kind::Variant { tag, .. } => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let chosen = str_or(map.get(tag), "");
                let arm = kind.arms().find(|a| a.value() == chosen);
                self.entries(map, depth, |key| {
                    if key == tag {
                        return Part::Keep;
                    }
                    match arm.and_then(|a| a.fields().find(|f| f.key() == key)) {
                        Some(field) => Part::Described(field.as_value()),
                        None => Part::Hide,
                    }
                })
            }
            Kind::Union(any_of) => {
                let arm = TryAsRef::<List>::try_as_ref(any_of)
                    .into_iter()
                    .flat_map(|list| list.iter())
                    .find(|arm| accepts(arm, value));
                match arm {
                    Some(arm) => self.node(arm, value, depth + 1),
                    None => self.fallback(value),
                }
            }
            // A scalar with no mark is not a secret, and a container
            // where the schema names a scalar is not what it described.
            Kind::Null
            | Kind::Bool
            | Kind::Int { .. }
            | Kind::Float { .. }
            | Kind::Str { .. }
            | Kind::Bytes
            | Kind::Enum(_)
            | Kind::Unknown(_)
            | Kind::Missing => self.fallback(value),
        }
    }

    /// What a value becomes when no part of the schema tells what is in
    /// it: a scalar is kept, a container is hidden.
    fn fallback(&self, value: &Value) -> Result<Value, ValueError> {
        if is_container(value) {
            self.mask()
        } else {
            self.keep(value)
        }
    }

    /// A map's entries, each treated as `part` says for its key.
    fn entries<'s>(
        &self,
        map: &Map,
        depth: u32,
        part: impl Fn(&str) -> Part<'s>,
    ) -> Result<Value, ValueError> {
        let mut out = Map::new_in(self.alloc);
        for entry in map {
            let child = match part(entry.key()) {
                Part::Keep => self.keep(entry.value())?,
                Part::Described(schema) => self.node(schema, entry.value(), depth + 1)?,
                Part::Hide => self.mask()?,
            };
            out.set_in(entry.key(), child, self.alloc)?;
        }
        Ok(out.into())
    }
}
