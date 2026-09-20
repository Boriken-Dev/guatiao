// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `x-merge`: a field declaring how its own value combines across
//! config layers.
//!
//! # Why a declaration and not just a call-site choice
//!
//! [`MergeMode`] is a call-site default: the caller merging two maps
//! picks one, and it applies to every key. That is enough to be useful
//! and it is deliberately all the merge itself needs — but it is not
//! enough to be *right*, because **the caller merging two maps does not
//! know what the values mean.**
//!
//! The declarer does. Whoever wrote the field knows whether its list is
//! an unordered tag set — where [`MergeMode::Deep`]'s union is the
//! correct answer — or an ordered fallback chain, where a later layer
//! must replace it wholesale or the fallback order becomes nonsense. That
//! knowledge lives with the declaration, so this is where it is read.
//!
//! # Why it lives here and not in `schema`
//!
//! **The schema knows nothing about merging.** It carries `x-merge` the
//! way it carries every other annotation: as an opaque value under a key
//! outside its vocabulary. This module is the only thing that gives that
//! key a meaning, and it sits beside the merge because that is the
//! direction the dependency has to run — merging is a consumer concern
//! layered over a schema, never something a schema has to know about.
//! When the merge moves to a crate of its own, this moves with it and
//! nothing in the core has to change.
//!
//! # The mechanism, and why it is an annotation rather than a new keyword
//!
//! Anything outside `schema::vocab` is an annotation: carried, never
//! interpreted. `x-merge` is one of those — a string value, one of
//! `"simple"`, `"substitute"` or `"deep"`, optionally `"deep+mergelists"`.
//!
//! Using the existing extension channel rather than adding a keyword to
//! the vocabulary is what that channel is for: a new keyword is something
//! every reader has to learn, and an annotation a reader has never heard
//! of costs it nothing.

#![forbid(unsafe_code)]

use crate::schema::flat::SEPARATOR;
use crate::schema::read::{FieldRef, Kind, SchemaRef};
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::read::str_or;
use crate::value::types::Value;
use crate::{MergeError, MergeMode, MergeOptions, MergeOverrides};

/// The annotation key a field declares its merge mode under.
///
/// `x-` prefixed to match the JSON Schema extension convention, so a
/// schema that round-trips through a text format carries the declaration
/// without special-casing.
pub const X_MERGE: &str = "x-merge";

/// The spelling that turns [`MergeMode::Deep`]'s positional map-in-list
/// merging on, appended to the mode: `"deep+mergelists"`.
///
/// A suffix on the mode string rather than a second annotation key,
/// because it is meaningless without `deep` — a separate
/// `x-merge-lists: true` beside `x-merge: substitute` would be a
/// declaration with no effect and no way to say so.
const MERGELISTS_SUFFIX: &str = "+mergelists";

/// A declared merge mode plus the sub-options it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredMerge {
    /// The mode itself.
    pub mode: MergeMode,
    /// The sub-options, which today means only `mergelists`.
    pub options: MergeOptions,
}

/// Parses an `x-merge` string.
///
/// `None` for anything unrecognised. Unrecognised is treated as *not
/// declared* rather than as an error, deliberately: a schema written
/// against a newer vocabulary must still be readable by an older
/// consumer, and the fallback — the call-site mode — is a defined,
/// sensible answer rather than a failure. Same forward-compatibility
/// contract the schema reader carries for a kind it has never heard of.
///
/// Case-insensitive, and surrounding whitespace is ignored, because these
/// are hand-written in declarations and in config files.
pub fn parse_mode(text: &str) -> Option<DeclaredMerge> {
    let text = text.trim().to_ascii_lowercase();
    let (name, mergelists) = match text.strip_suffix(MERGELISTS_SUFFIX) {
        Some(name) => (name, true),
        None => (text.as_str(), false),
    };
    let mode = match name {
        "simple" => MergeMode::Simple,
        "substitute" => MergeMode::Substitute,
        "deep" => MergeMode::Deep,
        _ => return None,
    };
    Some(DeclaredMerge {
        mode,
        options: MergeOptions::new().with_mergelists(mergelists),
    })
}

/// The mode declared on one field, if any.
///
/// Reads only the `x-merge` annotation and only as a string. A non-string
/// value is ignored for the same reason an unrecognised spelling is:
/// falling back to the call-site mode is a defined answer, and refusing
/// to merge at all because an annotation was the wrong kind would let an
/// advisory hint break a working config.
pub fn declared_for(field: FieldRef<'_>) -> Option<DeclaredMerge> {
    parse_mode(str_or(field.extra(X_MERGE), ""))
}

