// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Projecting a tagged value onto flat `key -> text` storage.
//!
//! The discriminant goes under the field's own key and each payload field
//! under `<key>.<field>`. That is also the URI spelling —
//! `?auth=userpass&auth.username=alice` — which is what lets a command
//! line select an arm at all.
//!
//! # The deletion is first, and unconditional
//!
//! [`flatten`] clears every `<key>.*` entry before it writes, and
//! [`unflatten`] drops any it finds that the selected arm does not
//! declare. That is what makes "an arm with no fields stores nothing" a
//! property of the code rather than a convention somebody has to remember.
//!
//! Move the deletion after the writes, or make it conditional on the new
//! arm, and a field that two arms happen to share keeps the old arm's
//! value. Same bug, longer fuse.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use super::ValidationError;
use super::read::{FieldRef, Kind, SchemaRef};
use super::validate::text_of;
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::read::str_or;
use crate::value::types::{Tag, Value};

/// Between a field's key and one of its payload fields.
pub const SEPARATOR: char = '.';

/// Writes a tagged value into flat storage. Answers whether it applied.
pub fn flatten(field: FieldRef<'_>, value: &Value, store: &mut BTreeMap<String, String>) -> bool {
    let kind = field.kind();
    let Kind::Variant { tag, .. } = kind else {
        return false;
    };
    if value.tag() != Ok(Tag::GUATIAO_MAP) {
        return false;
    }
    let chosen = str_or(value.get(tag), "");
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        return false;
    };

    // FIRST, AND UNCONDITIONAL. See the module note.
    let prefix = format!("{}{SEPARATOR}", field.key());
    store.retain(|key, _| !key.starts_with(&prefix));

    store.insert(field.key().to_string(), chosen.to_string());
    for member in arm.fields() {
        if let Some(present) = value.get(member.key())
            && let Some(text) = scalar_text(present)
        {
            store.insert(format!("{prefix}{}", member.key()), text);
        }
    }
    true
}

/// Reads a tagged value back out of flat storage.
///
/// `None` when the field is not tagged, the discriminant is absent, or
/// the stored discriminant names no declared arm. All three mean "there is
/// no tagged value here", which is a different thing from an arm with an
/// empty payload and must not be confused with it.
///
/// **Any `<key>.*` entry the selected arm does not declare is dropped**,
/// which is the read half of the rule above: a record that picked up a
/// stale field some other way still reads back as the arm says it is.
pub fn unflatten(
    alloc: Alloc,
    field: FieldRef<'_>,
    store: &BTreeMap<String, String>,
) -> Option<Value> {
    let kind = field.kind();
    let Kind::Variant { tag, .. } = kind else {
        return None;
    };
    let chosen = store.get(field.key())?;
    let arm = kind.arms().find(|a| a.value() == chosen)?;

    let mut map = Value::map_in(alloc);
    // The discriminant goes in FIRST, so re-emission is byte-stable: a map
    // is insertion-ordered by contract.
    map.set(tag, Value::string_in(alloc, chosen).ok()?).ok()?;
    let prefix = format!("{}{SEPARATOR}", field.key());
    for member in arm.fields() {
        if let Some(text) = store.get(&format!("{prefix}{}", member.key()))
            && let Ok(v) = Value::string_in(alloc, text)
        {
            let _ = map.set(member.key(), v);
        }
    }
    Some(map)
}

/// Every flat key this field can occupy: its own, plus one per field of
/// every arm.
///
/// For a caller that has to decide whether a key it is holding belongs to
/// this field at all — a config reader partitioning a flat record, say.
pub fn keys(field: FieldRef<'_>) -> Vec<String> {
    let mut out = vec![field.key().to_string()];
    for arm in field.kind().arms() {
        // `member`, not `field`: the inner binding would shadow the
        // parameter and the key would be built from one of them twice.
        // It compiles either way, which is the whole danger.
        for member in arm.fields() {
            let key = format!("{}{SEPARATOR}{}", field.key(), member.key());
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    out
}

/// Splits a flat key into `(owner key, member key)`, if it is one.
pub fn split(key: &str) -> Option<(&str, &str)> {
    key.split_once(SEPARATOR)
}

/// The field governing a flat key, following one level of projection.
///
/// So `sensitive` on `auth.password` resolves to the arm field's own flag,
/// which is what lets a field-level encryption path keep working with no
/// new concept: it already asks the schema whether a key is a secret, and
/// now the schema can answer for a dotted one.
pub fn resolve<'a>(schema: SchemaRef<'a>, key: &str) -> Option<FieldRef<'a>> {
    if let Some(direct) = schema.find(key) {
        return Some(direct);
    }
    // Two different things, and the rename briefly gave them one name:
    // `leaf` is the key AFTER the dot, `owner` is the field the dot hangs
    // off. Comparing a field against a key compiles nowhere, which is the
    // only reason this was caught rather than shipped.
    let (parent, leaf) = split(key)?;
    let owner = schema.find(parent)?;
    owner
        .kind()
        .arms()
        .find_map(|arm| arm.fields().find(|f| f.key() == leaf))
}

