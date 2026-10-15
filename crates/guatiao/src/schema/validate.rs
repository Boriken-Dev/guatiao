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

use core::cmp::Ordering;

use crate::value::convert::TryAsRef;

use super::ValidationError;
use super::read::{FieldRef, Kind, SchemaRef, is_nullable, members, peel};
use super::vocab;
use crate::value::error::MAX_DEPTH;
use crate::value::read::bool_or;
use crate::value::types::number::validate_json_number;

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
///
/// **The check for a store of text** (a flat `key -> text` map, a query
/// string, a command line), where a value has no type but its spelling:
/// a boolean is one of [`BOOL_WORDS`], a number is written as JSON
/// writes one (no `inf`, no `+1`, no leading zero), and a container has no
/// text form, so it is accepted here and checked by [`validate_value`].
/// A typed value is checked by [`validate_value`], which also requires
/// the type the kind names.
pub fn validate_text(field: FieldRef<'_>, text: &str) -> Result<(), ValidationError> {
    against(field.as_value(), field.key(), text, 0)
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

/// Recursive, because a union holds schemas. Written over the schema
/// rather than its [`Kind`], because a numeric bound is compared as the
/// exact text that declared it and the kind carries only what fits an
/// `i64` or an `f64`.
///
/// `depth` counts the levels already followed, shared with
/// [`value_against`] because the two recurse into each other.
fn against(schema: &Value, key: &str, value: &str, depth: u32) -> Result<(), ValidationError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep(key));
    }
    let kind = Kind::read(Some(schema));
    let own = peel(schema);
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
                && !super::pattern::matches(pattern, value)
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
        Kind::Int { multiple_of, .. } => {
            if !is_integer_text(value) {
                return Err(bad(key, "a whole number"));
            }
            let bounds = Bounds::read(own);
            if !bounds.admit(value) {
                return Err(bad(key, bounds.describe()));
            }
            // `-0` is zero, and an unsigned Rust type still refuses the
            // spelling: below a minimum of zero, a minus sign is refused.
            if value.starts_with('-')
                && bounds
                    .min
                    .is_some_and(|m| compare(m, "0").is_some_and(Ordering::is_ge))
            {
                return Err(bad(
                    key,
                    format!("{}, written without a minus sign", bounds.describe()),
                ));
            }
            if let Some(step) = multiple_of
                && !int_is_multiple(value, step)
            {
                return Err(bad(key, format!("a multiple of {step}")));
            }
            Ok(())
        }
        Kind::Float { multiple_of, .. } => {
            // The JSON number grammar and a finite result: `inf`, `NaN`
            // and `1e999` are not numbers, and NaN would otherwise sail
            // through a bounded range, failing every comparison.
            let Some(x) = number_of(value) else {
                return Err(bad(key, "a number"));
            };
            let bounds = Bounds::read(own);
            if !bounds.admit(value) {
                return Err(bad(key, bounds.describe()));
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
                Err(bad(key, one_of_members(kind)))
            }
        }
        Kind::Null => Err(bad(key, "null")),
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
        Kind::Union(any_of) => {
            if TryAsRef::<List>::try_as_ref(any_of)
                .map(|list| &list[..])
                .unwrap_or(&[])
                .iter()
                .any(|arm| against(arm, key, value, depth + 1).is_ok())
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

/// Whether `text` is a whole number written plainly: a JSON number with an
/// optional minus, digits, no leading zero, and no fraction or exponent.
///
/// Deliberately not a parse into any width. **A number is the exact text
/// that declared it**, and an integer too large for an `i64` is still an
/// integer -- refusing it here would make a `u64` field unable to carry
/// its own maximum, which is the failure the derive's own agreement test
/// found.
fn is_integer_text(text: &str) -> bool {
    validate_json_number(text.as_bytes()).is_ok()
        && !text.bytes().any(|b| matches!(b, b'.' | b'e' | b'E'))
}

/// The value of a number written as JSON writes one, or `None` for text
/// outside that grammar and for one too large for an `f64`.
///
/// Not `str::parse`, which takes `inf`, `NaN`, `.5` and `+1`.
fn number_of(text: &str) -> Option<f64> {
    validate_json_number(text.as_bytes()).ok()?;
    text.parse::<f64>().ok().filter(|x| x.is_finite())
}

/// The four bounds JSON Schema can put on a number, each as the exact
/// text that declared it. A bound that is not a number is no bound.
struct Bounds<'a> {
    min: Option<&'a str>,
    max: Option<&'a str>,
    exclusive_min: Option<&'a str>,
    exclusive_max: Option<&'a str>,
}