/// Every `x-merge` declaration in `schema`, as the lookup
/// [`MergeMode::merge_with`] takes.
///
/// Keyed by the **path** the field occupies in a values map, which is the
/// spelling the merge matches against: a top-level field's own key, and a
/// field of a nested object joined to its owner's with a dot
/// (`"tls.ciphers"`). A field key may not contain the separator itself —
/// [`flat::check_keys`](crate::schema::flat::check_keys) refuses that at
/// declaration — so a path here has exactly one reading.
///
/// **Nested objects are walked**, because the declaration lives with the
/// field and a field two levels down declares just as meaningfully as one
/// at the top. A variant's arms are not: two arms may declare different
/// modes for one path, and there is no arm selected at the time a merge
/// consults this table.
///
/// Fields with no declaration are simply absent, and absent means "take
/// the call-site mode" — so a schema that declares nothing produces an
/// empty override set and changes nothing.
pub fn merge_overrides(schema: SchemaRef<'_>) -> MergeOverrides {
    let mut overrides = MergeOverrides::new();
    collect_overrides(schema.fields(), "", &mut overrides);
    overrides
}

/// The recursion behind [`merge_overrides`], over one level of fields.
///
/// `prefix` is the path of the object these fields belong to, empty at
/// the root.
fn collect_overrides<'a>(
    fields: impl Iterator<Item = FieldRef<'a>>,
    prefix: &str,
    overrides: &mut MergeOverrides,
) {
    for field in fields {
        let path = if prefix.is_empty() {
            field.key().to_string()
        } else {
            format!("{prefix}{SEPARATOR}{}", field.key())
        };
        if let Some(declared) = declared_for(field) {
            overrides.set(path.clone(), declared.mode);
        }
        if let Kind::Map(_) = field.kind() {
            collect_overrides(field.kind().fields(), &path, overrides);
        }
    }
}

/// The `mergelists` sub-option, resolved across a whole schema.
///
/// # Why this is one flag rather than one per field
///
/// `mergelists` is carried on [`MergeOptions`], which applies to the
/// *whole* merge rather than per path. That is not an oversight there: it
/// changes how map elements inside a list are matched up, and a merge
/// where the answer to "does element 0 merge with element 0?" varied by
/// which subtree you were in would be very hard to reason about.
///
/// So a schema resolves it once: **on if any field asked for it.** The
/// alternative — silently ignoring the declaration on fields that asked
/// — would make a written declaration do nothing, which is worse than
/// applying it slightly more widely than asked. A caller that needs the
/// finer distinction merges the subtrees separately.
pub fn merge_options(schema: SchemaRef<'_>) -> MergeOptions {
    let any = any_mergelists(schema.fields());
    MergeOptions::new().with_mergelists(any)
}

/// Whether any field at or below this level asked for `mergelists`.
///
/// Nested objects are walked, for the reason [`merge_overrides`] walks
/// them: a declaration is a declaration wherever it sits.
fn any_mergelists<'a>(fields: impl Iterator<Item = FieldRef<'a>>) -> bool {
    fields.into_iter().any(|field| {
        declared_for(field).is_some_and(|d| d.options.mergelists)
            || (matches!(field.kind(), Kind::Map(_)) && any_mergelists(field.kind().fields()))
    })
}

/// Merges two values with this schema's declarations applied over
/// `call_site_mode`.
///
/// The one call a consumer that has both a schema and two maps actually
/// wants: it wires [`merge_overrides`] and [`merge_options`] into
/// [`MergeMode::merge_with`] so neither can be forgotten.
///
/// **A declared mode beats `call_site_mode` for its key**, and only for
/// its key. Every key the schema does not declare — including every key
/// the schema does not mention at all, since a values map may legitimately
/// carry more than the schema describes — takes `call_site_mode`.
pub fn merge_with_schema(
    schema: SchemaRef<'_>,
    call_site_mode: MergeMode,
    earlier: &Value,
    later: &Value,
    alloc: Alloc,
) -> Result<Value, MergeError> {
    let overrides = merge_overrides(schema);
    let fields = merge_options(schema);
    call_site_mode.merge_with(earlier, later, alloc, fields, Some(&overrides))
}

/// Declares `mode` on a field, as the string [`parse_mode`] reads.
///
/// Exists so a declaration site never spells the string by hand — a typo
/// in `"substitue"` would parse as "not declared" and silently fall back
/// to the call-site mode, which is the failure mode hardest to notice.
pub fn annotation(alloc: Alloc, declared: DeclaredMerge) -> Result<Value, ValueError> {
    let name = match declared.mode {
        MergeMode::Simple => "simple",
        MergeMode::Deep => "deep",
        // `MergeMode` is `#[non_exhaustive]`, so a mode appended upstream
        // lands here. Spelling it `substitute` is right for the current
        // enum and is also the safe answer for an unknown one: it is the
        // documented default, so a declaration that fell through would
        // assert exactly what would have happened anyway.
        _ => "substitute",
    };
    if declared.options.mergelists {
        Value::string_in(alloc, &format!("{name}{MERGELISTS_SUFFIX}"))
    } else {
        Value::string_in(alloc, name)
    }
}
