// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whether a value is one the schema would accept.
//!
//! A consumer validates before crossing a boundary, so a bad value is
//! reported where it was typed rather than deep inside a provider. The
//! provider validates again on entry, because it cannot trust a caller.
//!
//! # Bounded, like every other walk over a tree
//!
//! Both the schema and the value can come from a foreign producer, and
//! either nested past [`MAX_DEPTH`] is refused rather than followed. An
//! unbounded recursion over one is a stack overflow, which on Windows is
//! not catchable and takes the host down with it -- the failure every
//! other walk in this crate is bounded to avoid.
//!
//! # Errors never echo the offending value
//!
//! [`ValidationError`] carries the field's key and a description of what
//! *would* have been accepted, and deliberately not what was given. An
//! field may be marked sensitive, and an error type that quotes its input
//! is an error type that eventually logs a passphrase. It costs nothing to
//! leave out: the caller still holds the value it just passed in, and can
//! decide for itself whether showing it is safe.

#![forbid(unsafe_code)]

use crate::value::convert::TryAsRef;

use super::ValidationError;
use super::read::{FieldRef, Kind, SchemaRef};
use crate::value::error::MAX_DEPTH;

/// Between an owner and one of its members, in the KEY an error
/// reports: `"tls.verify"`.
///
/// **Prose for a person, not a path language.** Naming a place inside
/// a value is `guatiao-intake`'s business, and this crate does not
/// depend on it -- an error message that reads the way a reader
/// expects is not a reason to. Do not reintroduce that dependency to
/// "share" this character.
pub(crate) const NESTED: char = '.';
use crate::value::read::str_or;
use crate::value::types::{List, Map, Number, Tag, Value};

/// The text spellings a boolean accepts.
///
/// The empty string is among them, and that is not an oversight: a flag
/// written bare on a command line or in a query string — `?verbose` — has
/// no text after the `=`, and it means true. See [`bool_is_true`].
pub const BOOL_WORDS: [&str; 7] = ["1", "0", "true", "false", "yes", "no", ""];

/// Whether a given boolean *value* reads as true.
///
/// Not the same question as what the field defaults to: a field with no
/// value at all is absent, and absence is the caller's to resolve.
pub fn bool_is_true(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    matches!(value, "1" | "true" | "yes")
}

/// The single-line spelling of a value, for the text form.
///
/// A container has none, which is why the callers below reject one by name
/// rather than validating the empty string it would otherwise produce.
pub fn text_of(value: &Value) -> String {
    match value.tag() {
        Ok(Tag::GUATIAO_BOOL) => crate::value::read::bool_or(Some(value), false).to_string(),
        Ok(Tag::GUATIAO_NUMBER) => TryAsRef::<Number>::try_as_ref(value)
            .map(AsRef::<str>::as_ref)
            .unwrap_or("")
            .to_string(),
        Ok(Tag::GUATIAO_STRING) => TryAsRef::<str>::try_as_ref(value).unwrap_or("").to_string(),
        Ok(Tag::GUATIAO_BYTES) => {
            String::from_utf8_lossy(TryAsRef::<[u8]>::try_as_ref(value).unwrap_or(&[])).into_owned()
        }
        _ => String::new(),
    }
}

fn bad(key: impl Into<String>, expected: impl Into<String>) -> ValidationError {
    ValidationError::BadValue {
        key: key.into(),
        expected: expected.into(),
    }
}

/// Whether `field` accepts `text`.
pub fn validate_text(field: FieldRef<'_>, text: &str) -> Result<(), ValidationError> {
    against(field.kind(), field.key(), text, 0)
}

/// What a refusal says when a tree is nested too deep to follow.
///
/// Names the key and the bound and, like every other refusal here, not
/// the value.
fn too_deep(key: &str) -> ValidationError {
    bad(
        key,
        format!("a value nested no deeper than {MAX_DEPTH} levels"),
    )
}