impl<'a> Bounds<'a> {
    fn read(schema: &'a Value) -> Bounds<'a> {
        let at = |key| {
            TryAsRef::<Map>::try_as_ref(schema)
                .and_then(|m| m.get(key))
                .and_then(TryAsRef::<Number>::try_as_ref)
                .map(AsRef::<str>::as_ref)
        };
        Bounds {
            min: at(vocab::MINIMUM),
            max: at(vocab::MAXIMUM),
            exclusive_min: at(vocab::EXCLUSIVE_MINIMUM),
            exclusive_max: at(vocab::EXCLUSIVE_MAXIMUM),
        }
    }

    /// Whether the number written as `text` lies within every bound,
    /// compared exactly: no width, no rounding, so `u64::MAX + 1` is past
    /// a maximum of `u64::MAX` and `1.5` is past a maximum of `1`.
    fn admit(&self, text: &str) -> bool {
        let is = |bound: Option<&str>, ok: fn(Ordering) -> bool| {
            bound.is_none_or(|b| compare(text, b).is_none_or(ok))
        };
        is(self.min, Ordering::is_ge)
            && is(self.max, Ordering::is_le)
            && is(self.exclusive_min, Ordering::is_gt)
            && is(self.exclusive_max, Ordering::is_lt)
    }

    /// What a refusal says it wanted.
    fn describe(&self) -> String {
        if let (Some(lo), Some(hi), None, None) =
            (self.min, self.max, self.exclusive_min, self.exclusive_max)
        {
            return format!("a number between {lo} and {hi}");
        }
        let parts: Vec<String> = [
            self.min.map(|b| format!("no less than {b}")),
            self.exclusive_min.map(|b| format!("greater than {b}")),
            self.max.map(|b| format!("no greater than {b}")),
            self.exclusive_max.map(|b| format!("less than {b}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        if parts.is_empty() {
            "a number".to_string()
        } else {
            format!("a number {}", parts.join(" and "))
        }
    }
}

/// A number as JSON writes one, read exactly: `0.digits` times ten to
/// `exponent`, with no leading or trailing zero digit. Zero has no digits.
struct Decimal {
    negative: bool,
    digits: Vec<u8>,
    exponent: i64,
}

/// An exponent past this is clamped: no two numbers a schema compares
/// differ there, and the clamp keeps the arithmetic below in range.
const EXPONENT_CLAMP: i64 = 1 << 40;

fn decimal(text: &str) -> Option<Decimal> {
    validate_json_number(text.as_bytes()).ok()?;
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, written) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (rest, ""),
    };
    let (exponent_negative, exponent_digits) = match written.as_bytes().first() {
        Some(b'-') => (true, &written[1..]),
        Some(b'+') => (false, &written[1..]),
        _ => (false, written),
    };
    let mut exponent: i64 = 0;
    for b in exponent_digits.bytes() {
        exponent = (exponent * 10 + i64::from(b - b'0')).min(EXPONENT_CLAMP);
    }
    if exponent_negative {
        exponent = -exponent;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut digits: Vec<u8> = whole.bytes().chain(fraction.bytes()).collect();
    let leading = digits.iter().take_while(|d| **d == b'0').count();
    digits.drain(..leading);
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    let exponent = exponent + whole.len() as i64 - leading as i64;
    Some(Decimal {
        negative: negative && !digits.is_empty(),
        digits,
        exponent,
    })
}

/// The order of two numbers written as JSON writes them, exactly, or
/// `None` when either is not one. `-0` and `0` are equal.
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (decimal(a)?, decimal(b)?);
    let sign = |d: &Decimal| match (d.digits.is_empty(), d.negative) {
        (true, _) => 0,
        (false, true) => -1,
        (false, false) => 1,
    };
    let by_sign = sign(&a).cmp(&sign(&b));
    if by_sign != Ordering::Equal || sign(&a) == 0 {
        return Some(by_sign);
    }
    let magnitude = a
        .exponent
        .cmp(&b.exponent)
        .then_with(|| a.digits.cmp(&b.digits));
    Some(if a.negative {
        magnitude.reverse()
    } else {
        magnitude
    })
}

/// Whether a number written as `text` has no fractional part, which is
/// what JSON Schema's `integer` type asks: `1.0` is one.
fn is_whole(text: &str) -> bool {
    decimal(text).is_some_and(|d| d.exponent >= d.digits.len() as i64)
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

/// What a length refusal says it wanted.
fn lengths(min: Option<u64>, max: Option<u64>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("text of between {lo} and {hi} characters"),
        (Some(lo), None) => format!("text of at least {lo} characters"),
        (None, Some(hi)) => format!("text of at most {hi} characters"),
        (None, None) => "text".to_string(),
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
    value_against(field.as_value(), field.key(), value, 0)
}

/// The body of [`validate_value`], written over a schema rather than a
/// field.
///
/// A list's elements have a schema and no key of their own, so the
/// recursion cannot be written over fields. The key is carried along only
/// to build the path an error reports.
///
/// `depth` bounds the walk: a schema and a value that nest each other
/// past [`MAX_DEPTH`] are refused rather than followed, because this
/// recursion runs over two trees a caller did not write.
fn value_against(
    schema: &Value,
    key: &str,
    value: &Value,
    depth: u32,
) -> Result<(), ValidationError> {
    if depth >= MAX_DEPTH {
        return Err(too_deep(key));
    }
    let kind = Kind::read(Some(schema));
    let tag = value.tag();
    if tag == Ok(Tag::GUATIAO_NULL) && is_nullable(schema) {
        return Ok(());
    }
    match kind {
        Kind::Null => {
            return if tag == Ok(Tag::GUATIAO_NULL) {
                Ok(())
            } else {
                Err(bad(key, "null"))
            };
        }
        Kind::Bytes => {
            return if tag == Ok(Tag::GUATIAO_BYTES) {
                Ok(())
            } else {
                Err(bad(key, "bytes"))
            };
        }
        // A scalar is the type its kind names, and only then are its
        // text rules applied: `5900` is not a string and `"yes"` is not a
        // boolean, whatever their spelling would pass.
        Kind::Bool => {
            return if tag == Ok(Tag::GUATIAO_BOOL) {
                Ok(())
            } else {
                Err(bad(key, "a boolean (true or false)"))
            };
        }
        Kind::Int { .. } | Kind::Float { .. } => {
            let Some(number) = TryAsRef::<Number>::try_as_ref(value) else {
                return Err(bad(
                    key,
                    if matches!(kind, Kind::Int { .. }) {
                        "a whole number"
                    } else {
                        "a number"
                    },
                ));
            };
            return against(schema, key, AsRef::<str>::as_ref(number), depth);
        }
        Kind::Str { .. } => {
            let Some(text) = TryAsRef::<str>::try_as_ref(value) else {
                return Err(bad(key, "text"));
            };
            return against(schema, key, text, depth);
        }
        // Both keywords apply: a member of the `enum` is refused when it
        // is not of the `type` beside it.
        Kind::Enum(owner) => {
            return if type_admits(owner, value)
                && members(owner).any(|(member, _)| same_json(member, value))
            {
                Ok(())
            } else {
                Err(bad(key, one_of_members(kind)))
            };
        }
        Kind::Union(any_of) => {
            let arms = TryAsRef::<List>::try_as_ref(any_of)
                .map(|list| &list[..])
                .unwrap_or(&[]);
            return if arms
                .iter()
                .any(|arm| value_against(arm, key, value, depth + 1).is_ok())
            {
                Ok(())
            } else {
                Err(bad(key, "one of the accepted forms"))
            };
        }
        Kind::List { items, min, max } => {
            let Some(elements) = TryAsRef::<List>::try_as_ref(value).map(|list| &list[..]) else {
                return Err(bad(key, "a list"));
            };
            let n = elements.len() as u64;
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
            for (i, element) in elements.iter().enumerate() {
                value_against(items, &format!("{key}[{i}]"), element, depth + 1)?;
            }
            return Ok(());
        }
        Kind::MapOf(owner) => {
            // The keys are data, so there is nothing to check about them
            // and no such thing as an undeclared one. What is declared is
            // the value, and every entry answers to it.
            if tag != Ok(Tag::GUATIAO_MAP) {
                return Err(bad(key, "an object"));
            }
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return Err(bad(key, "keys that are text"));
            };
            let Some(element) = TryAsRef::<Map>::try_as_ref(owner)
                .and_then(|m| m.get(vocab::ADDITIONAL_PROPERTIES))
            else {
                return Ok(());
            };
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
        Kind::Map(owner) => {
            if tag != Ok(Tag::GUATIAO_MAP) {
                return Err(bad(key, "an object"));
            }
            let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
                return Err(bad(key, "keys that are text"));
            };
            // The same two rules a variant arm's payload obeys: a key the
            // object does not declare answers to `additionalProperties`,
            // and a required field that is absent is refused.
            for entry in map {
                let name = entry.key();
                let path = nested(key, name);
                match kind.fields().find(|f| f.key() == name) {
                    Some(field) => {
                        value_against(field.as_value(), &path, entry.value(), depth + 1)?
                    }
                    None => undeclared(owner, &path, entry.value(), depth, || {
                        format!("nothing — '{name}' is not a declared field")
                    })?,
                }
            }
            for field in kind.fields() {
                if field.is_required() && !map.contains_key(field.key()) {
                    return Err(bad(nested(key, field.key()), "a value — it is required"));
                }
            }
            return Ok(());
        }
        // A kind from a newer producer, or a shape this build does not
        // read: it cannot say whether the value is acceptable, so it does
        // not pretend to, and the provider validates again on entry.
        Kind::Unknown(_) => return Ok(()),
        Kind::Missing => {
            return Err(bad(key, "a declared kind — this field declares none"));
        }
        _ => {}
    }

    let Kind::Variant {
        tag: discriminant, ..
    } = kind
    else {
        // A kind added after this was written: not one this build can
        // judge, which is what `Unknown` means.
        return Ok(());
    };

    if tag != Ok(Tag::GUATIAO_MAP) {
        return Err(bad(
            key,
            format!("an object carrying a '{discriminant}' discriminant"),
        ));
    }
    let Some(map) = TryAsRef::<Map>::try_as_ref(value) else {
        return Err(bad(key, "keys that are text"));
    };
    let chosen = str_or(map.get(discriminant), "");
    let Some(arm) = kind.arms().find(|a| a.value() == chosen) else {
        return Err(bad(
            key,
            format!("a '{discriminant}' naming one of {}", arm_names(kind)),
        ));
    };

    // A field belonging to an arm that was not selected answers to the
    // arm's `additionalProperties`, which every arm the builders seal sets
    // to `false`: an error, not something to drop silently.
    for entry in map {
        let name = entry.key();
        if name == discriminant {
            continue;
        }
        let path = nested(key, name);
        match arm.fields().find(|f| f.key() == name) {
            Some(field) => value_against(field.as_value(), &path, entry.value(), depth + 1)?,
            None => undeclared(arm.schema(), &path, entry.value(), depth, || {
                format!("nothing — '{name}' is not declared by the '{chosen}' arm")
            })?,
        }
    }

    for field in arm.fields() {
        if field.is_required() && !map.contains_key(field.key()) {
            return Err(bad(
                nested(key, field.key()),
                format!("a value — it is required by the '{chosen}' arm"),
            ));
        }
    }
    Ok(())
}

/// `key.name`, or `name` alone at the root, for the key an error reports.
fn nested(key: &str, name: &str) -> String {
    if key.is_empty() {
        name.to_string()
    } else {
        format!("{key}{NESTED}{name}")
    }
}

/// What an object's `additionalProperties` says about a key it does not
/// declare.
enum Undeclared<'a> {
    /// `false`: refused. What every object the builders seal says.
    Refused,
    /// Absent or `true`: accepted unchecked, JSON Schema's default.
    Accepted,
    /// A schema: the value answers to it.
    Checked(&'a Value),
}

fn undeclared_rule(owner: &Value) -> Undeclared<'_> {
    match TryAsRef::<Map>::try_as_ref(owner).and_then(|m| m.get(vocab::ADDITIONAL_PROPERTIES)) {
        Some(v) if v.tag() == Ok(Tag::GUATIAO_BOOL) && !bool_or(Some(v), true) => {
            Undeclared::Refused
        }
        Some(v) if TryAsRef::<Map>::try_as_ref(v).is_some() => Undeclared::Checked(v),
        _ => Undeclared::Accepted,
    }
}

