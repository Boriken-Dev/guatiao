// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dates, times of day, civil date-times and instants, from C.
//!
//! Each form has three functions: `_parse` reads text into a plain
//! struct, `_format` writes a struct as canonical text, and `_normalize`
//! rewrites text in its canonical spelling. They are the Rust types'
//! own grammar behind a C signature, so a C, Python or Dart reader
//! accepts exactly what a Rust one does.

#![allow(non_camel_case_types)]

use core::fmt::Display;
use core::ptr;
use core::str::FromStr;

use super::{entry, out};
use crate::value::alloc::{Alloc, Allocator};
use crate::value::error::ValueError;
use crate::value::span::Span;
use crate::value::status::Status;
use crate::value::time::{Date, DateTime, Time, Timestamp};
use crate::value::types::{Str, Text, Value};

use super::span::SpanFields;

/// A day on the calendar, as `guatiao_date_parse` reads it: the year
/// (0 to 9999), the month (1 to 12) and the day of the month (from 1). A
/// function given one refuses a day the month does not have with
/// `GUATIAO_ERR_BAD_VALUE`. Zeroed, it is not a date.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateFields {
    /// 0 to 9999.
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to the month's last day.
    pub day: u8,
}

/// A time of day with no offset: hour 0 to 23, minute and second 0 to
/// 59 (no leap second), and the nanoseconds below the second, 0 to
/// 999999999. Zeroed, it is midnight.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeFields {
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
    /// 0 to 59.
    pub second: u8,
    /// 0 to 999999999.
    pub nanosecond: u32,
}

/// A civil date and time: a date and a time of day as a wall clock
/// shows them, with no time zone.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateTimeFields {
    /// The day.
    pub date: DateFields,
    /// The time of day.
    pub time: TimeFields,
}

/// An instant, as C's `struct timespec` counts one: whole seconds from
/// 1970-01-01T00:00:00Z (negative before it), then nanoseconds counted
/// FORWARD, 0 to 999999999, so one nanosecond before the epoch is
/// `{-1, 999999999}`. The instants run from 0000-01-01T00:00:00Z to
/// 9999-12-31T23:59:59.999999999Z, with no leap seconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimestampFields {
    /// Whole seconds from the epoch, negative before it.
    pub seconds: i64,
    /// 0 to 999999999.
    pub nanoseconds: u32,
}

/// Why a text was refused: the byte offset the reading stopped at, and
/// what would have been accepted there.
///
/// Written `{0, ABSENT}` on entry by every function that takes one.
/// After `GUATIAO_ERR_BAD_VALUE`, `at` is the offset and `expected` a
/// string built through the call's allocator, which the caller frees with
/// `guatiao_value_free`; it stays ABSENT when the call had no usable
/// allocator. Neither ever quotes the text. Text that is not UTF-8 is
/// refused at its first bad byte.
#[repr(C)]
#[derive(Debug)]
pub struct TextError {
    /// The byte offset the reading stopped at.
    pub at: usize,
    /// What would have been accepted there, as a string; ABSENT when
    /// there is no refusal or no allocator to build it with.
    pub expected: Value,
}

const _: () = {
    use core::mem::{offset_of, size_of};
    assert!(size_of::<DateFields>() == 8 && offset_of!(DateFields, month) == 4);
    assert!(offset_of!(DateFields, day) == 5);
    assert!(size_of::<TimeFields>() == 8 && offset_of!(TimeFields, nanosecond) == 4);
    assert!(size_of::<DateTimeFields>() == 16 && offset_of!(DateTimeFields, time) == 8);
    assert!(size_of::<TimestampFields>() == 16 && offset_of!(TimestampFields, nanoseconds) == 8);
    assert!(offset_of!(TextError, expected) == size_of::<usize>());
};

impl From<Date> for DateFields {
    fn from(d: Date) -> DateFields {
        DateFields {
            year: d.year(),
            month: d.month(),
            day: d.day(),
        }
    }
}

/// [`ValueError::Unrepresentable`] for a day that is not on the calendar.
impl TryFrom<DateFields> for Date {
    type Error = ValueError;

    fn try_from(f: DateFields) -> Result<Date, ValueError> {
        Date::try_new(f.year, f.month, f.day)
    }
}

impl From<Time> for TimeFields {
    fn from(t: Time) -> TimeFields {
        TimeFields {
            hour: t.hour(),
            minute: t.minute(),
            second: t.second(),
            nanosecond: t.nanosecond(),
        }
    }
}

/// [`ValueError::Unrepresentable`] for a time that is not on the clock.
impl TryFrom<TimeFields> for Time {
    type Error = ValueError;

