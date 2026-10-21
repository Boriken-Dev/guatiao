// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Spans and exact lengths of time, from C: a span read into a struct
//! and written back, and an exact length read in any of its spellings.

#![allow(non_camel_case_types)]

use core::ptr;
use core::time::Duration;

use super::entry;
use super::time::{TextError, format_from, normalize, parse_into};
use crate::value::alloc::Allocator;
use crate::value::convert::FromValue;
use crate::value::error::ValueError;
use crate::value::span::Span;
use crate::value::status::Status;
use crate::value::types::{Str, Value};

/// A span: a length of time in calendar units, each counted on its own,
/// with ONE sign for the whole span. Every non-zero field carries that
/// sign: `-P1DT2H` is `{.days = -1, .hours = -2}`, and a struct mixing
/// signs is refused with `GUATIAO_ERR_BAD_VALUE`. Each field is at most
/// INT64_MAX in magnitude (INT64_MIN is refused), and `nanoseconds` is
/// below a second, -999999999 to 999999999. Zeroed, it is `PT0S`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanFields {
    /// Years.
    pub years: i64,
    /// Months.
    pub months: i64,
    /// Weeks.
    pub weeks: i64,
    /// Days.
    pub days: i64,
    /// Hours.
    pub hours: i64,
    /// Minutes.
    pub minutes: i64,
    /// Whole seconds.
    pub seconds: i64,
    /// Below a second: -999999999 to 999999999.
    pub nanoseconds: i32,
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<SpanFields>() == 64 && offset_of!(SpanFields, nanoseconds) == 56);
    assert!(offset_of!(SpanFields, seconds) == 48);
};

impl From<Span> for SpanFields {
    fn from(s: Span) -> SpanFields {
        SpanFields {
            years: s.get_years(),
            months: s.get_months(),
            weeks: s.get_weeks(),
            days: s.get_days(),
            hours: s.get_hours(),
            minutes: s.get_minutes(),
            seconds: s.get_seconds(),
            // Below a second, by `Span`'s own invariant.
            nanoseconds: s.get_nanoseconds() as i32,
        }
    }
}

/// [`ValueError::Unrepresentable`] for fields of both signs, `i64::MIN`
/// anywhere, or nanoseconds of a second or more.
impl TryFrom<SpanFields> for Span {
    type Error = ValueError;

    fn try_from(f: SpanFields) -> Result<Span, ValueError> {
        let units = [
            f.years,
            f.months,
            f.weeks,
            f.days,
            f.hours,
            f.minutes,
            f.seconds,
            i64::from(f.nanoseconds),
        ];
        if units.contains(&i64::MIN)
            || f.nanoseconds.unsigned_abs() >= 1_000_000_000
            || (units.iter().any(|u| *u < 0) && units.iter().any(|u| *u > 0))
        {
            return Err(ValueError::Unrepresentable);
        }
        Ok(Span::new()
            .years(f.years)
            .months(f.months)
            .weeks(f.weeks)
            .days(f.days)
            .hours(f.hours)
            .minutes(f.minutes)
            .seconds(f.seconds)
            .nanoseconds(f.nanoseconds.into()))
    }
}

/// Reads a span into `out`: an ISO 8601 duration (`P1Y2M3W4DT5H6M7.5S`,
/// `-P1D`, designators in either case, a fraction on the seconds only) or
/// the friendly form (`1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns`, lowercase,
/// each unit once). The grammars are on the encodings page. Refusals as
/// `guatiao_date_parse`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_span_parse(
    alloc: *const Allocator,
    text: Str,
    out: *mut SpanFields,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { parse_into::<Span, _>(alloc, text, out, out_error) }
}

/// Writes `span` as its canonical ISO 8601 text (`-P1DT2H`, `PT0S` for
/// zero), as `guatiao_date_format` writes a date.
/// `GUATIAO_ERR_BAD_VALUE` for a struct that is not a span: see
/// `guatiao_span`.
///
/// # Safety
///
/// As `guatiao_date_format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_span_format(
    alloc: *const Allocator,
    span: *const SpanFields,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { format_from::<Span, _>(alloc, span, out) }
}

/// Reads a span as `guatiao_span_parse` does and writes its canonical
/// ISO 8601 text, as `guatiao_date_normalize` does: `1h 30m` becomes
/// `PT1H30M`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_span_normalize(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { normalize::<Span>(alloc, text, out, out_error) }
}

/// Reads an exact length of time from `value` into whole seconds and the
/// nanoseconds below a second, in any of its three spellings: the list
/// `[seconds, nanoseconds]` it is written as, a number of seconds read
/// exactly from its text (`1.5`, `2e-3`; not negative, no finer than a
/// nanosecond), or the map `{"secs": .., "nanos": ..}`.
///
/// Writing one needs no function: it is a list of two numbers.
/// `GUATIAO_ERR_BAD_VALUE` for anything else; both outputs are zeroed
/// first.
///
/// # Safety
///
/// `value` is null or addresses a well-formed value; the out-pointers are
/// null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_duration_read(
    value: *const Value,
    out_seconds: *mut u64,
    out_nanoseconds: *mut u32,
) -> Status {
    // SAFETY: the caller's contract, for both.
    unsafe {
        if !out_seconds.is_null() {
            ptr::write(out_seconds, 0);
        }
        if !out_nanoseconds.is_null() {
            ptr::write(out_nanoseconds, 0);
        }
    }
    entry!(value, out_seconds, out_nanoseconds => {
        // SAFETY: checked non-null, a well-formed value by contract.
        match Duration::from_value(unsafe { &*value }) {
            Ok(d) => {
                // SAFETY: checked non-null, writable by contract.
                unsafe {
                    ptr::write(out_seconds, d.as_secs());
                    ptr::write(out_nanoseconds, d.subsec_nanos());
                }
                Status::GUATIAO_OK
            }
            Err(_) => Status::GUATIAO_ERR_BAD_VALUE,
        }
    })
}
