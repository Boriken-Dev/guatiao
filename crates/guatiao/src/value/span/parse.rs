// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Reading a [`Span`]: the ISO 8601 form and the friendly one, as
//! [`Span::from_str`](core::str::FromStr::from_str) documents them.

#![forbid(unsafe_code)]

use super::{DAYS, HOURS, MINUTES, MONTHS, NANOS, NANOS_PER_SECOND, SECONDS, Span, WEEKS, YEARS};
use crate::value::error::ValueError;

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn refuse<T>(&self, expected: &'static str) -> Result<T, ValueError> {
        Err(ValueError::Unparsed {
            at: self.at,
            expected,
        })
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn skip_spaces(&mut self) {
        while self.peek() == Some(b' ') {
            self.at += 1;
        }
    }

    /// Decimal digits, at most `i64::MAX`, and an optional `.` with one to
    /// nine more: `(whole, nanoseconds, had a fraction)`.
    fn number(&mut self) -> Result<(u64, u64, bool), ValueError> {
        let start = self.at;
        let mut whole: u64 = 0;
        while let Some(d) = self.peek().filter(u8::is_ascii_digit) {
            whole = match whole
                .checked_mul(10)
                .and_then(|w| w.checked_add(u64::from(d - b'0')))
                .filter(|w| *w <= i64::MAX as u64)
            {
                Some(w) => w,
                None => {
                    self.at = start;
                    return self.refuse("a number no greater than 9223372036854775807");
                }
            };
            self.at += 1;
        }
        if self.at == start {
            return self.refuse("a number");
        }
        if !self.eat(b'.') {
            return Ok((whole, 0, false));
        }
        let from = self.at;
        let mut nanos: u64 = 0;
        while let Some(d) = self.peek().filter(u8::is_ascii_digit) {
            if self.at - from == 9 {
                return self.refuse("no more than nine fractional digits");
            }
            nanos = nanos * 10 + u64::from(d - b'0');
            self.at += 1;
        }
        let n = self.at - from;
        if n == 0 {
            return self.refuse("a digit after the '.'");
        }
        Ok((whole, nanos * 10u64.pow(9 - n as u32), true))
    }
}

pub(super) fn span(text: &str) -> Result<Span, ValueError> {
    let mut r = Reader {
        text: text.as_bytes(),
        at: 0,
    };
    let negative = r.eat(b'-');
    // `+` belongs to the ISO form only.
    if !negative && r.peek() == Some(b'+') && matches!(r.text.get(1), Some(b'P' | b'p')) {
        r.at += 1;
    }
    let units = if matches!(r.peek(), Some(b'P' | b'p')) {
        r.at += 1;
        iso(&mut r)?
    } else {
        friendly(&mut r)?
    };
    Ok(Span::from_parts(units, negative))
}

/// After the `P`.
fn iso(r: &mut Reader<'_>) -> Result<[u64; 8], ValueError> {
    let mut units = [0u64; 8];
    let mut any = false;
    // The date part: Y, M, W, D, each at most once and in that order.
    let mut next = 0;
    while r.peek().is_some_and(|b| b.is_ascii_digit()) {
        let start = r.at;
        let (n, _, fraction) = r.number()?;
        if fraction {
            r.at = start;
            return r.refuse("a whole number: only the seconds take a fraction");
        }
        let at = match r.peek().map(|b| b.to_ascii_uppercase()) {
            Some(b'Y') => YEARS,
            Some(b'M') => MONTHS,
            Some(b'W') => WEEKS,
            Some(b'D') => DAYS,
            _ => return r.refuse("Y, M, W or D, or T before a time"),
        };
        if at < next {
            return r.refuse("the date units in the order Y, M, W, D, each once");
        }
        r.at += 1;
        units[at] = n;
        next = at + 1;
        any = true;
    }
    if matches!(r.peek(), Some(b'T' | b't')) {
        r.at += 1;
        let mut next = HOURS;
        let mut timed = false;
        while r.at < r.text.len() {
            let start = r.at;
            let (n, nanos, fraction) = r.number()?;
            let at = match r.peek().map(|b| b.to_ascii_uppercase()) {
                Some(b'H') => HOURS,
                Some(b'M') => MINUTES,
                Some(b'S') => SECONDS,
                _ => return r.refuse("H, M or S"),
            };
            if at < next {
                return r.refuse("the time units in the order H, M, S, each once");
            }
            if fraction && at != SECONDS {
                r.at = start;
                return r.refuse("a whole number: only the seconds take a fraction");
            }
            r.at += 1;
            units[at] = n;
            if at == SECONDS {
                units[NANOS] = nanos;
            }
            next = at + 1;
            timed = true;
        }
        if !timed {
            return r.refuse("a time unit after T");
        }
        any = true;
    }
    if r.at != r.text.len() {
        return r.refuse("nothing after the last unit");
    }
    if !any {
        return r.refuse("at least one unit");
    }
    Ok(units)
}

