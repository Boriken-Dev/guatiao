// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Following a path through a value.
//!
//! Iterative, never recursive: a path is as deep as its text says, and its
//! text comes from whoever wrote it down.

#![forbid(unsafe_code)]

use super::{Path, Segment};
use crate::value::convert::{TryAsMut, TryAsRef};
use crate::value::types::{List, Map, Value};

/// The value at `path`, or `None`.
///
/// `None` says "nothing is there", for every reason at once: a key no map
/// holds, a position past the end of a list, or a segment applied to a
/// scalar. A caller that needs to tell those apart is asking about the
/// **schema**, and [`schema::flat::resolve`](crate::schema::flat::resolve)
/// is the one that answers.
pub fn get<'v>(value: &'v Value, path: Path<'_>) -> Option<&'v Value> {
    let mut at = value;
    for segment in path {
        at = match segment {
            Segment::Field(name) => TryAsRef::<Map>::try_as_ref(at)?.get(name)?,
            Segment::Key(key) => TryAsRef::<Map>::try_as_ref(at)?.get(&key)?,
            // A number is a position in a list and a key in a map, and
            // which one it is depends on what it was applied to. See the
            // module note on `path`.
            Segment::Index(index) => match TryAsRef::<List>::try_as_ref(at) {
                Some(list) => list.get(index)?,
                None => TryAsRef::<Map>::try_as_ref(at)?.get(&index.to_string())?,
            },
        };
    }
    Some(at)
}

/// The same, to write through.
///
/// **Does not create what is missing.** A path names a place; making one
/// needs an allocator and a decision about what kind of container each
/// missing step should be, which is
/// [`schema::flat::unflatten`](crate::schema::flat::unflatten)'s job,
/// because the schema is what says.
pub fn get_mut<'v>(value: &'v mut Value, path: Path<'_>) -> Option<&'v mut Value> {
    let mut at = value;
    for segment in path {
        at = match segment {
            Segment::Field(name) => TryAsMut::<Map>::try_as_mut(at)?.get_mut(name)?,
            Segment::Key(key) => TryAsMut::<Map>::try_as_mut(at)?.get_mut(&key)?,
            Segment::Index(index) => {
                // Asked in this order because a borrow of `at` as a list
                // cannot outlive the branch that then asks for a map. The
                // check is cheap: it reads one tag.
                if TryAsRef::<List>::try_as_ref(at).is_some() {
                    TryAsMut::<List>::try_as_mut(at)?.get_mut(index)?
                } else {
                    TryAsMut::<Map>::try_as_mut(at)?.get_mut(&index.to_string())?
                }
            }
        };
    }
    Some(at)
}
