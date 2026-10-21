// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Timestamp`]: an instant, `2026-10-21T13:45:00.5Z`, and the
//! conversions to and from `SystemTime`.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt;
use core::str::FromStr;
use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{DAY, Date, DateTime, FIRST_DAY, LAST_DAY, NANOS_PER_SECOND, Reader, Time};
use crate::value::error::ValueError;

/// An instant, to the nanosecond, from `0000-01-01T00:00:00Z` to
/// `9999-12-31T23:59:59.999999999Z`, counted as seconds and nanoseconds
/// from the Unix epoch with no leap seconds.
///
/// Its text is an RFC 3339 `date-time` in UTC, `2026-10-21T13:45:00.5Z`:
/// what [`Display`](fmt::Display) writes and a value holds. [`FromStr`]
/// reads any offset and converts to the instant it names, so the offset
/// is not kept. Timestamps order by time.
///
/// `SystemTime` converts both ways and crosses as the same text.
///
/// ```
/// use guatiao::Timestamp;
///
/// let t: Timestamp = "2026-10-21T19:15:00.5+05:30".parse()?;
/// assert_eq!(t.to_string(), "2026-10-21T13:45:00.5Z");
/// assert_eq!((t.seconds(), t.nanoseconds()), (1_792_590_300, 500_000_000));
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp {
    /// Within the writable years.
    seconds: i64,
    /// Below a second, counted forward even before the epoch.
    nanoseconds: u32,
}

impl Timestamp {
    /// The instant `seconds` after 1970-01-01T00:00:00Z (before it when
    /// negative), plus `nanoseconds`, counted forward.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unrepresentable`] for nanoseconds past 999 999 999
    /// or an instant outside the years 0000 to 9999.
    pub fn from_unix(seconds: i64, nanoseconds: u32) -> Result<Timestamp, ValueError> {
        if nanoseconds >= NANOS_PER_SECOND
            || !(FIRST_DAY * DAY..(LAST_DAY + 1) * DAY).contains(&seconds)
        {
            return Err(ValueError::Unrepresentable);
        }
        Ok(Timestamp {
            seconds,
            nanoseconds,
        })
    }

    /// Whole seconds from 1970-01-01T00:00:00Z, negative before it.
    pub fn seconds(&self) -> i64 {
        self.seconds
    }

    /// The nanoseconds after [`seconds`](Timestamp::seconds), 0 to
    /// 999 999 999: one nanosecond before the epoch is `-1` second and
    /// 999 999 999 nanoseconds.
    pub fn nanoseconds(&self) -> u32 {
        self.nanoseconds
    }

    /// The date and time of day in UTC.
    fn utc(self) -> DateTime {
        let date = Date::from_days(self.seconds.div_euclid(DAY));
        let time = Time::from_seconds_of_day(self.seconds.rem_euclid(DAY), self.nanoseconds);
        DateTime::new(date, time)
    }
}

/// The instant in UTC: the date, `T`, the time, `Z`.
impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}Z", self.utc())
    }
}

/// The text: `Timestamp(2026-10-21T13:45:00.5Z)`.
impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({self})")
    }
}

impl FromStr for Timestamp {
    type Err = ValueError;

    /// Reads an RFC 3339 `date-time` (section 5.6): a date and a time as
    /// [`DateTime`] reads them, then `Z` (or `z`) or an offset `+HH:MM` /
    /// `-HH:MM`, and nothing after. `-00:00` is UTC.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unparsed`] with the byte offset it stopped at and
    /// what it expected there, never quoting the text; an offset that
    /// moves the instant outside the years 0000 to 9999 is refused at the
    /// offset.
    fn from_str(text: &str) -> Result<Timestamp, ValueError> {
        let mut r = Reader::new(text);
        let local = r.date_time()?;
        let offset_at = r.at;
        let offset = r.offset()?;
        r.end("nothing after the offset")?;
        let seconds = local.date().days() * DAY + local.time().seconds_of_day() - offset;
        Timestamp::from_unix(seconds, local.time().nanosecond()).map_err(|_| ValueError::Unparsed {
            at: offset_at,
            expected: "an offset that keeps the instant within the years 0000 to 9999",
        })
    }
}

impl TryFrom<&str> for Timestamp {
    type Error = ValueError;

    fn try_from(text: &str) -> Result<Timestamp, ValueError> {
        text.parse()
    }
}

impl From<Timestamp> for String {
    fn from(t: Timestamp) -> String {
        alloc::string::ToString::to_string(&t)
    }
}

/// [`ValueError::Unrepresentable`] for a time outside the years 0000 to
/// 9999.
impl TryFrom<SystemTime> for Timestamp {
    type Error = ValueError;

    fn try_from(t: SystemTime) -> Result<Timestamp, ValueError> {
        let (seconds, nanoseconds) = match t.duration_since(UNIX_EPOCH) {
            Ok(d) => (
                i64::try_from(d.as_secs()).map_err(|_| ValueError::Unrepresentable)?,
                d.subsec_nanos(),
            ),
            Err(before) => {
                let d = before.duration();
                let secs = i64::try_from(d.as_secs()).map_err(|_| ValueError::Unrepresentable)?;
                match d.subsec_nanos() {
                    0 => (-secs, 0),
                    n => (-secs - 1, NANOS_PER_SECOND - n),
                }
            }
        };
        Timestamp::from_unix(seconds, nanoseconds)
    }
}

/// Kept to the platform clock's resolution (100 ns on Windows);
/// [`ValueError::Unrepresentable`] for an instant the clock cannot hold
/// (before 1601 on Windows).
impl TryFrom<Timestamp> for SystemTime {
    type Error = ValueError;

    fn try_from(t: Timestamp) -> Result<SystemTime, ValueError> {
        let whole = Duration::new(t.seconds.unsigned_abs(), 0);
        let at = if t.seconds >= 0 {
            UNIX_EPOCH.checked_add(whole)
        } else {
            UNIX_EPOCH.checked_sub(whole)
        };
        at.and_then(|at| at.checked_add(Duration::from_nanos(u64::from(t.nanoseconds))))
            .ok_or(ValueError::Unrepresentable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// The text form at its edges, to the nanosecond, whatever a
    /// platform's `SystemTime` can hold.
    #[test]
    fn the_text_form_reaches_year_zero_and_one_nanosecond() {
        for (secs, nanos, text) in [
            (-62_167_219_200, 0, "0000-01-01T00:00:00Z"),
            (
                253_402_300_799,
                999_999_999,
                "9999-12-31T23:59:59.999999999Z",
            ),
            (1_792_590_300, 1, "2026-10-21T13:45:00.000000001Z"),
            (-1, 500_000_000, "1969-12-31T23:59:59.5Z"),
        ] {
            let t = Timestamp::from_unix(secs, nanos).expect("in range");
            assert_eq!(t.to_string(), text);
            assert_eq!(text.parse::<Timestamp>(), Ok(t));
        }
        assert!(Timestamp::from_unix(-62_167_219_201, 0).is_err());
        assert!(Timestamp::from_unix(253_402_300_800, 0).is_err());
        assert!(Timestamp::from_unix(0, 1_000_000_000).is_err());
    }
}