/// The field governing a flat key **in a particular store**: the arm a
/// payload key belongs to must be the one the store selects.
///
/// [`resolve`] answers for the schema alone, and for a schema alone
/// `auth.username` is a field of SOME arm. A store has said which arm --
/// `selected("auth")` is the discriminant's text there, or `None` when
/// nothing was written and the field's default decides -- and a payload
/// key the chosen arm does not declare is an error rather than something
/// to drop. Ignoring it is how a credential ends up stored under a key
/// the chosen arm does not have.
///
/// A dotted key whose stem is not a tagged field is an unknown key, not a
/// payload field: reporting it as one lists the keys that do exist.
pub fn resolve_in<'a>(
    schema: SchemaRef<'a>,
    key: &str,
    selected: impl Fn(&str) -> Option<String>,
) -> Result<FieldRef<'a>, ValidationError> {
    if let Some(direct) = schema.find(key) {
        return Ok(direct);
    }
    let unknown = || ValidationError::UnknownOption {
        key: key.to_string(),
        known: schema.fields().map(|f| f.key().to_string()).collect(),
    };
    let Some((parent, leaf)) = split(key) else {
        return Err(unknown());
    };
    let Some(owner) = schema.find(parent) else {
        return Err(unknown());
    };
    let kind = owner.kind();
    if !matches!(kind, Kind::Variant { .. }) {
        return Err(unknown());
    }
    let chosen = selected(parent)
        .filter(|t| !t.is_empty())
        .or_else(|| owner.default().map(text_of))
        .unwrap_or_default();
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        // Either the tag names no arm -- which the tag's own entry
        // reports if it was given -- or nothing selected one and there is
        // no default. Naming the tag is the actionable half: the payload
        // cannot be checked until the arm is known.
        let names: Vec<&str> = kind.arms().map(|a| a.value()).collect();
        return Err(ValidationError::BadValue {
            key: parent.to_string(),
            expected: format!(
                "a value naming one of {} — '{key}' belongs to an arm and cannot be \
                 checked until one is chosen",
                names.join(", ")
            ),
        });
    };
    arm.fields().find(|f| f.key() == leaf).ok_or_else(|| {
        // Names the ARM rather than listing keys, for a measured reason:
        // the empty arm is the interesting case, and its list of accepted
        // payload keys is EMPTY -- a sentence that stops mid-air. The key
        // is not wrong in general, it is wrong for what was chosen.
        let accepted: Vec<&str> = arm.fields().map(|f| f.key()).collect();
        ValidationError::BadValue {
            key: key.to_string(),
            expected: if accepted.is_empty() {
                format!("nothing — the '{chosen}' arm of '{parent}' stores no fields at all")
            } else {
                format!(
                    "nothing — the '{chosen}' arm of '{parent}' accepts only {}",
                    accepted.join(", ")
                )
            },
        }
    })
}

/// Whether a flat key names a secret, following the projection.
///
/// Worth a named function rather than `resolve(..).is_sensitive()` for one
/// reason: an unknown key answers **false**, and that is the wrong default
/// to arrive at by accident. It is stated here once so a caller does not
/// have to.
pub fn is_sensitive(schema: SchemaRef<'_>, key: &str) -> bool {
    resolve(schema, key).is_some_and(|o| o.is_sensitive())
}

/// Rejects a declaration whose field key contains the separator, which
/// would make a payload key ambiguous with a field key.
///
/// Answers the offending key. A declaration bug, so it is caught at
/// declaration rather than tolerated at read time.
pub fn check_keys(schema: SchemaRef<'_>) -> Result<(), String> {
    for field in schema.fields() {
        if field.key().contains(SEPARATOR) {
            return Err(field.key().to_string());
        }
    }
    Ok(())
}

/// The text spelling of a scalar, or `None` for a container.
///
/// A nested container inside an arm has no flat spelling, so it is not
/// written rather than written wrongly.
fn scalar_text(value: &Value) -> Option<String> {
    match value.tag() {
        Ok(Tag::GUATIAO_BOOL | Tag::GUATIAO_NUMBER | Tag::GUATIAO_STRING)
        | Ok(Tag::GUATIAO_BYTES) => Some(text_of(value)),
        _ => None,
    }
}

/// The error type a caller of [`unflatten`] sees when an allocation fails.
///
/// Re-exported so a consumer does not have to name the module the value
/// model lives in today, which is moving.
pub type Error = ValueError;