/// The arm values of a tagged kind, for a message.
fn arm_names(kind: Kind<'_>) -> String {
    let names: Vec<&str> = kind.arms().map(|a| a.value()).collect();
    names.join(", ")
}

/// Recursive, because a union holds kinds. A free function rather than a
/// method so the error can name the field's key, which a bare kind does
/// not know.
///
/// `depth` counts the levels already followed, shared with
/// [`value_against`] because the two recurse into each other.
fn against(kind: Kind<'_>, key: &str, value: &str, depth: u32) -> Result<(), ValidationError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep(key));
    }
    match kind {
        Kind::Str {
            min_length,
            max_length,
            pattern,
            ..
        } => {
            // Code points, as JSON Schema counts: `chars`, never `len`.
            let n = value.chars().count() as u64;
            if min_length.is_some_and(|m| n < m) || max_length.is_some_and(|m| n > m) {
                return Err(bad(key, lengths(min_length, max_length)));
            }
            if let Some(pattern) = pattern
                && !matches_pattern(pattern, value)
            {
                return Err(bad(key, format!("text matching {pattern}")));
            }
            Ok(())
        }
        Kind::Bool => {
            if BOOL_WORDS.contains(&value) {
                Ok(())
            } else {
                Err(bad(key, "a boolean (1/0, true/false, yes/no)"))
            }
        }
        Kind::Int {
            min,
            max,
            multiple_of,
        } => {
            if !is_integer_text(value) {
                return Err(bad(key, "a whole number"));
            }
            if !within(value, min, max) {
                return Err(bad(key, bounds(min, max)));
            }
            if let Some(step) = multiple_of
                && !int_is_multiple(value, step)
            {
                return Err(bad(key, format!("a multiple of {step}")));
            }
            Ok(())
        }
        Kind::Float {
            min,
            max,
            multiple_of,
        } => {
            let x: f64 = value.parse().map_err(|_| bad(key, "a number"))?;
            // NaN fails every comparison, so it needs rejecting on its
            // own: without this, `NaN < min` is false, `NaN > max` is
            // false, and a NaN sails through a bounded range.
            if x.is_nan() {
                return Err(bad(key, "a number"));
            }
            if min.is_some_and(|m| x < m) || max.is_some_and(|m| x > m) {
                return Err(bad(key, bounds(min, max)));
            }
            if let Some(step) = multiple_of
                && !float_is_multiple(x, step)
            {
                return Err(bad(key, format!("a multiple of {step}")));
            }
            Ok(())
        }
        Kind::Enum(_) => {
            if kind.choices().any(|c| c.value() == value) {
                Ok(())
            } else {
                let names: Vec<&str> = kind.choices().map(|c| c.value()).collect();
                Err(bad(key, format!("one of {}", names.join(", "))))
            }
        }
        // THE TEXT FORM OF A TAGGED FIELD IS ITS DISCRIMINANT, and
        // nothing else. Behaviourally identical to an enum over the arm
        // values, which is what lets a flat `?auth=userpass` keep
        // validating with no special case anywhere.
        //
        // A payload cannot be checked from here, because there is no
        // payload in a `&str`. [`validate_value`] is where that happens.
        Kind::Variant { .. } => {
            if kind.arms().any(|a| a.value() == value) {
                Ok(())
            } else {
                Err(bad(key, format!("one of {}", arm_names(kind))))
            }
        }
        // ANY arm accepting is enough, which is `anyOf` rather than strict
        // `oneOf`. Knowing which arm took the value is explicitly not the
        // point of a union; that is what a variant is for.
        Kind::Union(_) => {
            if kind
                .alternatives()
                .any(|k| against(k, key, value, depth + 1).is_ok())
            {
                Ok(())
            } else {
                Err(bad(key, "one of the accepted forms"))
            }
        }
        // THREE KINDS WITH NO SINGLE-LINE SPELLING of this crate's
        // choosing. Bytes need an encoding, a list needs a separator and
        // an object needs a whole syntax — every one of those is a format
        // decision this crate deliberately does not make, so there is no
        // rule here to check against, and accepting is the honest answer
        // rather than the lenient one. The structural check is in
        // [`validate_value`], which has the value rather than its text.
        Kind::Bytes | Kind::List { .. } | Kind::Map(_) | Kind::MapOf(_) => Ok(()),
        // A kind from a newer producer: this build cannot say whether the
        // value is acceptable, so it does not pretend to. Accepting is the
        // right answer rather than the lenient one — rejecting would make
        // every value of that field unusable on an older consumer, which
        // is worse than letting the provider have the last word, and the
        // provider validates again on entry.
        Kind::Unknown(_) => Ok(()),
        Kind::Missing => Err(bad(key, "a declared kind — this field declares none")),
    }
}

