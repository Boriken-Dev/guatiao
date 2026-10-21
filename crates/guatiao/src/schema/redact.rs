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

use super::read::{Kind, SchemaRef, peel_once, prefix_items};
use super::validate::accepts;
use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::{ToValue, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::read::bool_or;
use crate::value::types::{List, Map, Tag, Text, Value};

/// A copy of `value` with every secret replaced by `placeholder`.
///
/// A value is a secret when its field's schema sets `x-sensitive` or
/// `writeOnly`, at any depth -- inside a nested object, in every element
/// of a list, in the selected arm of a variant, on the schema an optional
/// wraps (`anyOf: [X, null]`) or on any member of an `allOf` -- whatever it
/// holds (a null included, so the copy does not say whether one was set).
/// A value a union accepts is a secret when **any** arm accepting it is
/// marked, and is hidden whole when the accepting arms would redact it
/// differently.
///
/// **Where the schema stops describing the value, the value is hidden**,
/// because a secret may be there and nothing says it is not: a key the
/// schema does not declare, a container where it names a scalar, a variant
/// whose discriminant names no arm or is not text, a union no arm accepts,
/// and the contents of a kind this build does not read. A scalar of an
/// unreadable kind is kept, since only a mark makes a scalar secret --
/// except under a `$ref`, where the mark may be on the definition: **run
/// [`inline_refs`](super::inline_refs) first**, or everything under a
/// reference is hidden.
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

/// Whether a schema marks its value as a secret, by itself.
fn is_secret(schema: &Value) -> bool {
    let flag = |key| {
        bool_or(
            TryAsRef::<Map>::try_as_ref(schema).and_then(|m| m.get(key)),
            false,
        )
    };
    flag(vocab::X_SENSITIVE) || flag(vocab::WRITE_ONLY)
}

/// Whether a schema marks its value as a secret: by itself, on any member
/// of an `allOf` (every member describes the whole value), or on what an
/// optional or a lone `allOf` wraps. Past the depth bound, assumed so.
fn marks_secret(schema: &Value, depth: u32) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    let mut k = schema;
    loop {
        if is_secret(k) {
            return true;
        }
        let members = TryAsRef::<Map>::try_as_ref(k)
            .and_then(|m| m.get(vocab::ALL_OF))
            .and_then(TryAsRef::<List>::try_as_ref)
            .map(|list| &list[..])
            .unwrap_or(&[]);
        if members.iter().any(|m| marks_secret(m, depth + 1)) {
            return true;
        }
        match peel_once(k) {
            Some(inner) => k = inner,
            None => return false,
        }
    }
}

fn is_container(value: &Value) -> bool {
    matches!(value.tag(), Ok(Tag::GUATIAO_MAP | Tag::GUATIAO_LIST))
}

/// The schema an object gives a key it does not declare: its
/// `additionalProperties`, when that is a schema.
fn undeclared(owner: &Value) -> Option<&Value> {
    TryAsRef::<Map>::try_as_ref(owner)
        .and_then(|m| m.get(vocab::ADDITIONAL_PROPERTIES))
        .filter(|v| TryAsRef::<Map>::try_as_ref(*v).is_some())
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
        if marks_secret(schema, depth) {
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
            Kind::Tuple(owner) => {
                let Some(elements) = TryAsRef::<List>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let positions = prefix_items(owner);
                let mut out = List::new_in(self.alloc);
                for (i, element) in elements.iter().enumerate() {
                    // An element past the positions is described by nothing.
                    let redacted = match positions.get(i) {
                        Some(position) => self.node(position, element, depth + 1)?,
                        None => self.fallback(element)?,
                    };
                    out.push_in(redacted, self.alloc)?;
                }
                Ok(out.into())
            }
            Kind::MapOf(owner) => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let element = undeclared(owner);
                self.entries(map, depth, |_, _| match element {
                    Some(schema) => Part::Described(schema),
                    None => Part::Hide,
                })
            }
            Kind::Map(owner) => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                let extra = undeclared(owner);
                self.entries(map, depth, |key, _| {
                    match kind.fields().find(|f| f.key() == key) {
                        Some(field) => Part::Described(field.as_value()),
                        None => extra.map_or(Part::Hide, Part::Described),
                    }
                })
            }
            Kind::Variant { tag, .. } => {
                let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                    return self.fallback(value);
                };
                // Only a discriminant that is text names an arm, and only
                // text is kept as one: anything else under the tag's key
                // is data nothing describes.
                let chosen = map.get(tag).and_then(TryAsRef::<str>::try_as_ref);
                let arm = chosen.and_then(|c| kind.arms().find(|a| a.value() == c));
                self.entries(map, depth, |key, entry| {
                    if key == tag {
                        return if TryAsRef::<str>::try_as_ref(entry).is_some() {
                            Part::Keep
                        } else {
                            Part::Hide
                        };
                    }
                    match arm.and_then(|a| a.fields().find(|f| f.key() == key)) {
                        Some(field) => Part::Described(field.as_value()),
                        None => Part::Hide,
                    }
                })
            }
            Kind::Union(any_of) => {
                let accepting: Vec<&Value> = TryAsRef::<List>::try_as_ref(any_of)
                    .map(|list| &list[..])
                    .unwrap_or(&[])
                    .iter()
                    .filter(|arm| accepts(arm, value))
                    .collect();
                let Some((first, rest)) = accepting.split_first() else {
                    return self.fallback(value);
                };
                if accepting.iter().any(|arm| marks_secret(arm, depth + 1)) {
                    return self.mask();
                }
                // Two arms accepting one value may describe it differently,
                // and the copy must hide what either would.
                let redacted = self.node(first, value, depth + 1)?;
                for arm in rest {
                    if self.node(arm, value, depth + 1)? != redacted {
                        return self.mask();
                    }
                }
                Ok(redacted)
            }
            // A reference not inlined: its definition may carry the mark,
            // so nothing under it is shown.
            Kind::Unknown(vocab::REF) => self.mask(),
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

    /// A map's entries, each treated as `part` says for its key and value.
    fn entries<'s>(
        &self,
        map: &Map,
        depth: u32,
        part: impl Fn(&str, &Value) -> Part<'s>,
    ) -> Result<Value, ValueError> {
        let mut out = Map::new_in(self.alloc);
        for entry in map {
            let child = match part(entry.key(), entry.value()) {
                Part::Keep => self.keep(entry.value())?,
                Part::Described(schema) => self.node(schema, entry.value(), depth + 1)?,
                Part::Hide => self.mask()?,
            };
            out.set_in(entry.key(), child, self.alloc)?;
        }
        Ok(out.into())
    }
}
