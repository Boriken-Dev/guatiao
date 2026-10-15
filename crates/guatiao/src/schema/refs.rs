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

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use super::vocab;
use crate::value::alloc::Alloc;
use crate::value::convert::{ToValue, TryAsMut, TryAsRef};
use crate::value::error::{MAX_DEPTH, ValueError};
use crate::value::types::{List, Map, Number, Value};

/// The older spelling of [`vocab::DEFS`], which draft-07 generators write.
const DEFINITIONS: &str = "definitions";

/// The spellings a document's definitions go under, as the pointer
/// prefix a reference to one starts with.
const POINTER_PREFIXES: [(&str, &str); 2] =
    [("#/$defs/", vocab::DEFS), ("#/definitions/", DEFINITIONS)];

/// Keywords whose value is an instance rather than a schema: a `$ref`
/// inside one is data, and copied as written.
const DATA_KEYWORDS: [&str; 4] = [vocab::DEFAULT, vocab::CONST, vocab::EXAMPLES, vocab::ENUM];

/// Keywords whose value maps names to schemas: a key there is a name, and
/// a map there is never itself a reference.
const NAME_MAPS: [&str; 5] = [
    vocab::PROPERTIES,
    vocab::DEFS,
    DEFINITIONS,
    "patternProperties",
    "dependentSchemas",
];

/// The budget is this many times the document's own size, plus
/// [`BUDGET_FLOOR`]. Sizes count one per node and one per byte of text, so
/// a definition holding a long description costs what copying it costs.
const BUDGET_FACTOR: usize = 32;

/// What any document may grow by, however small it is.
const BUDGET_FLOOR: usize = 256 * 1024;

/// A copy of `schema` with every local reference replaced by the schema it
/// names.
///
/// **What it leaves alone is skipped, not refused**: a reference to
/// somewhere else, to a name that is not defined, back into a definition
/// already being expanded (a recursive type has no finite inline form) or
/// past the budget stays as written and reads as a kind to skip.
///
/// **The budget is on what is copied**, counted as nodes plus bytes of
/// text: 32 times the document's own size plus 256 Ki. A reference is
/// followed only while its definition still fits, so definitions that
/// each name the one before twice -- a handful of bytes, exponentially
/// many schemas once expanded -- stop growing at that size. A reference
/// that names another reference is followed in place and is not a level
/// of nesting, so a long chain of aliases resolves.
///
/// Under `default`, `const`, `examples` and `enum` the value is an
/// instance, not a schema, and a `{"$ref": ..}` there is copied as data.
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
    let mut inliner = Inliner {
        root: TryAsRef::<Map>::try_as_ref(schema),
        alloc,
        active: BTreeSet::new(),
        sizes: BTreeMap::new(),
        budget: size(schema)
            .saturating_mul(BUDGET_FACTOR)
            .saturating_add(BUDGET_FLOOR),
        spent: 0,
    };
    inliner.schema(schema, 0, true)
}

/// What copying `v` costs: one per node and one per byte of text, a key's
/// included. A loop, because the document is not ours to bound.
fn size(v: &Value) -> usize {
    let mut total = 0usize;
    let mut stack = alloc::vec![v];
    while let Some(v) = stack.pop() {
        total = total.saturating_add(1);
        if let Some(map) = TryAsRef::<Map>::try_as_ref(v) {
            for entry in map {
                total = total.saturating_add(entry.key().len());
                stack.push(entry.value());
            }
        } else if let Some(list) = TryAsRef::<List>::try_as_ref(v) {
            stack.extend(list.iter());
        } else if let Some(text) = TryAsRef::<str>::try_as_ref(v) {
            total = total.saturating_add(text.len());
        } else if let Some(number) = TryAsRef::<Number>::try_as_ref(v) {
            total = total.saturating_add(AsRef::<str>::as_ref(number).len());
        } else if let Some(bytes) = TryAsRef::<[u8]>::try_as_ref(v) {
            total = total.saturating_add(bytes.len());
        }
    }
    total
}

