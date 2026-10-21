// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Date`]: a day on the calendar, `2026-10-21`.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt;
use core::str::FromStr;

use super::{FIRST_DAY, LAST_DAY, Reader, YEARS, civil_from_days, days_from_civil, days_in_month};
use crate::value::error::ValueError;
use crate::value::span::Span;

/// A day on the proleptic Gregorian calendar, with no time of day and no
/// time zone: `2026-10-21`. The years run from 0000 to 9999.
///
/// Its text is RFC 3339's `full-date`, `YYYY-MM-DD`, what
/// [`Display`](fmt::Display) writes, [`FromStr`] reads and a value holds.
/// Dates order by the calendar.
///
/// ```
/// use guatiao::{Date, Span};
///
/// let date: Date = "2026-01-31".parse()?;
/// assert_eq!((date.year(), date.month(), date.day()), (2026, 1, 31));
/// // A month later is the last day of February: the day is clamped.
/// assert_eq!(date.checked_add("P1M".parse()?), Some("2026-02-28".parse()?));
/// assert!("2026-02-29".parse::<Date>().is_err());
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    // In this order, so the derived order is the calendar's.
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// The date `year-month-day`.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unrepresentable`] for a year outside 0000 to 9999, a
    /// month outside 1 to 12, or a day the month does not have.
    pub fn try_new(year: i32, month: u8, day: u8) -> Result<Date, ValueError> {
        if !YEARS.contains(&year)
            || !(1..=12).contains(&month)
            || day == 0
            || u32::from(day) > days_in_month(i64::from(year), u32::from(month))
        {
            return Err(ValueError::Unrepresentable);
        }
        Ok(Date::from_civil(year, month, day))
    }

    /// The year, 0 to 9999.
    pub fn year(&self) -> i32 {
        self.year
    }

    /// The month, 1 to 12.
    pub fn month(&self) -> u8 {
        self.month
    }

    /// The day of the month, from 1.
    pub fn day(&self) -> u8 {
        self.day
    }

    /// The date `span` later, or earlier for a negative span: years and
    /// months first, a day past the month's end becoming its last day,
    /// then weeks and days.
    ///
    /// `None` for a span with hours, minutes, seconds or nanoseconds,
    /// which a date has no time of day to carry, and for a step that
    /// lands outside the years 0000 to 9999.
    pub fn checked_add(self, span: Span) -> Option<Date> {
        if span.get_hours() != 0
            || span.get_minutes() != 0
            || span.get_seconds() != 0
            || span.get_nanoseconds() != 0
        {
            return None;
        }
        self.shifted(span, 0)
    }

    /// The date after the span's calendar units, then `more_days`.
    pub(super) fn shifted(self, span: Span, more_days: i128) -> Option<Date> {
        let months = i128::from(span.get_years()) * 12 + i128::from(span.get_months());
        let ym = i128::from(self.year) * 12 + i128::from(self.month) - 1 + months;
        let year = i32::try_from(ym.div_euclid(12))
            .ok()
            .filter(|y| YEARS.contains(y))?;
        let month = (ym.rem_euclid(12) + 1) as u8;
        let last = days_in_month(i64::from(year), u32::from(month)) as u8;
        let landed = Date::from_civil(year, month, self.day.min(last));
        let days = i128::from(landed.days())
            + i128::from(span.get_weeks()) * 7
            + i128::from(span.get_days())
            + more_days;
        if days < i128::from(FIRST_DAY) || days > i128::from(LAST_DAY) {
            return None;
        }
        Some(Date::from_days(days as i64))
    }

    /// A date already known to be on the calendar.
    pub(super) fn from_civil(year: i32, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    /// Days from 1970-01-01, negative before it.
    pub(super) fn days(self) -> i64 {
        days_from_civil(
            i64::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        )
    }

    /// The date `days` after 1970-01-01, which must be a writable day.
    pub(super) fn from_days(days: i64) -> Date {
        let (y, m, d) = civil_from_days(days);
        Date::from_civil(y as i32, m as u8, d as u8)
    }
}

/// `YYYY-MM-DD`.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// The text: `Date(2026-10-21)`.
impl fmt::Debug for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Date({self})")
    }
}

impl FromStr for Date {
    type Err = ValueError;

    /// Reads `YYYY-MM-DD`: four digits of year, a month from 01 to 12 and
    /// a day the month has, and nothing after.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unparsed`] with the byte offset it stopped at and
    /// what it expected there; it never quotes the text.
    fn from_str(text: &str) -> Result<Date, ValueError> {
        let mut r = Reader::new(text);
        let date = r.date()?;
        r.end("nothing after the date")?;
        Ok(date)
    }
}

impl TryFrom<&str> for Date {
    type Error = ValueError;

    fn try_from(text: &str) -> Result<Date, ValueError> {
        text.parse()
    }
}

impl From<Date> for String {
    fn from(date: Date) -> String {
        alloc::string::ToString::to_string(&date)
    }
}
