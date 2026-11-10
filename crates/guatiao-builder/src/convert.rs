// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A described value to a guatiao value, and back.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use coqui_builder::describe::{Field, Kind};
use coqui_builder::path::Segment;
use coqui_builder::{Describe, Description, Entries, FillError, Inspect, Missing, Partial};
use guatiao::{Alloc, List, Map, Text, Value, ValueError};

use crate::text;

/// How deep a value is written: a description is finite, but a recursive one is not.
const DEPTH: usize = 128;

/// What `value` is as a guatiao value, built in `alloc`: the description walked through
/// [`Inspect`], each leaf written by the rules in [`crate::text`], a secret's text included
/// (it is the value). A key that holds nothing is left out; the text that writes a `None` is
/// the value of its key.
///
/// # Errors
///
/// [`ValueError`] when an allocation fails or the description nests too deeply.
pub fn to_value_in<T: Describe + Inspect>(value: &T, alloc: Alloc) -> Result<Value, ValueError> {
    let description = T::description();
    let mut path = Vec::new();
    Ok(place(&description, value, &mut path, alloc, 0)?
        .unwrap_or_else(|| Map::new_in(alloc).into()))
}

/// [`to_value_in`] through guatiao's own allocator.
///
/// # Errors
///
/// As [`to_value_in`].
pub fn to_value<T: Describe + Inspect>(value: &T) -> Result<Value, ValueError> {
    to_value_in(value, Alloc::rust())
}

/// The value at `path`, `None` when the view holds nothing there.
fn place(
    description: &Description,
    view: &dyn Inspect,
    path: &mut Vec<Segment>,
    alloc: Alloc,
    depth: usize,
) -> Result<Option<Value>, ValueError> {
    if depth >= DEPTH {
        return Err(ValueError::TooDeep);
    }
    let leaf = |view: &dyn Inspect, path: &[Segment]| -> Result<Option<Value>, ValueError> {
        view.at(path)
            .map(|datum| text::to_value_in(datum.ty(), datum.text(), alloc))
            .transpose()
    };
    match description.kind() {
        Kind::Leaf(_) => leaf(view, path),
        Kind::Group(group) => {
            if view.entries(path).is_none() {
                return Ok(None);
            }
            fields(&group.fields, None, view, path, alloc, depth).map(Some)
        }
        Kind::Variant(variant) => {
            path.push(Segment::Key(variant.tag().into()));
            let tag = view.at(path).map(|datum| datum.text().to_string());
            path.pop();
            let Some(tag) = tag else { return Ok(None) };
            let Some(arm) = variant.arm(&tag) else {
                return Ok(None);
            };
            let named = (variant.tag(), tag.as_str());
            fields(&arm.fields, Some(named), view, path, alloc, depth).map(Some)
        }
        Kind::List(item) => {
            let Some(Entries::Len(count)) = view.entries(path) else {
                return Ok(None);
            };
            let mut list = List::new_in(alloc);
            for index in 0..count {
                path.push(Segment::Index(index));
                let written = place(item, view, path, alloc, depth + 1);
                path.pop();
                // A list has no hole to leave, and dropping the item would move the rest.
                list.push_in(written?.ok_or(ValueError::Unrepresentable)?, alloc)?;
            }
            Ok(Some(list.into()))
        }
        Kind::Map(entry) => {
            let Some(Entries::Keys(keys)) = view.entries(path) else {
                return Ok(None);
            };
            let mut map = Map::new_in(alloc);
            for key in keys {
                path.push(Segment::Key(key.clone()));
                let written = place(entry, view, path, alloc, depth + 1);
                path.pop();
                if let Some(written) = written? {
                    map.set_in(&key, written, alloc)?;
                }
            }
            Ok(Some(map.into()))
        }
        Kind::Union(union) => {
            if view.at(path).is_some() {
                leaf(view, path)
            } else {
                place(union.structured(), view, path, alloc, depth + 1)
            }
        }
        _ => Ok(None),
    }
}

/// A map of the fields the view holds, with the tag first when there is one.
fn fields(
    list: &[Field],
    tag: Option<(&str, &str)>,
    view: &dyn Inspect,
    path: &mut Vec<Segment>,
    alloc: Alloc,
    depth: usize,
) -> Result<Value, ValueError> {
    let mut map = Map::new_in(alloc);
    if let Some((key, name)) = tag {
        map.set_in(key, Value::from(Text::new_in(alloc, name)?), alloc)?;
    }
    for field in list {
        path.push(Segment::Key(field.key.clone()));
        // The text that writes a `None` is text whatever the type, even one that looks like
        // a number.
        let none = field
            .none
            .as_deref()
            .filter(|none| view.at(path).is_some_and(|datum| datum.text() == *none));
        let written = match none {
            Some(none) => Text::new_in(alloc, none).map(|text| Some(Value::from(text))),
            None => place(&field.description, view, path, alloc, depth + 1),
        };
        path.pop();
        if let Some(written) = written? {
            map.set_in(&field.key, written, alloc)?;
        }
    }
    Ok(map.into())
}

/// Why a described type could not be read from a value.
#[derive(Debug)]
#[non_exhaustive]
pub enum FromError {
    /// A key or a leaf the value holds was refused.
    Fill(FillError),
    /// Keys the type needs were not given.
    Missing(Missing),
}

impl fmt::Display for FromError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FromError::Fill(error) => write!(f, "{error}"),
            FromError::Missing(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for FromError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            FromError::Fill(error) => Some(error),
            FromError::Missing(error) => Some(error),
        }
    }
}

impl From<FillError> for FromError {
    fn from(error: FillError) -> FromError {
        FromError::Fill(error)
    }
}

impl From<Missing> for FromError {
    fn from(error: Missing) -> FromError {
        FromError::Missing(error)
    }
}

/// The `T` a value holds: filled from a [`ValueSource`](crate::ValueSource) and built.
///
/// # Errors
///
/// [`FromError::Fill`] for a refused key or leaf, [`FromError::Missing`] naming every key
/// the value lacked.
pub fn from_value<T: Describe>(value: &Value) -> Result<T, FromError> {
    Ok(crate::source::partial::<T>(value)?.build()?)
}

/// `base` with the keys a value holds laid over it.
///
/// # Errors
///
/// As [`from_value`], except that nothing is missing: `base` holds every key.
pub fn from_value_over<T: Describe>(base: T, value: &Value) -> Result<T, FromError> {
    let mut partial = Partial::from(base);
    crate::source::fill(&mut partial, value)?;
    Ok(partial.build()?)
}

/// The key path of a refusal as text, empty for the root.
pub(crate) fn at_of(error: &FillError) -> String {
    error.at().to_string()
}
