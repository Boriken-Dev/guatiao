// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Dates and times with no time zone: [`Date`], [`Time`], [`DateTime`]
//! and [`Timestamp`], each written as RFC 3339 text.
//!
//! The calendar is the proleptic Gregorian one with no leap seconds, and
//! every type covers the years 0000 to 9999, which is what four digits
//! write. The forms and their grammar are the "Dates and times" section of
//! the encodings page; what is here implements it once, for the Rust
//! types, `SystemTime` and the C functions alike.
//!
//! The day arithmetic is Howard Hinnant's `days_from_civil` and
//! `civil_from_days` (<https://howardhinnant.github.io/date_algorithms.html>).

#![forbid(unsafe_code)]

mod clock;
mod date;
mod datetime;
mod timestamp;

pub use clock::Time;
pub use date::Date;
pub use datetime::DateTime;
pub use timestamp::Timestamp;

use alloc::string::String;

use super::error::ValueError;

/// Seconds in a day.
pub(crate) const DAY: i64 = 86_400;
/// Nanoseconds in a second.
pub(crate) const NANOS_PER_SECOND: u32 = 1_000_000_000;
/// Nanoseconds in a day.
const DAY_NANOS: i128 = DAY as i128 * NANOS_PER_SECOND as i128;
/// The writable years.
const YEARS: core::ops::RangeInclusive<i32> = 0..=9999;
/// Days from 1970-01-01 to 0000-01-01 and to 9999-12-31: the writable
/// days.
const FIRST_DAY: i64 = -719_528;
const LAST_DAY: i64 = 2_932_896;

/// Days from 1970-01-01 to `y-m-d`, negative before it.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `.` and the nanoseconds with trailing zeros removed, or nothing for 0.
pub(crate) fn write_fraction(out: &mut String, nanos: u32) {
    if nanos == 0 {
        return;
    }
    let mut digits = alloc::format!("{nanos:09}");
    while digits.ends_with('0') {
        digits.pop();
    }
    out.push('.');
    out.push_str(&digits);
}

/// A cursor over the text being read. Every refusal is
/// [`ValueError::Unparsed`] at the byte it stopped at.
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Reader<'a> {
        Reader {
            text: text.as_bytes(),
            at: 0,
        }
    }

    fn refuse<T>(&self, expected: &'static str) -> Result<T, ValueError> {
        Err(ValueError::Unparsed {
            at: self.at,
            expected,
        })
    }

    /// Exactly `n` ASCII digits, as a number.
    fn digits(&mut self, n: usize, expected: &'static str) -> Result<u32, ValueError> {
        let Some(run) = self.text.get(self.at..self.at + n) else {
            return self.refuse(expected);
        };
        if !run.iter().all(u8::is_ascii_digit) {
            return self.refuse(expected);
        }
        self.at += n;
        Ok(run.iter().fold(0, |v, b| v * 10 + u32::from(b - b'0')))
    }

    /// Two digits from `min` to `max`.
    fn field(&mut self, min: u32, max: u32, expected: &'static str) -> Result<u32, ValueError> {
        let start = self.at;
        let v = self.digits(2, expected)?;
        if v < min || v > max {
            self.at = start;
            return self.refuse(expected);
        }
        Ok(v)
    }

    fn byte(&mut self, any_of: &[u8], expected: &'static str) -> Result<u8, ValueError> {
        match self.text.get(self.at) {
            Some(b) if any_of.contains(b) => {
                self.at += 1;
                Ok(*b)
            }
            _ => self.refuse(expected),
        }
    }

    fn end(&self, expected: &'static str) -> Result<(), ValueError> {
        if self.at == self.text.len() {
            Ok(())
        } else {
            self.refuse(expected)
        }
    }

    /// `YYYY-MM-DD`, a day the month has.
    fn date(&mut self) -> Result<Date, ValueError> {
        let y = self.digits(4, "a four-digit year")?;
        self.byte(b"-", "'-' after the year")?;
        let m = self.field(1, 12, "a month from 01 to 12")?;
        self.byte(b"-", "'-' after the month")?;
        let d_at = self.at;
        let d = self.field(1, 31, "a day of the month")?;
        if d > days_in_month(i64::from(y), m) {
            self.at = d_at;
            return self.refuse("a day of the month");
        }
        Ok(Date::from_civil(y as i32, m as u8, d as u8))
    }

    /// `HH:MM:SS` and an optional `.` with one to nine digits.
    fn time(&mut self) -> Result<Time, ValueError> {
        let hh = self.field(0, 23, "an hour from 00 to 23")?;
        self.byte(b":", "':' after the hour")?;
        let mm = self.field(0, 59, "a minute from 00 to 59")?;
        self.byte(b":", "':' after the minute")?;
        let ss = self.field(0, 59, "a second from 00 to 59")?;
        let mut nanos = 0u32;
        if self.text.get(self.at) == Some(&b'.') {
            self.at += 1;
            let start = self.at;
            while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            let n = self.at - start;
            if n == 0 || n > 9 {
                self.at = start;
                return self.refuse("one to nine digits of a second, no finer than a nanosecond");
            }
            let digits = &self.text[start..self.at];
            nanos = digits.iter().fold(0, |v, b| v * 10 + u32::from(b - b'0'))
                * 10u32.pow((9 - n) as u32);
        }
        Ok(Time::from_clock(hh as u8, mm as u8, ss as u8, nanos))
    }

    /// A date, `T` or `t`, and a time.
    fn date_time(&mut self) -> Result<DateTime, ValueError> {
        let date = self.date()?;
        self.byte(b"Tt", "'T' between the date and the time")?;
        Ok(DateTime::new(date, self.time()?))
    }

    /// `Z`, `z`, or `+HH:MM` / `-HH:MM`, as seconds east of UTC.
    fn offset(&mut self) -> Result<i64, ValueError> {
        match self.byte(b"Zz+-", "'Z' or an offset such as +05:30")? {
            b'Z' | b'z' => Ok(0),
            sign => {
                let oh = i64::from(self.field(0, 23, "an offset hour from 00 to 23")?);
                self.byte(b":", "':' in the offset")?;
                let om = i64::from(self.field(0, 59, "an offset minute from 00 to 59")?);
                let o = oh * 3600 + om * 60;
                Ok(if sign == b'-' { -o } else { o })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_round_trips_across_eras() {
        for days in [-1_000_000, -719_528, -1, 0, 1, 11_016, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(0, 1, 1), FIRST_DAY);
        assert_eq!(days_from_civil(9999, 12, 31), LAST_DAY);
    }
}