/// A value under a key `owner` does not declare, judged by its
/// `additionalProperties`.
fn undeclared(
    owner: &Value,
    path: &str,
    value: &Value,
    depth: u32,
    refusal: impl FnOnce() -> String,
) -> Result<(), ValidationError> {
    match undeclared_rule(owner) {
        Undeclared::Refused => Err(bad(path, refusal())),
        Undeclared::Accepted => Ok(()),
        Undeclared::Checked(schema) => value_against(schema, path, value, depth + 1),
    }
}

/// Whether `value` is of a type the schema's `type` names; a schema with
/// no `type`, or one naming a type this build does not know, admits it.
fn type_admits(schema: &Value, value: &Value) -> bool {
    let Some(ty) = TryAsRef::<Map>::try_as_ref(schema).and_then(|m| m.get(vocab::TYPE)) else {
        return true;
    };
    if let Some(name) = TryAsRef::<str>::try_as_ref(ty) {
        return admits(name, value);
    }
    let names: Vec<&str> = TryAsRef::<List>::try_as_ref(ty)
        .map(|list| {
            list.iter()
                .filter_map(TryAsRef::<str>::try_as_ref)
                .collect()
        })
        .unwrap_or_default();
    names.is_empty() || names.iter().any(|name| admits(name, value))
}