struct Inliner<'a> {
    root: Option<&'a Map>,
    alloc: Alloc,
    /// The definitions being expanded right now.
    active: BTreeSet<String>,
    /// The size of each definition as written, measured once.
    sizes: BTreeMap<String, usize>,
    budget: usize,
    spent: usize,
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

    /// Whether the definition `name` still fits what is left of the budget.
    fn affords(&mut self, name: &str, target: &Value) -> bool {
        let cost = *self
            .sizes
            .entry(name.into())
            .or_insert_with(|| size(target));
        self.spent.saturating_add(cost) <= self.budget
    }

    fn charge(&mut self, units: usize) {
        self.spent = self.spent.saturating_add(units);
    }

    /// `v` as a schema: a reference is followed, through any chain of
    /// references, in place.
    fn schema(&mut self, v: &'a Value, depth: u32, is_root: bool) -> Result<Value, ValueError> {
        if depth >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        // Each hop of a chain replaces the node where it stands, so it is
        // not a level of nesting: followed in a loop, not by recursion.
        let mut referrers: Vec<&'a Map> = Vec::new();
        let mut followed: Vec<String> = Vec::new();
        let mut current = v;
        while let Some(map) = TryAsRef::<Map>::try_as_ref(current) {
            let Some((name, target)) = self.target(map) else {
                break;
            };
            if !self.affords(&name, target) {
                break;
            }
            referrers.push(map);
            self.active.insert(name.clone());
            followed.push(name);
            current = target;
        }
        let expanded = self.expand(current, &referrers, depth, is_root);
        for name in &followed {
            self.active.remove(name);
        }
        expanded
    }

    /// The schema a chain of references ended at, with what sits beside
    /// each `$ref` laid over it, the outermost referrer's last so that it
    /// wins.
    fn expand(
        &mut self,
        current: &'a Value,
        referrers: &[&'a Map],
        depth: u32,
        is_root: bool,
    ) -> Result<Value, ValueError> {
        let alloc = self.alloc;
        let mut out = match TryAsRef::<Map>::try_as_ref(current) {
            Some(map) => self.object(map, depth, is_root && referrers.is_empty())?,
            None => self.data(current)?,
        };
        // A chain that ended at something other than a map has nowhere to
        // put the referrers' own keywords.
        if TryAsRef::<Map>::try_as_ref(&out).is_none() {
            return Ok(out);
        }
        for (index, referrer) in referrers.iter().enumerate().rev() {
            for entry in *referrer {
                let key = entry.key();
                if key == vocab::REF || (is_root && index == 0 && is_definitions(key)) {
                    continue;
                }
                let child = self.entry(key, entry.value(), depth)?;
                if let Some(into) = TryAsMut::<Map>::try_as_mut(&mut out) {
                    into.set_in(key, child, alloc)?;
                }
            }
        }
        Ok(out)
    }

    /// A schema map, entry by entry.
    fn object(&mut self, map: &'a Map, depth: u32, is_root: bool) -> Result<Value, ValueError> {
        let alloc = self.alloc;
        self.charge(1);
        let mut out = Map::new_in(alloc);
        for entry in map {
            if is_root && is_definitions(entry.key()) {
                continue;
            }
            self.charge(entry.key().len());
            let child = self.entry(entry.key(), entry.value(), depth)?;
            out.set_in(entry.key(), child, alloc)?;
        }
        Ok(out.into())
    }

    /// The value under `key` in a schema map, read as that keyword says.
    fn entry(&mut self, key: &str, value: &'a Value, depth: u32) -> Result<Value, ValueError> {
        let alloc = self.alloc;
        if DATA_KEYWORDS.contains(&key) {
            return self.data(value);
        }
        if depth + 1 >= MAX_DEPTH {
            return Err(ValueError::TooDeep);
        }
        if NAME_MAPS.contains(&key)
            && let Some(names) = TryAsRef::<Map>::try_as_ref(value)
        {
            self.charge(1);
            let mut out = Map::new_in(alloc);
            for named in names {
                self.charge(named.key().len());
                let child = self.schema(named.value(), depth + 2, false)?;
                out.set_in(named.key(), child, alloc)?;
            }
            return Ok(out.into());
        }
        if let Some(list) = TryAsRef::<List>::try_as_ref(value) {
            self.charge(1);
            let mut out = List::new_in(alloc);
            for item in list.iter() {
                out.push_in(self.schema(item, depth + 2, false)?, alloc)?;
            }
            return Ok(out.into());
        }
        self.schema(value, depth + 1, false)
    }

    /// `v` copied exactly as written.
    fn data(&mut self, v: &Value) -> Result<Value, ValueError> {
        self.charge(size(v));
        v.to_value_in(self.alloc)
    }
}

fn is_definitions(key: &str) -> bool {
    key == vocab::DEFS || key == DEFINITIONS
}
