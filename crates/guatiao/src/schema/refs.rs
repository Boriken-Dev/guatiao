// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Replacing local `$ref`s with the schemas they name.
//!
//! The readers in [`super::read`] are borrowed views of one subschema and
//! hold no root to look a reference up in, so a `$ref` reads as a kind
//! they skip. A generator that writes shared definitions (serde's
//! `schemars` puts every struct under `$defs`) is read by inlining them
//! first: [`inline_refs`] copies the document with every
//! `{"$ref": "#/$defs/Name"}` replaced by a copy of `Name`'s schema.
//!
//! **What it leaves alone is skipped, not refused**, which is the rule
//! every reader here follows: a reference to somewhere else, to a name
//! that is not defined, back into a definition already being expanded (a
//! recursive type has no finite inline form) or past the budget stays as
//! written and reads as a kind to skip.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec::Vec;

use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::{ToValue, TryAsMut, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::{List, Map, Value};

/// The spellings a document's definitions go under, as the pointer
/// prefix a reference to one starts with.
const POINTER_PREFIXES: [(&str, &str); 2] =
    [("#/$defs/", vocab::DEFS), ("#/definitions/", "definitions")];

/// How many references one call expands. A document of definitions that
/// each name the previous one twice is a handful of bytes and
/// exponentially many schemas once expanded; past this the remaining
/// references stay as written.
const BUDGET: usize = 10_000;

/// A copy of `schema` with every local reference replaced by the schema it
/// names.
///
/// **What it leaves alone is skipped, not refused**: a reference to
/// somewhere else, to a name that is not defined, back into a definition
/// already being expanded (a recursive type has no finite inline form) or
/// past the expansion budget stays as written and reads as a kind to skip.
///
/// The root's `$defs` (and the older `definitions`) are not copied: after
/// inlining nothing refers to them. A schema with sibling keywords next
/// to its `$ref` (`description`, say) keeps them, over the target's.
///
/// Built with the crate's allocator; [`inline_refs_in`] names one.
///
/// # Errors
///
/// [`ValueError::TooDeep`] when the result would nest past `MAX_DEPTH`,
/// and the allocator's refusal.
pub fn inline_refs(schema: &Value) -> Result<Value, ValueError> {
    inline_refs_in(schema, Alloc::rust())
}

/// [`inline_refs`], building the copy through `alloc`.
pub fn inline_refs_in(schema: &Value, alloc: Alloc) -> Result<Value, ValueError> {
    let root = TryAsRef::<Map>::try_as_ref(schema);
    let mut inliner = Inliner {
        root,
        alloc,
        active: Vec::new(),
        budget: BUDGET,
    };
    inliner.copy(schema, 0, true)
}

struct Inliner<'a> {
    root: Option<&'a Map>,
    alloc: Alloc,
    /// The definitions being expanded right now, innermost last.
    active: Vec<String>,
    budget: usize,
}

impl<'a> Inliner<'a> {
    /// The definition `schema`'s `$ref` names, with its name, when this
    /// can expand it.
    fn target(&self, schema: &Map) -> Option<(String, &'a Value)> {
        let reference = schema
            .get(vocab::REF)
            .and_then(TryAsRef::<str>::try_as_ref)?;
        for (prefix, key) in POINTER_PREFIXES {
            let Some(escaped) = reference.strip_prefix(prefix) else {
                continue;
            };
            // JSON Pointer's two escapes, `~1` before `~0`.
            let name = escaped.replace("~1", "/").replace("~0", "~");
            if self.active.contains(&name) {
                return None;
            }
            let found = self
                .root?
                .get(key)
                .and_then(TryAsRef::<Map>::try_as_ref)?
                .get(&name)?;
            return Some((name, found));
        }
        None
    }

    fn copy(&mut self, v: &'a Value, depth: u32, is_root: bool) -> Result<Value, ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        let alloc = self.alloc;
        if let Some(map) = TryAsRef::<Map>::try_as_ref(v) {
            if self.budget > 0
                && let Some((name, target)) = self.target(map)
            {
                self.budget -= 1;
                self.active.push(name);
                let expanded = self.copy(target, depth + 1, false);
                self.active.pop();
                let mut expanded = expanded?;
                // What sits beside the `$ref` is the referrer's own and
                // wins over the target's.
                if let Some(into) = TryAsMut::<Map>::try_as_mut(&mut expanded) {
                    for entry in map {
                        if entry.key() != vocab::REF {
                            into.set_in(
                                entry.key(),
                                self.copy(entry.value(), depth + 1, false)?,
                                alloc,
                            )?;
                        }
                    }
                }
                return Ok(expanded);
            }
            let mut out = Map::new_in(alloc);
            for entry in map {
                if is_root && POINTER_PREFIXES.iter().any(|(_, key)| *key == entry.key()) {
                    continue;
                }
                let child = self.copy(entry.value(), depth + 1, false)?;
                out.set_in(entry.key(), child, alloc)?;
            }
            return Ok(out.into());
        }
        if let Some(list) = TryAsRef::<List>::try_as_ref(v) {
            let mut out = List::new_in(alloc);
            for item in list.iter() {
                out.push_in(self.copy(item, depth + 1, false)?, alloc)?;
            }
            return Ok(out.into());
        }
        v.to_value_in(alloc)
    }
}
