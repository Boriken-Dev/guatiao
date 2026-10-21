// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The two durations: `std::time::Duration`, an exact length, as
//! `[seconds, nanoseconds]`; [`Span`], a calendar length, as its ISO 8601
//! text.

#![forbid(unsafe_code)]

use alloc::string::ToString;
use core::time::Duration;

use super::{FromValue, MapError, ToValue, TryAsRef};
use crate::value::alloc::Alloc;
use crate::value::error::ValueError;
use crate::value::span::Span;
use crate::value::types::{List, Map, Number, Tag, Text, Value};

/// What a refused duration was expected to be.
const DURATION: &str = "a duration: [seconds, nanoseconds], a non-negative number of seconds no \
                        finer than a nanosecond, or {\"secs\", \"nanos\"}";

/// `[secs, nanos]`: the whole seconds as a `u64`, then the nanoseconds
/// below a second, in the order of C's `struct timespec`.
impl ToValue for Duration {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        let mut list = List::new_in(alloc);
        list.push_in(self.as_secs().to_value_in(alloc)?, alloc)?;
        list.push_in(self.subsec_nanos().to_value_in(alloc)?, alloc)?;
        Ok(list.into())
    }
}

/// Three spellings, one length:
///
/// - `[secs, nanos]`, what is written: a `u64` and a `u32` below
///   1 000 000 000;
/// - a number of seconds, read exactly from its text (`1.5`, `2e-3`):
///   not negative and no finer than a nanosecond;
/// - `{"secs": .., "nanos": ..}`, the map serde writes, both keys present.
///
/// Anything else is refused naming all three.
impl FromValue for Duration {
    fn from_value(value: &Value) -> Result<Duration, MapError> {
        let refused = || MapError::bad_value(DURATION);
        let parts = match value.tag() {
            Ok(Tag::GUATIAO_LIST) => {
                let items = &<&List>::try_from(value)?[..];
                let [secs, nanos] = items else {
                    return Err(refused());
                };
                (
                    u64::from_value(secs).map_err(|e| e.at(0))?,
                    u32::from_value(nanos).map_err(|e| e.at(1))?,
                )
            }
            Ok(Tag::GUATIAO_MAP) => {
                let map = <&Map>::try_from(value)?;
                let read = |key: &str| map.required(key);
                (
                    u64::from_value(read("secs")?).map_err(|e| e.under("secs"))?,
                    u32::from_value(read("nanos")?).map_err(|e| e.under("nanos"))?,
                )
            }
            Ok(Tag::GUATIAO_NUMBER) => {
                let number = TryAsRef::<Number>::try_as_ref(value).ok_or_else(refused)?;
                seconds_of(number.as_ref()).ok_or_else(refused)?
            }
            _ => return Err(refused()),
        };
        if parts.1 >= 1_000_000_000 {
            return Err(MapError::bad_value("nanoseconds below 1000000000").at(1));
        }
        Ok(Duration::new(parts.0, parts.1))
    }
}

/// A number in the JSON grammar as whole seconds and nanoseconds, exactly;
/// `None` when it is negative, finer than a nanosecond, or past a `u64` of
/// seconds.
fn seconds_of(text: &str) -> Option<(u64, u32)> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], rest[at + 1..].parse::<i64>().ok()?),
        None => (rest, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits: alloc::string::String = whole.chars().chain(fraction.chars()).collect();
    let digits = digits.trim_start_matches('0');
    let trimmed = digits.trim_end_matches('0');
    if trimmed.is_empty() {
        return Some((0, 0));
    }
    if negative {
        return None;
    }
    // The value is `trimmed` times ten to `shift`, counted in nanoseconds.
    let shift = exponent
        .checked_sub(fraction.len() as i64)?
        .checked_add(9)?
        .checked_add((digits.len() - trimmed.len()) as i64)?;
    if shift < 0 {
        return None;
    }
    let mut nanos: u128 = trimmed.parse().ok()?;
    for _ in 0..shift {
        nanos = nanos.checked_mul(10)?;
    }
    let secs = u64::try_from(nanos / 1_000_000_000).ok()?;
    Some((secs, (nanos % 1_000_000_000) as u32))
}

/// Its ISO 8601 text, canonical: `P1DT2H`.
impl ToValue for Span {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        Ok(Text::new_in(alloc, &self.to_string())?.into())
    }
}

/// Text in either form `Span::from_str` reads; a refusal names the byte
/// it stopped at and what it expected.
impl FromValue for Span {
    fn from_value(value: &Value) -> Result<Span, MapError> {
        <&str>::try_from(value)?.parse().map_err(|e| match e {
            ValueError::Unparsed { at, expected } => MapError::bad_value(alloc::format!(
                "an ISO 8601 duration or a span such as 1h 30m: {expected} at byte {at}"
            )),
            _ => MapError::bad_value("an ISO 8601 duration or a span such as 1h 30m"),
        })
    }
}
