// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Time`]: a time of day, `13:45:00.5`.

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt;
use core::str::FromStr;

use super::{NANOS_PER_SECOND, Reader, write_fraction};
use crate::value::error::ValueError;

/// A time of day to the nanosecond, with no date and no time zone:
/// `13:45:00.5`. From `00:00:00` to `23:59:59.999999999`; there is no leap
/// second.
///
/// Its text is RFC 3339's `partial-time`, `HH:MM:SS`, then `.` and the
/// fraction with trailing zeros removed (none when it is zero): what
/// [`Display`](fmt::Display) writes and a value holds. [`FromStr`] reads
/// one to nine fractional digits. Times order by the clock.
///
/// ```
/// use guatiao::Time;
///
/// let time: Time = "13:45:00.500".parse()?;
/// assert_eq!(time.to_string(), "13:45:00.5");
/// assert_eq!(time.nanosecond(), 500_000_000);
/// assert!("13:45:00Z".parse::<Time>().is_err(), "a time of day has no offset");
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Time {
    // In this order, so the derived order is the clock's.
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

impl Time {
    /// The time `hour:minute:second` and `nanosecond` billionths.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unrepresentable`] for an hour past 23, a minute or
    /// second past 59, or a nanosecond past 999 999 999.
    pub fn try_new(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Result<Time, ValueError> {
        if hour > 23 || minute > 59 || second > 59 || nanosecond >= NANOS_PER_SECOND {
            return Err(ValueError::Unrepresentable);
        }
        Ok(Time::from_clock(hour, minute, second, nanosecond))
    }

    /// Midnight, `00:00:00`.
    pub fn midnight() -> Time {
        Time::from_clock(0, 0, 0, 0)
    }

    /// The hour, 0 to 23.
    pub fn hour(&self) -> u8 {
        self.hour
    }

    /// The minute, 0 to 59.
    pub fn minute(&self) -> u8 {
        self.minute
    }

    /// The second, 0 to 59.
    pub fn second(&self) -> u8 {
        self.second
    }

    /// The nanoseconds below the second, 0 to 999 999 999.
    pub fn nanosecond(&self) -> u32 {
        self.nanosecond
    }

    /// A time already known to be on the clock.
    pub(super) fn from_clock(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Time {
        Time {
            hour,
            minute,
            second,
            nanosecond,
        }
    }

    /// Whole seconds since midnight.
    pub(super) fn seconds_of_day(self) -> i64 {
        i64::from(self.hour) * 3600 + i64::from(self.minute) * 60 + i64::from(self.second)
    }

    /// The time `seconds` after midnight, which must be under a day.
    pub(super) fn from_seconds_of_day(seconds: i64, nanosecond: u32) -> Time {
        Time::from_clock(
            (seconds / 3600) as u8,
            (seconds / 60 % 60) as u8,
            (seconds % 60) as u8,
            nanosecond,
        )
    }
}

/// `HH:MM:SS`, and the fraction when there is one.
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fraction = String::new();
        write_fraction(&mut fraction, self.nanosecond);
        write!(
            f,
            "{:02}:{:02}:{:02}{fraction}",
            self.hour, self.minute, self.second
        )
    }
}

/// The text: `Time(13:45:00.5)`.
impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Time({self})")
    }
}

impl FromStr for Time {
    type Err = ValueError;

    /// Reads `HH:MM:SS` with an optional `.` and one to nine digits, and
    /// nothing after: no offset.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unparsed`] with the byte offset it stopped at and
    /// what it expected there; it never quotes the text.
    fn from_str(text: &str) -> Result<Time, ValueError> {
        let mut r = Reader::new(text);
        let time = r.time()?;
        r.end("nothing after the time: a time of day has no offset")?;
        Ok(time)
    }
}

impl TryFrom<&str> for Time {
    type Error = ValueError;

    fn try_from(text: &str) -> Result<Time, ValueError> {
        text.parse()
    }
}

impl From<Time> for String {
    fn from(time: Time) -> String {
        alloc::string::ToString::to_string(&time)
    }
}