/// Whether `text` is a whole number written plainly: an optional sign and
/// then digits.
///
/// Deliberately not a parse into any width. **A number is the exact text
/// that declared it**, and an integer too large for an `i64` is still an
/// integer -- refusing it here would make a `u64` field unable to carry
/// its own maximum, which is the failure the derive's own agreement test
/// found.
fn is_integer_text(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Whether an integer written as `text` satisfies the declared bounds.
///
/// Compared as `i128`, which covers every Rust integer width. Beyond even
/// that, the value cannot violate a bound that was not declared and must
/// violate one that was, so its SIGN settles it — no bound this
/// vocabulary can write down is out there.
fn within(text: &str, min: Option<i64>, max: Option<i64>) -> bool {
    match text.parse::<i128>() {
        Ok(n) => min.is_none_or(|m| n >= i128::from(m)) && max.is_none_or(|m| n <= i128::from(m)),
        Err(_) if text.starts_with('-') => min.is_none(),
        Err(_) => max.is_none(),
    }
}

/// Whether an integer written as `text` is a multiple of `step`.
///
/// **Exact** for a whole-number step, in `i128` like [`within`]. An
/// integer past even that cannot be checked by this build and is not
/// refused for it. A fractional step on an integer is a real-number
/// question, and goes to [`float_is_multiple`].
fn int_is_multiple(text: &str, step: f64) -> bool {
    if step.fract() == 0.0 && step < 9.0e18 {
        // `step` is positive (the reader drops any other) and whole, so
        // this is the integer it spells.
        let step = step as i128;
        return text.parse::<i128>().map_or(true, |n| n % step == 0);
    }
    text.parse::<f64>()
        .map_or(true, |x| float_is_multiple(x, step))
}

/// Whether `x` is a multiple of `step`, to within a few ulps of `x`.
///
/// Not `x % step == 0.0`: in binary `0.3 % 0.1` is `0.09999999999999998`,
/// and a person who typed `0.3` into a field stepping by `0.1` meant a
/// multiple. The nearest multiple is compared against `x` with a
/// tolerance that grows with `x`, which is where float error does.
fn float_is_multiple(x: f64, step: f64) -> bool {
    let nearest = (x / step).round() * step;
    (nearest - x).abs() <= f64::EPSILON * x.abs().max(1.0) * 4.0
}

/// Whether `text` matches `pattern`, unanchored, as JSON Schema specifies.
///
/// A pattern the `regex` crate cannot compile -- ECMA-262 lookaround or a
/// backreference -- is carried, not enforced: refusing every value for a
/// schema this build cannot evaluate would make the field unusable, the
/// same argument [`Kind::Unknown`] makes. Compiled per call, because this
/// crate keeps no process-global cache; `regex` matches in linear time,
/// so a hostile pattern costs its compile and nothing worse.
#[cfg(feature = "regex")]
fn matches_pattern(pattern: &str, text: &str) -> bool {
    regex::Regex::new(pattern).map_or(true, |re| re.is_match(text))
}

/// Without the `regex` feature a pattern is carried and never enforced.
#[cfg(not(feature = "regex"))]
fn matches_pattern(_pattern: &str, _text: &str) -> bool {
    true
}

/// What a length refusal says it wanted.
fn lengths(min: Option<u64>, max: Option<u64>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("text of between {lo} and {hi} characters"),
        (Some(lo), None) => format!("text of at least {lo} characters"),
        (None, Some(hi)) => format!("text of at most {hi} characters"),
        (None, None) => "text".to_string(),
    }
}

