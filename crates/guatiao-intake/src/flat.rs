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

use guatiao::value::convert::TryAsRef;
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::path::{self, Segment};
use guatiao::schema::ValidationError;
use guatiao::schema::read::{FieldRef, Kind, SchemaRef};
use guatiao::schema::validate::text_of;
use guatiao::schema::vocab as schema_vocab;
use guatiao::value::alloc::Alloc;
use guatiao::value::error::ValueError;
use guatiao::value::read::str_or;
use guatiao::value::types::{Map, Tag, Text, Value};

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
    let chosen = str_or(
        TryAsRef::<Map>::try_as_ref(value).and_then(|m| m.get(tag)),
        "",
    );
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        return false;
    };

    // FIRST, AND UNCONDITIONAL. See the module note.
    let prefix = format!("{}{SEPARATOR}", field.key());
    store.retain(|key, _| !key.starts_with(&prefix));

    store.insert(field.key().to_string(), chosen.to_string());
    for member in arm.fields() {
        if let Some(present) = TryAsRef::<Map>::try_as_ref(value).and_then(|m| m.get(member.key()))
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

    let mut map = Map::new_in(alloc);
    // The discriminant goes in FIRST, so re-emission is byte-stable: a map
    // is insertion-ordered by contract.
    map.set(tag, Text::new_in(alloc, chosen).map(Value::from).ok()?)
        .ok()?;
    let prefix = format!("{}{SEPARATOR}", field.key());
    for member in arm.fields() {
        if let Some(text) = store.get(&format!("{prefix}{}", member.key()))
            && let Ok(v) = Text::new_in(alloc, text).map(Value::from)
        {
            let _ = map.set(member.key(), v);
        }
    }
    Some(map.into())
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

/// The field governing a path, walking every segment of it.
///
/// `auth.password` reaches the arm field an arm of a variant adds;
/// `connection.tls.ca` walks two declared objects; `agent[1].name`
/// indexes a list and then names a member; `env[PATH]` reaches into an
/// open map. A segment that names nothing answers `None`, and so does a
/// segment applied to something that has no members at all.
///
/// **A key that names a field outright wins**, whatever it contains: a
/// schema may declare a field called `a.b`, and `resolve(schema, "a.b")`
/// means that field rather than a walk. To reach a *member* called `a.b`
/// of a declared object, quote it: `owner["a.b"]`.
///
/// What comes back for a list element or an open map's entry is a
/// [`FieldRef`] over the **element's** schema with an **empty key**: such
/// a thing has no declared name, and the path that reached it is the
/// caller's to keep.
pub fn resolve<'a>(schema: SchemaRef<'a>, key: &str) -> Option<FieldRef<'a>> {
    if let Some(direct) = schema.find(key) {
        return Some(direct);
    }
    let parsed = path::parse(key).ok()?;
    let mut segments = parsed.segments();
    let mut at = root(schema, &segments.next()?)?;
    for segment in segments {
        at = into(at, &segment)?;
    }
    Some(at)
}

/// The top-level field a path's first segment names.
///
/// A schema's root is an object, so a bracket there is a key like any
/// other: `["a.b"]` names the field spelled `a.b`, and `[0]` names one
/// spelled `0`.
fn root<'a>(schema: SchemaRef<'a>, first: &Segment<'_>) -> Option<FieldRef<'a>> {
    schema.find(&name_of(first))
}

/// A segment as the text it names, for a container that is keyed.
fn name_of<'a>(segment: &'a Segment<'_>) -> Cow<'a, str> {
    match segment {
        Segment::Field(name) => Cow::Borrowed(name),
        Segment::Key(key) => Cow::Borrowed(key.as_ref()),
        // A number applied to something keyed is that key's text, the
        // same rule `path::get` follows over a value.
        Segment::Index(at) => Cow::Owned(at.to_string()),
    }
}

/// One step INTO a field, by one segment.
fn into<'a>(field: FieldRef<'a>, segment: &Segment<'_>) -> Option<FieldRef<'a>> {
    let kind = field.kind();
    match kind {
        // A declared object, or the members an arm of a variant adds:
        // both are named fields, and the segment names one.
        Kind::Map(_) => kind.fields().find(|f| f.key() == name_of(segment)),
        Kind::Variant { .. } => kind
            .arms()
            .find_map(|arm| arm.fields().find(|f| f.key() == name_of(segment))),
        // A list is indexed, never keyed: `agent[0]` reaches an element
        // and `agent[first]` reaches nothing. Every element has the one
        // schema, so WHICH index it is changes nothing here.
        Kind::List(items) => {
            matches!(segment, Segment::Index(_)).then(|| FieldRef::new("", items))?
        }
        // An open map is the other way round: its keys are data, so any
        // segment reaches its one value schema.
        Kind::MapOf(_) => FieldRef::new("", values_of(field)?),
        _ => None,
    }
}

