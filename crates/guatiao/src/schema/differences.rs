// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Where a value differs from its schema's defaults, or from a baseline.
//!
//! [`differences`] walks a value with its schema and lists the places it
//! departs from the baseline: the schema's own defaults, or a second value.
//! Each place is a path, the value there and the baseline's. The walk is
//! the one [`redact`](super::redact) makes, so a secret is reported as
//! changed and never shown.
//!
//! # What is one difference
//!
//! - An object's declared fields, an open map's keys and a variant's
//!   fields (when both sides name the same arm) are walked.
//! - Everything else is **one leaf**, compared whole: a scalar, a list, a
//!   tuple (a duration is one), an enumeration, an untagged union, a
//!   variant whose arm changed, and any container present on one side
//!   only. A list is reported whole, never per element.
//! - A field absent from a side reads as its schema default, when it has
//!   one, so an absent field equal to the other side's value is no
//!   difference. With no default, absent stays absent (`None`).
//! - A leaf under a secret, or in a place the schema does not describe, is
//!   compared as it is and reported as the placeholder on each side that
//!   has a value.
//!
//! Results come in order of their path segments, compared as text.

#![forbid(unsafe_code)]

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::read::{Kind, SchemaRef};
use super::redact::{marks_secret, placeholder_value, redact_node, undeclared};
use crate::value::alloc::Alloc;
use crate::value::convert::TryAsRef;
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::merge::path::render;
use crate::value::types::{List, Map, Text, Value};

/// One place where a value departs from its baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    /// The key segments from the root: `["waiting", "per_source"]`. A map
    /// key is one segment, whatever it holds.
    pub path: Vec<String>,
    /// The value there, or `None` when the value has none. A secret is the
    /// placeholder.
    pub value: Option<Value>,
    /// The baseline's, or `None` when it has none. A secret is the
    /// placeholder.
    pub baseline: Option<Value>,
}

impl Difference {
    /// The path as one line in `guatiao-intake`'s grammar: `tls.ca`,
    /// `peers[0].name`, `["a.b"]`. Every spelling parses back to `path`.
    pub fn key(&self) -> String {
        render(&self.path)
    }
}

/// Where `value` differs from `baseline`, or from the schema's defaults
/// when `baseline` is `None`.
///
/// The module documentation says what counts as one difference. Secrets are
/// compared as they are and shown as `placeholder`. **Run
/// [`inline_refs`](super::inline_refs) first**: a place under a `$ref`
/// that is still there is compared whole and shown as `placeholder`,
/// because the mark may be on the definition.
///
/// # Errors
///
/// [`ValueError::TooDeep`] for a value nested past `MAX_DEPTH`, and the
/// allocator's refusal.
pub fn differences(
    schema: SchemaRef<'_>,
    value: &Value,
    baseline: Option<&Value>,
    placeholder: &str,
) -> Result<Vec<Difference>, ValueError> {
    differences_in(schema, value, baseline, placeholder, Alloc::rust())
}

/// [`differences`], building the reported values through `alloc`.
pub fn differences_in(
    schema: SchemaRef<'_>,
    value: &Value,
    baseline: Option<&Value>,
    placeholder: &str,
    alloc: Alloc,
) -> Result<Vec<Difference>, ValueError> {
    let empty = Value::from(Map::new_in(alloc));
    let mut walk = Walk {
        placeholder,
        alloc,
        out: Vec::new(),
    };
    walk.node(
        schema.as_value(),
        Some(value),
        Some(baseline.unwrap_or(&empty)),
        &mut Vec::new(),
        0,
    )?;
    Ok(walk.out)
}

/// [`differences`] as a value, for a caller that reads values and not
/// Rust types: a list of maps `{path: [segments], key: text, value: ..,
/// default: ..}`, with `value` and `default` left out where the side has
/// none.
pub fn differences_value(
    schema: SchemaRef<'_>,
    value: &Value,
    baseline: Option<&Value>,
    placeholder: &str,
) -> Result<Value, ValueError> {
    differences_value_in(schema, value, baseline, placeholder, Alloc::rust())
}

/// [`differences_value`], building the list through `alloc`.
pub fn differences_value_in(
    schema: SchemaRef<'_>,
    value: &Value,
    baseline: Option<&Value>,
    placeholder: &str,
    alloc: Alloc,
) -> Result<Value, ValueError> {
    let found = differences_in(schema, value, baseline, placeholder, alloc)?;
    let mut list = List::new_in(alloc);
    for d in found {
        let mut path = List::new_in(alloc);
        for segment in &d.path {
            path.push_in(Text::new_in(alloc, segment)?, alloc)?;
        }
        let mut entry = Map::new_in(alloc);
        entry.set_in("path", path, alloc)?;
        entry.set_in("key", Text::new_in(alloc, &d.key())?, alloc)?;
        if let Some(v) = d.value {
            entry.set_in("value", v, alloc)?;
        }
        if let Some(b) = d.baseline {
            entry.set_in("default", b, alloc)?;
        }
        list.push_in(entry, alloc)?;
    }
    Ok(list.into())
}

