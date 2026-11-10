// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A guatiao value as a coqui-builder [`Source`].

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_builder::describe::{Kind, LeafInfo};
use coqui_builder::fill::FillErrorKind;
use coqui_builder::path::Segment;
use coqui_builder::{
    Describe, Description, Entries, FillError, KeyPath, Partial, Source, SourceError,
};
use guatiao::{List, Map, Tag, Value};

use crate::text;

/// The value at `at` below `root`: a key of a map, a position of a list, or a position read as
/// the key of its decimal text in an open map.
pub(crate) fn lookup<'v>(root: &'v Value, at: &[Segment]) -> Option<&'v Value> {
    let mut node = root;
    for segment in at {
        node = match (node.tag().ok()?, segment) {
            (Tag::GUATIAO_MAP, Segment::Key(key)) => <&Map>::try_from(node).ok()?.get(key)?,
            (Tag::GUATIAO_MAP, Segment::Index(index)) => {
                <&Map>::try_from(node).ok()?.get(&index.to_string())?
            }
            (Tag::GUATIAO_LIST, Segment::Index(index)) => {
                <&List>::try_from(node).ok()?.get(*index)?
            }
            _ => return None,
        };
    }
    Some(node)
}

/// What `node` holds as entries: a list's length, a map's keys, nothing for a scalar.
pub(crate) fn entries_of(node: &Value) -> Option<Entries> {
    match node.tag().ok()? {
        Tag::GUATIAO_LIST => Some(Entries::Len(<&List>::try_from(node).ok()?.len())),
        Tag::GUATIAO_MAP => Some(Entries::Keys(
            <&Map>::try_from(node)
                .ok()?
                .keys()
                .map(String::from)
                .collect(),
        )),
        _ => None,
    }
}

/// A guatiao [`Value`] as what a [`Partial`] is filled from.
///
/// A leaf is the value's text by the rules in [`crate::text`]; a list answers its length and
/// a map its keys. Null is a key that is not given. A list or a map where a leaf is expected
/// is a [`SourceError`], which an untagged enum with a structured arm tolerates.
#[derive(Debug, Clone, Copy)]
pub struct ValueSource<'a> {
    root: &'a Value,
}

impl<'a> ValueSource<'a> {
    /// A source over `root`.
    #[must_use]
    pub fn new(root: &'a Value) -> ValueSource<'a> {
        ValueSource { root }
    }
}

impl Source for ValueSource<'_> {
    fn value(&mut self, at: &KeyPath, leaf: &LeafInfo) -> Result<Option<String>, SourceError> {
        match lookup(self.root, at.segments()) {
            Some(node) => text::from_value(node, leaf.ty())
                .map_err(|error| SourceError::new(alloc::format!("{at}: {error}"))),
            None => Ok(None),
        }
    }

    fn entries(&mut self, at: &KeyPath) -> Result<Option<Entries>, SourceError> {
        Ok(lookup(self.root, at.segments()).and_then(entries_of))
    }

    fn unknown(&mut self, known: &dyn Fn(&KeyPath) -> bool) -> Vec<KeyPath> {
        let mut found = Vec::new();
        let mut path = KeyPath::root();
        strangers(self.root, &mut path, known, &mut found, 0);
        found
    }
}

/// How deep a value is searched for keys the description lacks.
const DEPTH: usize = 64;

/// The shallowest places below `node` that `known` does not recognise.
fn strangers(
    node: &Value,
    path: &mut KeyPath,
    known: &dyn Fn(&KeyPath) -> bool,
    found: &mut Vec<KeyPath>,
    depth: usize,
) {
    if depth >= DEPTH {
        // Deeper than this is not searched, so it is not trusted either.
        found.push(path.clone());
        return;
    }
    let mut visit = |segment: Segment, child: &Value| {
        path.push(segment);
        if known(path) {
            strangers(child, path, known, found, depth + 1);
        } else {
            found.push(path.clone());
        }
        path.pop();
    };
    match node.tag() {
        Ok(Tag::GUATIAO_MAP) => {
            if let Ok(map) = <&Map>::try_from(node) {
                for entry in map.iter() {
                    visit(Segment::Key(entry.key().to_string()), entry.value());
                }
            }
        }
        Ok(Tag::GUATIAO_LIST) => {
            if let Ok(list) = <&List>::try_from(node) {
                for (index, item) in list.iter().enumerate() {
                    visit(Segment::Index(index), item);
                }
            }
        }
        _ => {}
    }
}