fn admits(name: &str, value: &Value) -> bool {
    let tag = value.tag();
    match name {
        vocab::TYPE_NULL => tag == Ok(Tag::GUATIAO_NULL),
        vocab::TYPE_BOOLEAN => tag == Ok(Tag::GUATIAO_BOOL),
        vocab::TYPE_STRING => tag == Ok(Tag::GUATIAO_STRING),
        vocab::TYPE_BYTES => tag == Ok(Tag::GUATIAO_BYTES),
        vocab::TYPE_ARRAY => tag == Ok(Tag::GUATIAO_LIST),
        vocab::TYPE_OBJECT => tag == Ok(Tag::GUATIAO_MAP),
        vocab::TYPE_NUMBER => tag == Ok(Tag::GUATIAO_NUMBER),
        vocab::TYPE_INTEGER => {
            TryAsRef::<Number>::try_as_ref(value).is_some_and(|n| is_whole(n.as_ref()))
        }
        _ => true,
    }
}

/// Whether two values are the same JSON value: numbers by what they
/// denote (`1` and `1.0`), everything else structurally.
fn same_json(a: &Value, b: &Value) -> bool {
    match (
        TryAsRef::<Number>::try_as_ref(a).map(AsRef::<str>::as_ref),
        TryAsRef::<Number>::try_as_ref(b).map(AsRef::<str>::as_ref),
    ) {
        (Some(x), Some(y)) => {
            // Past 15 significant digits `f64` conflates neighbours, so
            // only the exact text speaks for a long number.
            x == y
                || (x.len() <= 15
                    && y.len() <= 15
                    && matches!((x.parse::<f64>(), y.parse::<f64>()), (Ok(p), Ok(q)) if p == q))
        }
        _ => a == b,
    }
}