    fn try_from(f: TimeFields) -> Result<Time, ValueError> {
        Time::try_new(f.hour, f.minute, f.second, f.nanosecond)
    }
}

impl From<DateTime> for DateTimeFields {
    fn from(at: DateTime) -> DateTimeFields {
        DateTimeFields {
            date: at.date().into(),
            time: at.time().into(),
        }
    }
}

impl TryFrom<DateTimeFields> for DateTime {
    type Error = ValueError;

    fn try_from(f: DateTimeFields) -> Result<DateTime, ValueError> {
        Ok(DateTime::new(f.date.try_into()?, f.time.try_into()?))
    }
}

impl From<Timestamp> for TimestampFields {
    fn from(t: Timestamp) -> TimestampFields {
        TimestampFields {
            seconds: t.seconds(),
            nanoseconds: t.nanoseconds(),
        }
    }
}

impl TryFrom<TimestampFields> for Timestamp {
    type Error = ValueError;

    fn try_from(f: TimestampFields) -> Result<Timestamp, ValueError> {
        Timestamp::from_unix(f.seconds, f.nanoseconds)
    }
}

// --- the shared bodies ----------------------------------------------------

/// Writes `{0, ABSENT}` through a non-null `out_error`.
///
/// # Safety
///
/// `out_error` is null or writable storage for one, holding nothing the
/// caller still owns.
unsafe fn clear_error(out_error: *mut TextError) {
    if !out_error.is_null() {
        // SAFETY: the caller's contract.
        unsafe {
            ptr::write(
                out_error,
                TextError {
                    at: 0,
                    expected: Value::absent(),
                },
            )
        };
    }
}

/// `text` read as `T`, or `GUATIAO_ERR_BAD_VALUE` with the refusal
/// written through `out_error` (the message built through `alloc` when
/// it can be).
///
/// # Safety
///
/// The view's bytes are readable; `alloc` and `out_error` are null or
/// valid, `out_error` already cleared.
unsafe fn read<T: FromStr<Err = ValueError>>(
    alloc: *const Allocator,
    text: Str,
    out_error: *mut TextError,
) -> Result<T, Status> {
    let bytes: &[u8] = text.into();
    let refused = match core::str::from_utf8(bytes) {
        Ok(text) => match text.parse::<T>() {
            Ok(v) => return Ok(v),
            Err(ValueError::Unparsed { at, expected }) => (at, expected),
            Err(e) => return Err(e.into()),
        },
        Err(e) => (e.valid_up_to(), "UTF-8 text"),
    };
    if !out_error.is_null() {
        // SAFETY: `alloc` is null or valid by the caller's contract.
        let expected = match unsafe { Alloc::from_raw(alloc) } {
            Ok(alloc) => {
                Text::new_in(alloc, refused.1).map_or_else(|_| Value::absent(), Value::from)
            }
            Err(_) => Value::absent(),
        };
        // SAFETY: non-null, writable, and cleared on entry: nothing is
        // overwritten that the caller owns.
        unsafe {
            ptr::write(
                out_error,
                TextError {
                    at: refused.0,
                    expected,
                },
            )
        };
    }
    Err(Status::GUATIAO_ERR_BAD_VALUE)
}

/// The body of every `_parse`: `out` zeroed first, then the text read
/// into it.
///
/// # Safety
///
/// As [`read`], and `out` is null or writable.
pub(super) unsafe fn parse_into<T, F>(
    alloc: *const Allocator,
    text: Str,
    out: *mut F,
    out_error: *mut TextError,
) -> Status
where
    T: FromStr<Err = ValueError>,
    F: From<T> + Default,
{
    // SAFETY: the caller's contract, for both.
    unsafe {
        if !out.is_null() {
            ptr::write(out, F::default());
        }
        clear_error(out_error);
    }
    entry!(out => {
        // SAFETY: forwarded.
        match unsafe { read::<T>(alloc, text, out_error) } {
            Ok(v) => {
                // SAFETY: checked non-null, writable by contract.
                unsafe { ptr::write(out, F::from(v)) };
                Status::GUATIAO_OK
            }
            Err(status) => status,
        }
    })
}

/// `v`'s canonical text, built through `alloc` and moved into `out`.
///
/// # Safety
///
/// `alloc` is null or valid; `out` is writable.
unsafe fn write_text(alloc: *const Allocator, v: impl Display, out: *mut Value) -> Status {
    // SAFETY: the caller's contract.
    let Ok(alloc) = (unsafe { Alloc::from_raw(alloc) }) else {
        return Status::GUATIAO_ERR_ALLOC;
    };
    match Text::new_in(alloc, &alloc::string::ToString::to_string(&v)) {
        Ok(text) => {
            // SAFETY: writable by contract, holding ABSENT.
            unsafe { ptr::write(out, Value::from(text)) };
            Status::GUATIAO_OK
        }
        Err(e) => e.into(),
    }
}

