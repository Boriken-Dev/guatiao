// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`DateTime`]: a civil date and time, `2026-10-21T13:45:00.5`.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt;
use core::str::FromStr;

use super::{DAY_NANOS, Date, NANOS_PER_SECOND, Reader, Time};
use crate::value::error::ValueError;
use crate::value::span::Span;

/// A date and a time of day as a wall clock shows them, with no time
/// zone: `2026-10-21T13:45:00.5`. Which instant it is depends on a zone
/// this type does not have; [`Timestamp`](super::Timestamp) is an instant.
///
/// Its text is the date, `T` and the time, what
/// [`Display`](fmt::Display) writes and a value holds. [`FromStr`] reads
/// `T` or `t` and refuses an offset. Date-times order by the calendar,
/// then the clock.
///
/// ```
/// use guatiao::DateTime;
///
/// let at: DateTime = "2026-10-21T23:30:00".parse()?;
/// assert_eq!(
///     at.checked_add("PT1H".parse()?),
///     Some("2026-10-22T00:30:00".parse()?),
/// );
/// assert!("2026-10-21T23:30:00Z".parse::<DateTime>().is_err());
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateTime {
    date: Date,
    time: Time,
}

impl DateTime {
    /// The time of day `time` on `date`.
    pub fn new(date: Date, time: Time) -> DateTime {
        DateTime { date, time }
    }

    /// The date.
    pub fn date(&self) -> Date {
        self.date
    }

    /// The time of day.
    pub fn time(&self) -> Time {
        self.time
    }

    /// The date-time `span` later, or earlier for a negative span: years
    /// and months first, a day past the month's end becoming its last
    /// day, then weeks and days, then hours, minutes, seconds and
    /// nanoseconds, carried across midnight. A day is 24 hours: there is
    /// no zone to say otherwise.
    ///
    /// `None` for a step that lands outside the years 0000 to 9999.
    pub fn checked_add(self, span: Span) -> Option<DateTime> {
        let nanos = (i128::from(self.time.seconds_of_day())
            + i128::from(span.get_hours()) * 3600
            + i128::from(span.get_minutes()) * 60
            + i128::from(span.get_seconds()))
            * i128::from(NANOS_PER_SECOND)
            + i128::from(self.time.nanosecond())
            + i128::from(span.get_nanoseconds());
        let date = self.date.shifted(span, nanos.div_euclid(DAY_NANOS))?;
        let of_day = nanos.rem_euclid(DAY_NANOS);
        let per = i128::from(NANOS_PER_SECOND);
        let time = Time::from_seconds_of_day((of_day / per) as i64, (of_day % per) as u32);
        Some(DateTime::new(date, time))
    }
}

/// The date, `T`, the time.
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}T{}", self.date, self.time)
    }
}

/// The text: `DateTime(2026-10-21T13:45:00.5)`.
impl fmt::Debug for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DateTime({self})")
    }
}

impl FromStr for DateTime {
    type Err = ValueError;

    /// Reads a date as [`Date`] does, `T` or `t`, a time as [`Time`]
    /// does, and nothing after: no offset.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unparsed`] with the byte offset it stopped at and
    /// what it expected there; it never quotes the text.
    fn from_str(text: &str) -> Result<DateTime, ValueError> {
        let mut r = Reader::new(text);
        let at = r.date_time()?;
        r.end("nothing after the time: a civil date and time has no offset")?;
        Ok(at)
    }
}

impl TryFrom<&str> for DateTime {
    type Error = ValueError;

    fn try_from(text: &str) -> Result<DateTime, ValueError> {
        text.parse()
    }
}

impl From<DateTime> for String {
    fn from(at: DateTime) -> String {
        alloc::string::ToString::to_string(&at)
    }
}
