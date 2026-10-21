// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! RFC 3339 timestamps, as seconds and nanoseconds from the Unix epoch,
//! in the proleptic Gregorian calendar with no leap seconds: what
//! `SystemTime` counts.
//!
//! The day arithmetic is Howard Hinnant's `days_from_civil` and
//! `civil_from_days` (<https://howardhinnant.github.io/date_algorithms.html>).

#![forbid(unsafe_code)]

use alloc::string::String;
use core::fmt::Write;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// Days from 1970-01-01 to `y-m-d`, negative before it.
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01.
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
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

/// `secs` and `nanos` from the epoch as `YYYY-MM-DDTHH:MM:SS[.f]Z`, the
/// fraction as few digits as it needs and none when it is zero; `None`
/// outside the years 0000 to 9999, which four digits cannot write.
pub(crate) fn write_rfc3339(secs: i64, nanos: u32) -> Option<String> {
    let (y, m, d) = civil_from_days(secs.div_euclid(DAY));
    if !(0..=9999).contains(&y) {
        return None;
    }
    let of_day = secs.rem_euclid(DAY);
    let mut out = String::new();
    let _ = write!(
        out,
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        of_day / 3600,
        of_day / 60 % 60,
        of_day % 60
    );
    write_fraction(&mut out, nanos);
    out.push('Z');
    Some(out)
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

/// Why a timestamp was refused: the byte it was refused at and what was
/// expected there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Refused {
    pub(crate) at: usize,
    pub(crate) expected: &'static str,
}

/// A cursor over the text being read.
struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn refuse<T>(&self, expected: &'static str) -> Result<T, Refused> {
        Err(Refused {
            at: self.at,
            expected,
        })
    }

    /// Exactly `n` ASCII digits, as a number.
    fn digits(&mut self, n: usize, expected: &'static str) -> Result<u32, Refused> {
        let Some(run) = self.text.get(self.at..self.at + n) else {
            return self.refuse(expected);
        };
        if !run.iter().all(u8::is_ascii_digit) {
            return self.refuse(expected);
        }
        self.at += n;
        Ok(run.iter().fold(0, |v, b| v * 10 + u32::from(b - b'0')))
    }

    /// A number of exactly `n` digits no greater than `max`.
    fn field(&mut self, n: usize, max: u32, expected: &'static str) -> Result<u32, Refused> {
        let start = self.at;
        let v = self.digits(n, expected)?;
        if v > max {
            self.at = start;
            return self.refuse(expected);
        }
        Ok(v)
    }

    fn byte(&mut self, any_of: &[u8], expected: &'static str) -> Result<u8, Refused> {
        match self.text.get(self.at) {
            Some(b) if any_of.contains(b) => {
                self.at += 1;
                Ok(*b)
            }
            _ => self.refuse(expected),
        }
    }
}

/// Reads an RFC 3339 `date-time` (section 5.6): `YYYY-MM-DD`, `T` (or
/// `t`), `HH:MM:SS`, an optional `.` and one to nine digits, then `Z` (or
/// `z`) or an offset `+HH:MM` / `-HH:MM`. Seconds run 00 to 59: a leap
/// second has no instant here. The answer is UTC.
pub(crate) fn parse_rfc3339(text: &str) -> Result<(i64, u32), Refused> {
    let mut r = Reader {
        text: text.as_bytes(),
        at: 0,
    };
    let y = i64::from(r.digits(4, "a four-digit year")?);
    r.byte(b"-", "'-' after the year")?;
    let m_at = r.at;
    let m = r.field(2, 12, "a month from 01 to 12")?;
    if m == 0 {
        r.at = m_at;
        return r.refuse("a month from 01 to 12");
    }
    r.byte(b"-", "'-' after the month")?;
    let d_at = r.at;
    let d = r.field(2, 31, "a day of the month")?;
    if d == 0 || d > days_in_month(y, m) {
        r.at = d_at;
        return r.refuse("a day of the month");
    }
    r.byte(b"Tt", "'T' between the date and the time")?;
    let hh = r.field(2, 23, "an hour from 00 to 23")?;
    r.byte(b":", "':' after the hour")?;
    let mm = r.field(2, 59, "a minute from 00 to 59")?;
    r.byte(b":", "':' after the minute")?;
    let ss = r.field(2, 59, "a second from 00 to 59")?;
    let mut nanos = 0u32;
    if r.text.get(r.at) == Some(&b'.') {
        r.at += 1;
        let start = r.at;
        while r.text.get(r.at).is_some_and(u8::is_ascii_digit) {
            r.at += 1;
        }
        let n = r.at - start;
        if n == 0 || n > 9 {
            r.at = start;
            return r.refuse("one to nine digits of a second, no finer than a nanosecond");
        }
        let digits = &r.text[start..r.at];
        nanos =
            digits.iter().fold(0, |v, b| v * 10 + u32::from(b - b'0')) * 10u32.pow((9 - n) as u32);
    }
    let offset = match r.byte(b"Zz+-", "'Z' or an offset such as +05:30")? {
        b'Z' | b'z' => 0,
        sign => {
            let oh = i64::from(r.field(2, 23, "an offset hour from 00 to 23")?);
            r.byte(b":", "':' in the offset")?;
            let om = i64::from(r.field(2, 59, "an offset minute from 00 to 59")?);
            let o = oh * 3600 + om * 60;
            if sign == b'-' { -o } else { o }
        }
    };
    if r.at != r.text.len() {
        return r.refuse("nothing after the offset");
    }
    let local =
        days_from_civil(y, m, d) * DAY + i64::from(hh) * 3600 + i64::from(mm) * 60 + i64::from(ss);
    Ok((local - offset, nanos))
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
        assert_eq!(days_from_civil(0, 1, 1), -719_528);
    }

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
            assert_eq!(write_rfc3339(secs, nanos).as_deref(), Some(text));
            assert_eq!(parse_rfc3339(text), Ok((secs, nanos)));
        }
        assert_eq!(write_rfc3339(-62_167_219_201, 0), None);
        assert_eq!(write_rfc3339(253_402_300_800, 0), None);
        assert_eq!(
            parse_rfc3339("2026-10-21T13:45:00.12Z"),
            Ok((1_792_590_300, 120_000_000))
        );
    }
}