/// `"one of a, b"` for an enumeration's refusal.
fn one_of_members(kind: Kind<'_>) -> String {
    let names: Vec<&str> = kind.choices().map(|c| c.value()).collect();
    if names.is_empty() {
        "one of the listed values".to_string()
    } else {
        format!("one of {}", names.join(", "))
    }
}

/// Whether `schema` accepts `value`, for a caller that has the schema as a
/// bare value (an arm of a union) and no field to name.
pub(crate) fn accepts(schema: &Value, value: &Value) -> bool {
    value_against(schema, "", value, 0).is_ok()
}

/// Whether every entry of a map of **typed** values is accepted.
///
/// Each value is the type its field's kind names (see [`validate_value`]),
/// every required field is present, and a key the schema does not declare
/// answers to its `additionalProperties`: refused for `false`, which every
/// schema the builders and the derive write says, checked against a
/// schema, and accepted when the document says nothing, as JSON Schema
/// reads it. A root that is not a declared object -- a tagged enum's
/// document -- is checked as the value it describes.
///
/// **A `pattern` is checked only if this build enforces them**: without
/// the `regex` feature it is carried and read but never evaluated, and a
/// value it would refuse is accepted. [`PATTERNS_ENFORCED`](super::PATTERNS_ENFORCED)
/// says which build this is.
pub fn validate_map(schema: SchemaRef<'_>, values: &Value) -> Result<(), ValidationError> {
    let root = schema.as_value();
    // A root that is not a declared object -- a tagged enum's document, a
    // map whose keys are data -- is checked as the value it describes.
    if !matches!(
        Kind::read(Some(root)),
        Kind::Map(_) | Kind::Unknown(_) | Kind::Missing
    ) {
        return value_against(root, "", values, 0);
    }
    if values.tag() != Ok(Tag::GUATIAO_MAP) {
        return Err(bad("", "a map of values"));
    }
    let Some(map) = TryAsRef::<Map>::try_as_ref(values) else {
        return Err(bad("", "keys that are text"));
    };
    for entry in map {
        let key = entry.key();
        let Some(field) = schema.find(key) else {
            match undeclared_rule(root) {
                Undeclared::Refused => {
                    return Err(ValidationError::UnknownOption {
                        key: key.to_string(),
                        known: schema.fields().map(|o| o.key().to_string()).collect(),
                    });
                }
                Undeclared::Accepted => continue,
                Undeclared::Checked(element) => {
                    value_against(element, key, entry.value(), 1)?;
                    continue;
                }
            }
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