fn bounds<T: std::fmt::Display>(min: Option<T>, max: Option<T>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("a number between {lo} and {hi}"),
        (Some(lo), None) => format!("a number no less than {lo}"),
        (None, Some(hi)) => format!("a number no greater than {hi}"),
        (None, None) => "a number".to_string(),
    }
}

/// Whether `field` accepts `value` as a **typed** value.
///
/// # Why this exists beside [`validate_text`]
///
/// The text form of a tagged field carries the discriminant only —
/// `auth = "userpass"` and nothing else — so the text check can say which
/// arm was named and nothing at all about the payload. That is coherent
/// (it is exactly what a dropdown selects and what a flat `?auth=userpass`
/// spells) and it is not sufficient: the whole argument for a tagged kind
/// is that a field belonging to an *unselected* arm should not merely be
/// ignored.
///
/// So this checks the three things the text form structurally cannot: the
/// discriminant names a declared arm, every key present is declared **by
/// that arm**, and every required field of that arm is present.
pub fn validate_value(field: FieldRef<'_>, value: &Value) -> Result<(), ValidationError> {
    value_against(field.kind(), field.key(), value, 0)
}

/// The body of [`validate_value`], written over a kind rather than an
/// field.
///
/// A list's elements have a kind and no key of their own, so the recursion
/// cannot be written over fields. The key is carried along only to build
/// the path an error reports.
///
/// `depth` bounds the walk: a schema and a value that nest each other
/// past [`MAX_DEPTH`] are refused rather than followed, because this
/// recursion runs over two trees a caller did not write.
fn value_against(
    kind: Kind<'_>,
    key: &str,
    value: &Value,
    depth: u32,
) -> Result<(), ValidationError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep(key));
    }
    match kind {
        Kind::Bytes => {
            return if value.tag() == Ok(Tag::GUATIAO_BYTES) {
                Ok(())
            } else {
                Err(bad(key, "bytes"))
            };
        }
        Kind::List { min, max, .. } => {
            let Some(items) = TryAsRef::<List>::try_as_ref(value).map(|list| &list[..]) else {
                return Err(bad(key, "a list"));
            };
            let n = items.len() as u64;
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(bad(
                    key,
                    match (min, max) {
                        (Some(lo), Some(hi)) => format!("a list of between {lo} and {hi} items"),
                        (Some(lo), None) => format!("a list of at least {lo} items"),
                        (None, Some(hi)) => format!("a list of at most {hi} items"),
                        (None, None) => "a list".to_string(),
                    },
                ));
            }
            let element = kind.items();
            for (i, item) in items.iter().enumerate() {
                value_against(element, &format!("{key}[{i}]"), item, depth + 1)?;
            }
            return Ok(());
        }
        Kind::MapOf(_) => {
            // The keys are data, so there is nothing to check about them
            // and no such thing as an undeclared one. What is declared is
            // the value, and every entry answers to it.
            if value.tag() != Ok(Tag::GUATIAO_MAP) {
                return Err(bad(key, "an object"));
            }
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return Err(bad(key, "keys that are text"));
            };
            let element = kind.values();
            for entry in map {
                // `[key]` rather than `.key`, the same bracket the list
                // branch writes for a position: an entry of an open map
                // is reached by its key, and a key is data.
                value_against(
                    element,
                    &format!("{key}[{}]", entry.key()),
                    entry.value(),
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        Kind::Map(_) => {
            if value.tag() != Ok(Tag::GUATIAO_MAP) {
                return Err(bad(key, "an object"));
            }
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return Err(bad(key, "keys that are text"));
            };
            // The same two rules a variant arm's payload obeys, for the
            // same reason: a key nobody declared is a mistake worth
            // reporting rather than something to drop, and a required
            // field that is absent is the other half of the same check.
            for entry in map {
                let name = entry.key();
                let Some(field) = kind.fields().find(|f| f.key() == name) else {
                    return Err(bad(
                        format!("{key}{}{name}", NESTED),
                        format!("nothing — '{name}' is not a declared field"),
                    ));
                };
                value_against(
                    field.kind(),
                    &format!("{key}{}{name}", NESTED),
                    entry.value(),
                    depth + 1,
                )?;
            }
            for field in kind.fields() {
                if field.is_required()
                    && !TryAsRef::<Map>::try_as_ref(value)
                        .is_some_and(|m| m.contains_key(field.key()))
                {
                    return Err(bad(
                        format!("{key}{}{}", NESTED, field.key()),
                        "a value — it is required",
                    ));
                }
            }
            return Ok(());
        }
        _ => {}
    }

    let Kind::Variant { tag, .. } = kind else {
        // A scalar's own check is its text form, and a container reaching
        // here has none: it is rejected by name rather than silently
        // validated as the empty string.
        return match value.tag() {
            Ok(Tag::GUATIAO_NULL | Tag::GUATIAO_LIST | Tag::GUATIAO_MAP) => {
                Err(bad(key, "a single value, not a container"))
            }
            _ => against(kind, key, &text_of(value), depth),
        };
    };

    if value.tag() != Ok(Tag::GUATIAO_MAP) {
        return Err(bad(
            key,
            format!("an object carrying a '{tag}' discriminant"),
        ));
    }
    let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
        return Err(bad(key, "keys that are text"));
    };
    let chosen = str_or(map.get(tag), "");
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        return Err(bad(
            key,
            format!("a '{tag}' naming one of {}", arm_names(kind)),
        ));
    };

    // A field belonging to an arm that was not selected is an error, not
    // something to drop silently — the same argument the schema makes for
    // a field key it does not declare.
    for entry in map {
        let name = entry.key();
        if name == tag {
            continue;
        }
        let Some(field) = arm.fields().find(|f| f.key() == name) else {
            return Err(bad(
                format!("{key}{}{name}", NESTED),
                format!("nothing — '{name}' is not declared by the '{chosen}' arm"),
            ));
        };
        value_against(
            field.kind(),
            &format!("{key}{}{name}", NESTED),
            entry.value(),
            depth + 1,
        )?;
    }

    for field in arm.fields() {
        if field.is_required()
            && !TryAsRef::<Map>::try_as_ref(value).is_some_and(|m| m.contains_key(field.key()))
        {
            return Err(bad(
                format!("{key}{}{}", NESTED, field.key()),
                format!("a value — it is required by the '{chosen}' arm"),
            ));
        }
    }
    Ok(())
}

/// Whether every entry of a map of **typed** values is accepted.
pub fn validate_map(schema: SchemaRef<'_>, values: &Value) -> Result<(), ValidationError> {
    if values.tag() != Ok(Tag::GUATIAO_MAP) {
        return Err(bad("", "a map of values"));
    }
    let Some(map) = TryAsRef::<Map>::try_as_ref(values) else {
        return Err(bad("", "keys that are text"));
    };
    for entry in map {
        let key = entry.key();
        let Some(field) = schema.find(key) else {
            return Err(ValidationError::UnknownOption {
                key: key.to_string(),
                known: schema.fields().map(|o| o.key().to_string()).collect(),
            });
        };
        validate_value(field, entry.value())?;
    }
    for field in schema.fields() {
        if field.is_required()
            && !TryAsRef::<Map>::try_as_ref(values).is_some_and(|m| m.contains_key(field.key()))
        {
            return Err(bad(field.key(), "a value — it is required"));
        }
    }
    Ok(())
}