/// The body of every `_format`.
///
/// # Safety
///
/// `fields` is null or readable; `alloc` null or valid; `out` null or
/// writable.
pub(super) unsafe fn format_from<T, F>(
    alloc: *const Allocator,
    fields: *const F,
    out: *mut Value,
) -> Status
where
    T: TryFrom<F, Error = ValueError> + Display,
    F: Copy,
{
    out!(out);
    entry!(fields, out => {
        // SAFETY: checked non-null, readable by contract.
        match T::try_from(unsafe { *fields }) {
            // SAFETY: forwarded.
            Ok(v) => unsafe { write_text(alloc, v, out) },
            Err(e) => e.into(),
        }
    })
}

/// The body of every `_normalize`.
///
/// # Safety
///
/// As [`read`], and `out` is null or writable.
pub(super) unsafe fn normalize<T>(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status
where
    T: FromStr<Err = ValueError> + Display,
{
    out!(out);
    // SAFETY: the caller's contract.
    unsafe { clear_error(out_error) };
    entry!(out => {
        // SAFETY: forwarded.
        match unsafe { read::<T>(alloc, text, out_error) } {
            // SAFETY: forwarded.
            Ok(v) => unsafe { write_text(alloc, v, out) },
            Err(status) => status,
        }
    })
}

// --- dates ----------------------------------------------------------------

/// Reads `YYYY-MM-DD` (RFC 3339 `full-date`) into `out`.
///
/// `GUATIAO_ERR_BAD_VALUE` for text the grammar refuses, with the byte
/// offset and what was expected written through `out_error`, which may be
/// null; `alloc` builds that message and may be null too. `out` is zeroed
/// first, and `GUATIAO_ERR_NULL` when it is null.
///
/// # Safety
///
/// The view's bytes are readable for the call; every non-null pointer
/// addresses what its type says, and the out-pointers writable storage
/// holding nothing the caller still owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_date_parse(
    alloc: *const Allocator,
    text: Str,
    out: *mut DateFields,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { parse_into::<Date, _>(alloc, text, out, out_error) }
}

/// Writes `date` as `YYYY-MM-DD`: a string value built through `alloc`,
/// moved into `out`, which is written ABSENT first.
///
/// `GUATIAO_ERR_BAD_VALUE` for a day that is not on the calendar,
/// `GUATIAO_ERR_ALLOC` for a null or unusable allocator.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says; `out` addresses
/// writable storage holding nothing the caller still owns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_date_format(
    alloc: *const Allocator,
    date: *const DateFields,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { format_from::<Date, _>(alloc, date, out) }
}

/// Reads a date as `guatiao_date_parse` does and writes its canonical
/// text into `out`, a string value built through `alloc`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_date_normalize(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { normalize::<Date>(alloc, text, out, out_error) }
}

/// The date `span` after `date` (before it for a negative span), into
/// `out`: years and months first, a day past the month's end becoming
/// its last day (2026-01-31 plus P1M is 2026-02-28), then weeks and days.
///
/// `GUATIAO_ERR_BAD_VALUE` for a date or a span that is not one, a span
/// with hours, minutes, seconds or nanoseconds (a date has no time of day
/// to carry them), and a step that lands outside the years 0000 to 9999.
/// `out` is zeroed first; `out` may be `date`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_date_add_span(
    date: *const DateFields,
    span: *const SpanFields,
    out: *mut DateFields,
) -> Status {
    entry!(date, span, out => {
        // SAFETY: checked non-null, readable by contract; read before
        // `out` is written, which may be the same struct.
        let (date, span) = unsafe { (*date, *span) };
        // SAFETY: checked non-null, writable by contract.
        unsafe { ptr::write(out, DateFields::default()) };
        let sum = Date::try_from(date)
            .and_then(|d| Ok(d.checked_add(Span::try_from(span)?)))
            .ok()
            .flatten();
        match sum {
            Some(d) => {
                // SAFETY: as above.
                unsafe { ptr::write(out, d.into()) };
                Status::GUATIAO_OK
            }
            None => Status::GUATIAO_ERR_BAD_VALUE,
        }
    })
}

// --- times of day -----------------------------------------------------------

/// Reads `HH:MM:SS[.fraction]` (RFC 3339 `partial-time`: one to nine
/// fractional digits, no offset) into `out`. Refusals as
/// `guatiao_date_parse`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_time_parse(
    alloc: *const Allocator,
    text: Str,
    out: *mut TimeFields,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { parse_into::<Time, _>(alloc, text, out, out_error) }
}