struct Walk<'p> {
    placeholder: &'p str,
    alloc: Alloc,
    out: Vec<Difference>,
}

fn map_of(value: Option<&Value>) -> Option<&Map> {
    value.and_then(TryAsRef::<Map>::try_as_ref)
}

impl Walk<'_> {
    fn node(
        &mut self,
        schema: &Value,
        value: Option<&Value>,
        base: Option<&Value>,
        path: &mut Vec<String>,
        depth: u32,
    ) -> Result<(), ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        // Compared as they are, so a secret that did not change is not
        // listed and one that did is.
        if value == base {
            return Ok(());
        }
        if !marks_secret(schema, depth) && self.children(schema, value, base, path, depth)? {
            return Ok(());
        }
        self.leaf(Some(schema), value, base, path)
    }

    /// Walks what both sides hold under a kind that has parts. `false`
    /// when the pair is one leaf.
    fn children(
        &mut self,
        schema: &Value,
        value: Option<&Value>,
        base: Option<&Value>,
        path: &mut Vec<String>,
        depth: u32,
    ) -> Result<bool, ValueError> {
        let (Some(v), Some(b)) = (map_of(value), map_of(base)) else {
            return Ok(false);
        };
        let kind = Kind::read(Some(schema));
        match kind {
            Kind::Map(owner) => {
                let extra = undeclared(owner);
                self.entries(v, b, depth, path, |key| {
                    match kind.fields().find(|f| f.key() == key) {
                        Some(f) => Part::Field(f.as_value(), f.default()),
                        None => extra.map_or(Part::Undescribed, |s| Part::Field(s, None)),
                    }
                })?;
            }
            Kind::MapOf(owner) => {
                let element = undeclared(owner);
                self.entries(v, b, depth, path, |_| {
                    element.map_or(Part::Undescribed, |s| Part::Field(s, None))
                })?;
            }
            Kind::Variant { tag, .. } => {
                let arm_of = |m: &Map| {
                    m.get(tag)
                        .and_then(TryAsRef::<str>::try_as_ref)
                        .and_then(|c| kind.arms().find(|a| a.value() == c))
                };
                let (Some(left), Some(right)) = (arm_of(v), arm_of(b)) else {
                    return Ok(false);
                };
                if left.value() != right.value() {
                    return Ok(false);
                }
                self.entries(v, b, depth, path, |key| {
                    if key == tag {
                        return Part::Skip;
                    }
                    match left.fields().find(|f| f.key() == key) {
                        Some(f) => Part::Field(f.as_value(), f.default()),
                        None => Part::Undescribed,
                    }
                })?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The keys either map holds, in order, each compared as `part` says.
    fn entries<'s>(
        &mut self,
        v: &Map,
        b: &Map,
        depth: u32,
        path: &mut Vec<String>,
        part: impl Fn(&str) -> Part<'s>,
    ) -> Result<(), ValueError> {
        let keys: BTreeSet<&str> = v.iter().chain(b.iter()).map(|e| e.key()).collect();
        for key in keys {
            path.push(key.to_string());
            match part(key) {
                Part::Skip => {}
                Part::Undescribed => self.leaf(None, v.get(key), b.get(key), path)?,
                Part::Field(schema, default) => self.node(
                    schema,
                    v.get(key).or(default),
                    b.get(key).or(default),
                    path,
                    depth + 1,
                )?,
            }
            path.pop();
        }
        Ok(())
    }

    /// One difference. Without a schema (nothing describes the place) a
    /// side that has a value is the placeholder.
    fn leaf(
        &mut self,
        schema: Option<&Value>,
        value: Option<&Value>,
        base: Option<&Value>,
        path: &[String],
    ) -> Result<(), ValueError> {
        let show = |side: Option<&Value>| -> Result<Option<Value>, ValueError> {
            side.map(|x| match schema {
                Some(s) => redact_node(s, x, self.placeholder, self.alloc),
                None => placeholder_value(self.placeholder, self.alloc),
            })
            .transpose()
        };
        let found = Difference {
            path: path.to_vec(),
            value: show(value)?,
            baseline: show(base)?,
        };
        self.out.push(found);
        Ok(())
    }
}

/// What a map key is compared as.
enum Part<'s> {
    /// Not data: the variant's discriminant, equal on both sides.
    Skip,
    /// A place the schema does not describe.
    Undescribed,
    /// A place with this schema and, for a declared field, its default.
    Field(&'s Value, Option<&'s Value>),
}