/// The value schema of an open map, as the value it is written as.
///
/// [`Kind::values`](guatiao::schema::read::Kind::values) answers a kind;
/// a [`FieldRef`] needs the value under it, which is the same place JSON
/// Schema puts it.
fn values_of<'a>(field: FieldRef<'a>) -> Option<&'a Value> {
    TryAsRef::<Map>::try_as_ref(field.as_value())
        .and_then(|m| m.get(schema_vocab::ADDITIONAL_PROPERTIES))
}

/// The field governing a path **in a particular store**: the arm a
/// payload key belongs to must be the one the store selects.
///
/// [`resolve`] answers for the schema alone, and for a schema alone
/// `auth.username` is a field of SOME arm. A store has said which arm --
/// `selected(path)` is the discriminant's text there, or `None` when
/// nothing was written and the field's default decides -- and a payload
/// key the chosen arm does not declare is an error rather than something
/// to drop. Ignoring it is how a credential ends up stored under a key
/// the chosen arm does not have.
///
/// The discriminant is asked for **by the owner's own path**, which is
/// the key it is stored under: `auth` at the root, `agent[0].auth` for a
/// variant inside a list of objects.
///
/// Every refusal names the **prefix that failed**, not the whole path: a
/// path is wrong at one step, and saying which one is the difference
/// between a message a person can act on and one they have to bisect.
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
    let Ok(parsed) = path::parse(key) else {
        return Err(unknown());
    };
    let mut segments = parsed.segments();
    let Some(first) = segments.next() else {
        return Err(unknown());
    };
    let Some(mut at) = root(schema, &first) else {
        return Err(unknown());
    };

    // The path walked so far, which is both what a refusal names and the
    // key a discriminant is stored under.
    let mut walked = first.to_string();
    for segment in segments {
        at = step_in(at, &segment, &walked, key, &selected)?;
        if !matches!(segment, Segment::Field(_)) {
            walked.push_str(&segment.to_string());
        } else {
            walked.push(SEPARATOR);
            walked.push_str(&segment.to_string());
        }
    }
    Ok(at)
}

/// One step of [`resolve_in`], with the arm a store selected.
fn step_in<'a>(
    field: FieldRef<'a>,
    segment: &Segment<'_>,
    walked: &str,
    whole: &str,
    selected: &impl Fn(&str) -> Option<String>,
) -> Result<FieldRef<'a>, ValidationError> {
    let kind = field.kind();
    let Kind::Variant { .. } = kind else {
        // Everything else steps the same way it does without a store:
        // which element or member a path reaches is not something a
        // store's contents can change.
        return into(field, segment).ok_or_else(|| {
            let known: Vec<String> = kind.fields().map(|f| f.key().to_string()).collect();
            if known.is_empty() {
                ValidationError::BadValue {
                    key: walked.to_string(),
                    expected: format!(
                        "something with members — '{whole}' reads through it as if it had them"
                    ),
                }
            } else {
                ValidationError::UnknownOption {
                    key: whole.to_string(),
                    known,
                }
            }
        });
    };

    let chosen = selected(walked)
        .filter(|t| !t.is_empty())
        .or_else(|| field.default().map(text_of))
        .unwrap_or_default();
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        // Either the tag names no arm -- which the tag's own entry
        // reports if it was given -- or nothing selected one and there is
        // no default. Naming the tag is the actionable half: the payload
        // cannot be checked until the arm is known.
        let names: Vec<&str> = kind.arms().map(|a| a.value()).collect();
        return Err(ValidationError::BadValue {
            key: walked.to_string(),
            expected: format!(
                "a value naming one of {} — '{whole}' belongs to an arm and cannot be \
                 checked until one is chosen",
                names.join(", ")
            ),
        });
    };
    let wanted = name_of(segment);
    arm.fields().find(|f| f.key() == wanted).ok_or_else(|| {
        // Names the ARM rather than listing keys, for a measured
        // reason: the empty arm is the interesting case, and its list
        // of accepted payload keys is EMPTY -- a sentence that stops
        // mid-air. The key is not wrong in general, it is wrong for
        // what was chosen.
        let accepted: Vec<&str> = arm.fields().map(|f| f.key()).collect();
        ValidationError::BadValue {
            key: whole.to_string(),
            expected: if accepted.is_empty() {
                format!("nothing — the '{chosen}' arm of '{walked}' stores no fields at all")
            } else {
                format!(
                    "nothing — the '{chosen}' arm of '{walked}' accepts only {}",
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