/// Writes `time` as `HH:MM:SS`, then `.` and the fraction with trailing
/// zeros removed when there is one, as `guatiao_date_format` writes a
/// date. `GUATIAO_ERR_BAD_VALUE` for a time that is not on the clock.
///
/// # Safety
///
/// As `guatiao_date_format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_time_format(
    alloc: *const Allocator,
    time: *const TimeFields,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { format_from::<Time, _>(alloc, time, out) }
}

/// Reads a time of day as `guatiao_time_parse` does and writes its
/// canonical text, as `guatiao_date_normalize` does.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_time_normalize(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { normalize::<Time>(alloc, text, out, out_error) }
}

// --- civil dates and times ------------------------------------------------

/// Reads a date, `T` or `t`, and a time of day, with no offset, into
/// `out`. Refusals as `guatiao_date_parse`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_datetime_parse(
    alloc: *const Allocator,
    text: Str,
    out: *mut DateTimeFields,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { parse_into::<DateTime, _>(alloc, text, out, out_error) }
}

/// Writes `datetime` as the date, `T` and the time, as
/// `guatiao_date_format` writes a date.
///
/// # Safety
///
/// As `guatiao_date_format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_datetime_format(
    alloc: *const Allocator,
    datetime: *const DateTimeFields,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { format_from::<DateTime, _>(alloc, datetime, out) }
}

/// Reads a civil date and time as `guatiao_datetime_parse` does and
/// writes its canonical text, as `guatiao_date_normalize` does.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_datetime_normalize(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { normalize::<DateTime>(alloc, text, out, out_error) }
}

/// The civil date and time `span` after `datetime`, into `out`: as
/// `guatiao_date_add_span`, then hours, minutes, seconds and nanoseconds
/// carried across midnight (a day is 24 hours: there is no zone).
///
/// `GUATIAO_ERR_BAD_VALUE` for a date-time or a span that is not one,
/// and a step that lands outside the years 0000 to 9999. `out` is zeroed
/// first; `out` may be `datetime`.
///
/// # Safety
///
/// Every non-null pointer addresses what its type says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_datetime_add_span(
    datetime: *const DateTimeFields,
    span: *const SpanFields,
    out: *mut DateTimeFields,
) -> Status {
    entry!(datetime, span, out => {
        // SAFETY: checked non-null, readable by contract; read before
        // `out` is written, which may be the same struct.
        let (at, span) = unsafe { (*datetime, *span) };
        // SAFETY: checked non-null, writable by contract.
        unsafe { ptr::write(out, DateTimeFields::default()) };
        let sum = DateTime::try_from(at)
            .and_then(|at| Ok(at.checked_add(Span::try_from(span)?)))
            .ok()
            .flatten();
        match sum {
            Some(at) => {
                // SAFETY: as above.
                unsafe { ptr::write(out, at.into()) };
                Status::GUATIAO_OK
            }
            None => Status::GUATIAO_ERR_BAD_VALUE,
        }
    })
}

// --- instants -------------------------------------------------------------

/// Reads an RFC 3339 `date-time` with any offset (`Z`, `z`, `+HH:MM`,
/// `-HH:MM`; `-00:00` is UTC) into `out`, as the instant it names. An
/// offset that moves the instant outside the years 0000 to 9999 is
/// refused at the offset. Refusals as `guatiao_date_parse`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_timestamp_parse(
    alloc: *const Allocator,
    text: Str,
    out: *mut TimestampFields,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { parse_into::<Timestamp, _>(alloc, text, out, out_error) }
}

/// Writes `timestamp` in UTC, `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, as
/// `guatiao_date_format` writes a date. `GUATIAO_ERR_BAD_VALUE` for
/// nanoseconds past 999999999 or an instant outside the years.
///
/// # Safety
///
/// As `guatiao_date_format`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_timestamp_format(
    alloc: *const Allocator,
    timestamp: *const TimestampFields,
    out: *mut Value,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { format_from::<Timestamp, _>(alloc, timestamp, out) }
}

/// Reads an instant as `guatiao_timestamp_parse` does and writes it in
/// UTC, as `guatiao_date_normalize` does: `2026-10-21T19:15:00+05:30`
/// becomes `2026-10-21T13:45:00Z`.
///
/// # Safety
///
/// As `guatiao_date_parse`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn guatiao_timestamp_normalize(
    alloc: *const Allocator,
    text: Str,
    out: *mut Value,
    out_error: *mut TextError,
) -> Status {
    // SAFETY: forwarded from this function's contract.
    unsafe { normalize::<Timestamp>(alloc, text, out, out_error) }
}