/// The partly filled `T` a value holds: `Partial::fill_from` over a [`ValueSource`](crate::ValueSource) on an
/// empty partial. The keys the value holds that `T` lacks are refused under a group that
/// refuses unknown keys and dropped otherwise.
///
/// # Errors
///
/// The first [`FillError`] of the value or of a leaf.
pub fn partial<T: Describe>(value: &Value) -> Result<Partial<T>, FillError> {
    let mut partial = Partial::<T>::new();
    fill(&mut partial, value)?;
    Ok(partial)
}

/// Fills `partial` from `value`, after checking that every place the value holds has the
/// shape the description gives it: a table where there are keys, a list where there are
/// items. A source answers `None` for a place it cannot read, which would leave the key
/// unfilled and say nothing.
pub(crate) fn fill<T: Describe>(partial: &mut Partial<T>, value: &Value) -> Result<(), FillError> {
    let description = partial.description().clone();
    shape(&description, value, &mut Vec::new()).map_err(|(at, message)| {
        let mut error = FillError::new(FillErrorKind::Source(SourceError::new(message)));
        for segment in at.into_iter().rev() {
            error = error.under(segment);
        }
        error
    })?;
    partial.fill_from(&mut ValueSource::new(value))?;
    Ok(())
}

type Refusal = (Vec<Segment>, &'static str);

/// Whether `value` has the shape `description` reads it in; null is a key not given.
fn shape(description: &Description, value: &Value, at: &mut Vec<Segment>) -> Result<(), Refusal> {
    let tag = value.tag().ok();
    if tag == Some(Tag::GUATIAO_NULL) {
        return Ok(());
    }
    let refuse = |at: &[Segment], message| Err((at.to_vec(), message));
    let below = |at: &mut Vec<Segment>, segment: Segment, child: &Description, value: &Value| {
        at.push(segment);
        let checked = shape(child, value, at);
        at.pop();
        checked
    };
    match description.kind() {
        Kind::Leaf(_) => {
            if tag == Some(Tag::GUATIAO_LIST) || tag == Some(Tag::GUATIAO_MAP) {
                return refuse(at, "expected a single value");
            }
        }
        Kind::Group(group) => {
            let Ok(map) = <&Map>::try_from(value) else {
                return refuse(at, "expected a table");
            };
            for field in &group.fields {
                if let Some(child) = map.get(&field.key) {
                    below(
                        at,
                        Segment::Key(field.key.clone()),
                        &field.description,
                        child,
                    )?;
                }
            }
        }
        Kind::Variant(variant) => {
            let Ok(map) = <&Map>::try_from(value) else {
                return refuse(at, "expected a table");
            };
            let chosen = map
                .get(variant.tag())
                .and_then(|tag| text::from_value(tag, "String").ok().flatten());
            if let Some(arm) = chosen.as_deref().and_then(|name| variant.arm(name)) {
                for field in &arm.fields {
                    if let Some(child) = map.get(&field.key) {
                        below(
                            at,
                            Segment::Key(field.key.clone()),
                            &field.description,
                            child,
                        )?;
                    }
                }
            }
        }
        Kind::List(item) => {
            if let Ok(list) = <&List>::try_from(value) {
                for (index, child) in list.iter().enumerate() {
                    below(at, Segment::Index(index), item, child)?;
                }
            } else if <&Map>::try_from(value).is_err() {
                return refuse(at, "expected a list");
            }
        }
        Kind::Map(entry) => {
            let Ok(map) = <&Map>::try_from(value) else {
                return refuse(at, "expected a table");
            };
            for child in map.iter() {
                below(
                    at,
                    Segment::Key(child.key().to_string()),
                    entry,
                    child.value(),
                )?;
            }
        }
        Kind::Union(union) => {
            if tag == Some(Tag::GUATIAO_MAP) {
                shape(union.structured(), value, at)?;
            } else if tag == Some(Tag::GUATIAO_LIST) {
                if !matches!(union.structured().kind(), Kind::List(_)) {
                    return refuse(at, "expected a value or a table");
                }
                shape(union.structured(), value, at)?;
            }
        }
        _ => {}
    }
    Ok(())
}
