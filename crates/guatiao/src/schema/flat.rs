// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Projecting a tagged value onto flat `key -> text` storage.
//!
//! The discriminant goes under the option's own key and each payload field
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

use super::read::{FieldRef, Kind, SchemaRef};
use super::validate::text_of;
use crate::value::alloc::Alloc;
use crate::value::mutate::ValueError;
use crate::value::read::str_or;
use crate::value::types::{Tag, Value};

/// Between an option's key and one of its payload fields.
pub const SEPARATOR: char = '.';

/// Writes a tagged value into flat storage. Answers whether it applied.
pub fn flatten(option: FieldRef<'_>, value: &Value, store: &mut BTreeMap<String, String>) -> bool {
    let kind = option.kind();
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
    let prefix = format!("{}{SEPARATOR}", option.key());
    store.retain(|key, _| !key.starts_with(&prefix));

    store.insert(option.key().to_string(), chosen.to_string());
    for field in arm.fields() {
        if let Some(present) = value.get(field.key())
            && let Some(text) = scalar_text(present)
        {
            store.insert(format!("{prefix}{}", field.key()), text);
        }
    }
    true
}

/// Reads a tagged value back out of flat storage.
///
/// `None` when the option is not tagged, the discriminant is absent, or
/// the stored discriminant names no declared arm. All three mean "there is
/// no tagged value here", which is a different thing from an arm with an
/// empty payload and must not be confused with it.
///
/// **Any `<key>.*` entry the selected arm does not declare is dropped**,
/// which is the read half of the rule above: a record that picked up a
/// stale field some other way still reads back as the arm says it is.
pub fn unflatten(
    alloc: Alloc,
    option: FieldRef<'_>,
    store: &BTreeMap<String, String>,
) -> Option<Value> {
    let kind = option.kind();
    let Kind::Variant { tag, .. } = kind else {
        return None;
    };
    let chosen = store.get(option.key())?;
    let arm = kind.arms().find(|a| a.value() == chosen)?;

    let mut map = Value::map_in(alloc);
    // The discriminant goes in FIRST, so re-emission is byte-stable: a map
    // is insertion-ordered by contract.
    map.set(tag, Value::string_in(alloc, chosen).ok()?).ok()?;
    let prefix = format!("{}{SEPARATOR}", option.key());
    for field in arm.fields() {
        if let Some(text) = store.get(&format!("{prefix}{}", field.key()))
            && let Ok(v) = Value::string_in(alloc, text)
        {
            let _ = map.set(field.key(), v);
        }
    }
    Some(map)
}

/// Every flat key this option can occupy: its own, plus one per field of
/// every arm.
///
/// For a caller that has to decide whether a key it is holding belongs to
/// this option at all — a config reader partitioning a flat record, say.
pub fn keys(option: FieldRef<'_>) -> Vec<String> {
    let mut out = vec![option.key().to_string()];
    for arm in option.kind().arms() {
        for field in arm.fields() {
            let key = format!("{}{SEPARATOR}{}", option.key(), field.key());
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    out
}

/// Splits a flat key into `(option key, field key)`, if it is one.
pub fn split(key: &str) -> Option<(&str, &str)> {
    key.split_once(SEPARATOR)
}

/// The option governing a flat key, following one level of projection.
///
/// So `sensitive` on `auth.password` resolves to the arm field's own flag,
/// which is what lets a field-level encryption path keep working with no
/// new concept: it already asks the schema whether a key is a secret, and
/// now the schema can answer for a dotted one.
pub fn resolve<'a>(schema: SchemaRef<'a>, key: &str) -> Option<FieldRef<'a>> {
    if let Some(direct) = schema.find(key) {
        return Some(direct);
    }
    let (parent, field) = split(key)?;
    let option = schema.find(parent)?;
    option
        .kind()
        .arms()
        .find_map(|arm| arm.fields().find(|f| f.key() == field))
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

/// Rejects a declaration whose option key contains the separator, which
/// would make a payload key ambiguous with an option key.
///
/// Answers the offending key. A declaration bug, so it is caught at
/// declaration rather than tolerated at read time.
pub fn check_keys(schema: SchemaRef<'_>) -> Result<(), String> {
    for option in schema.options() {
        if option.key().contains(SEPARATOR) {
            return Err(option.key().to_string());
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