/// `1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns`.
fn friendly(r: &mut Reader<'_>) -> Result<[u64; 8], ValueError> {
    let mut units = [0u64; 8];
    // Which of the eleven spellings' units were seen: y mo w d h m s ms us ns.
    let mut seen = [false; 10];
    r.skip_spaces();
    if r.at == r.text.len() {
        return r.refuse("a number and a unit, such as 1h or PT1H");
    }
    while r.at < r.text.len() {
        let start = r.at;
        let (n, nanos, fraction) = r.number()?;
        let unit_at = r.at;
        let rest = &r.text[r.at..];
        let letters = rest
            .iter()
            .take_while(|b| b.is_ascii_alphabetic() || **b >= 0x80)
            .count();
        let (slot, nanos_per, len) = match &rest[..letters] {
            b"y" => (0, 0, 1),
            b"mo" => (1, 0, 2),
            b"w" => (2, 0, 1),
            b"d" => (3, 0, 1),
            b"h" => (4, 0, 1),
            b"m" => (5, 0, 1),
            b"min" => (5, 0, 3),
            b"s" => (6, 0, 1),
            b"ms" => (7, 1_000_000, 2),
            b"us" => (8, 1_000, 2),
            // `µs`, MICRO SIGN U+00B5 in UTF-8.
            b"\xC2\xB5s" => (8, 1_000, 3),
            b"ns" => (9, 1, 2),
            _ => {
                return r.refuse("a unit: y, mo, w, d, h, m, min, s, ms, us or ns");
            }
        };
        if fraction && slot != 6 {
            r.at = start;
            return r.refuse("a whole number: only seconds take a fraction");
        }
        if seen[slot] {
            r.at = unit_at;
            return r.refuse("each unit once");
        }
        seen[slot] = true;
        r.at += len;
        match slot {
            0..=6 => {
                units[[YEARS, MONTHS, WEEKS, DAYS, HOURS, MINUTES, SECONDS][slot]] = n;
                if slot == 6 {
                    units[NANOS] += nanos;
                }
            }
            _ => {
                // Parts of a second gather into the nanoseconds, any whole
                // seconds among them carried into the seconds.
                let total = n.checked_mul(nanos_per).ok_or(ValueError::Unparsed {
                    at: start,
                    expected: "a part of a second no larger than a u64 of nanoseconds",
                })?;
                units[NANOS] += total % NANOS_PER_SECOND;
                units[SECONDS] += total / NANOS_PER_SECOND;
            }
        }
        r.skip_spaces();
    }
    units[SECONDS] += units[NANOS] / NANOS_PER_SECOND;
    units[NANOS] %= NANOS_PER_SECOND;
    if units[SECONDS] > i64::MAX as u64 {
        return Err(ValueError::Unparsed {
            at: 0,
            expected: "seconds no greater than 9223372036854775807",
        });
    }
    Ok(units)
}
