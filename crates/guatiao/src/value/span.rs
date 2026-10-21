// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! [`Span`]: a length of time in calendar units, written as an ISO 8601
//! duration.

#![forbid(unsafe_code)]

mod parse;

use alloc::string::String;
use core::fmt;
use core::str::FromStr;
use core::time::Duration;

use super::error::ValueError;

/// The units, in the order they are written.
const YEARS: usize = 0;
const MONTHS: usize = 1;
const WEEKS: usize = 2;
const DAYS: usize = 3;
const HOURS: usize = 4;
const MINUTES: usize = 5;
const SECONDS: usize = 6;
const NANOS: usize = 7;

/// Nanoseconds in a second.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// A length of time in **calendar units**: years, months, weeks, days,
/// hours, minutes, seconds and nanoseconds, each counted on its own, with
/// one sign for the whole span. Named after jiff's `Span`, and like it,
/// two spans are equal unit by unit: `P1D` is not `PT24H`, because a day
/// is not always 24 hours where a calendar applies it.
///
/// Its text is an ISO 8601 duration, `P1Y2M3W4DT5H6M7.5S`, `-P1D`, `PT0S`
/// for zero: what [`Display`](fmt::Display) writes and a value holds.
/// [`FromStr`] reads that form and a friendlier one, `1y 2mo 3w 4d 5h 6m
/// 7s 500ms`; the grammar of both is on [`Span::from_str`].
///
/// This crate holds no calendar: a host applies a span to a date with its
/// own date library. [`Duration`] converts both ways where the units are
/// exact (`TryFrom<Span>` counts a week as 7 days and a day as 24 hours).
///
/// ```
/// use guatiao::Span;
///
/// let span = Span::new().hours(1).minutes(30);
/// assert_eq!(span.to_string(), "PT1H30M");
/// assert_eq!("1h 30m".parse::<Span>()?, span);
/// assert_eq!("PT1H30M".parse::<Span>()?, span);
/// assert_eq!(span.negate().to_string(), "-PT1H30M");
/// # Ok::<(), guatiao::ValueError>(())
/// ```
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Every unit's magnitude, at most `i64::MAX`; the nanoseconds below
    /// a second, the rest carried into the seconds.
    units: [u64; 8],
    /// Never set on a span with every unit zero.
    negative: bool,
}

macro_rules! unit {
    ($($set:ident $get:ident $at:ident $what:literal),* $(,)?) => {$(
        #[doc = concat!("The same span with its ", $what, " set to `n`. A non-zero `n` sets the")]
        /// whole span's sign, as jiff's builders do; `i64::MIN` is read as
        /// `-i64::MAX`.
        #[must_use]
        pub fn $set(self, n: i64) -> Span {
            self.with($at, n)
        }

        #[doc = concat!("The span's ", $what, ", carrying its sign.")]
        pub fn $get(&self) -> i64 {
            self.signed($at)
        }
    )*};
}

impl Span {
    /// The zero span, `PT0S`.
    pub fn new() -> Span {
        Span::default()
    }

    unit!(
        years get_years YEARS "years",
        months get_months MONTHS "months",
        weeks get_weeks WEEKS "weeks",
        days get_days DAYS "days",
        hours get_hours HOURS "hours",
        minutes get_minutes MINUTES "minutes",
        seconds get_seconds SECONDS "seconds",
    );

    /// The same span with its nanoseconds set to `n`. A billion or more
    /// carry into the seconds, since the text form has no unit below one:
    /// `nanoseconds(1_500_000_000)` is `PT1.5S`. A non-zero `n` sets the
    /// sign, as the other builders do.
    #[must_use]
    pub fn nanoseconds(self, n: i64) -> Span {
        let magnitude = n.unsigned_abs().min(i64::MAX as u64);
        let mut out = self.with(NANOS, 0);
        let carried = out.units[SECONDS].saturating_add(magnitude / NANOS_PER_SECOND);
        out.units[SECONDS] = carried.min(i64::MAX as u64);
        out.units[NANOS] = magnitude % NANOS_PER_SECOND;
        if n != 0 {
            out.negative = n < 0;
        }
        out.normalised()
    }

    /// The span's nanoseconds below a second, carrying its sign.
    pub fn get_nanoseconds(&self) -> i64 {
        self.signed(NANOS)
    }

    /// Whether every unit is zero.
    pub fn is_zero(&self) -> bool {
        self.units.iter().all(|u| *u == 0)
    }

    /// Whether the span runs backwards: `-P1D`.
    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// The same span running forwards.
    #[must_use]
    pub fn abs(self) -> Span {
        Span {
            negative: false,
            ..self
        }
    }

    /// The same span with its sign turned over. Zero stays zero.
    #[must_use]
    pub fn negate(self) -> Span {
        Span {
            negative: !self.negative,
            ..self
        }
        .normalised()
    }

    fn with(mut self, at: usize, n: i64) -> Span {
        self.units[at] = n.unsigned_abs().min(i64::MAX as u64);
        if n != 0 {
            self.negative = n < 0;
        }
        self.normalised()
    }

    fn signed(&self, at: usize) -> i64 {
        // At most `i64::MAX`, by construction.
        let magnitude = self.units[at] as i64;
        if self.negative { -magnitude } else { magnitude }
    }

    /// Zero has no sign.
    fn normalised(mut self) -> Span {
        if self.is_zero() {
            self.negative = false;
        }
        self
    }

    /// A span from its parts: magnitudes and a sign. The nanoseconds must
    /// be below a second.
    pub(crate) fn from_parts(units: [u64; 8], negative: bool) -> Span {
        Span { units, negative }.normalised()
    }
}

impl core::ops::Neg for Span {
    type Output = Span;

    fn neg(self) -> Span {
        self.negate()
    }
}

/// The canonical ISO 8601 text: `-` for a negative span, `P`, then each
/// non-zero unit in order with its designator (`Y`, `M`, `W`, `D`, and
/// after `T`, `H`, `M`, `S`), the seconds with as few fractional digits as
/// the nanoseconds need. Zero is `PT0S`.
impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negative {
            f.write_str("-")?;
        }
        f.write_str("P")?;
        if self.is_zero() {
            return f.write_str("T0S");
        }
        let u = &self.units;
        for (at, designator) in [(YEARS, 'Y'), (MONTHS, 'M'), (WEEKS, 'W'), (DAYS, 'D')] {
            if u[at] != 0 {
                write!(f, "{}{designator}", u[at])?;
            }
        }
        if u[HOURS..].iter().any(|n| *n != 0) {
            f.write_str("T")?;
            for (at, designator) in [(HOURS, 'H'), (MINUTES, 'M')] {
                if u[at] != 0 {
                    write!(f, "{}{designator}", u[at])?;
                }
            }
            if u[SECONDS] != 0 || u[NANOS] != 0 {
                let mut seconds = alloc::format!("{}", u[SECONDS]);
                super::time::write_fraction(&mut seconds, u[NANOS] as u32);
                write!(f, "{seconds}S")?;
            }
        }
        Ok(())
    }
}

/// The text, as `String` and a number show theirs: `Span(P1DT2H)`.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Span({self})")
    }
}

impl FromStr for Span {
    type Err = ValueError;

    /// Reads an ISO 8601 duration or the friendly form.
    ///
    /// **ISO 8601**: an optional `+` or `-`, `P`, then any of `nY`, `nM`,
    /// `nW`, `nD` in that order, then optionally `T` and any of `nH`,
    /// `nM`, `nS` in that order, at least one unit in all. `n` is decimal
    /// digits; only the seconds may carry a fraction, `.` and one to nine
    /// digits. Designators are read in either case.
    ///
    /// **Friendly**: an optional `-`, then one or more `n` and a unit,
    /// spaces between them allowed and not required. Units: `y` years,
    /// `mo` months, `w` weeks, `d` days, `h` hours, `m` or `min` minutes,
    /// `s` seconds, `ms`, `us` (or `µs`) and `ns` for parts of a second.
    /// Each unit at most once, in any order; only `s` may carry a
    /// fraction, as above. Lowercase only.
    ///
    /// Each number is at most `i64::MAX`.
    ///
    /// # Errors
    ///
    /// [`ValueError::Unparsed`] with the byte offset it stopped at and
    /// what it expected there; it never quotes the text.
    fn from_str(text: &str) -> Result<Span, ValueError> {
        parse::span(text)
    }
}

impl TryFrom<&str> for Span {
    type Error = ValueError;

    fn try_from(text: &str) -> Result<Span, ValueError> {
        text.parse()
    }
}

/// Hours, minutes and seconds, the nanoseconds below them: `PT1H1M1.5S`
/// for 3661.5 seconds. Exact.
impl From<Duration> for Span {
    fn from(d: Duration) -> Span {
        let secs = d.as_secs();
        let mut units = [0u64; 8];
        units[HOURS] = secs / 3600;
        units[MINUTES] = secs % 3600 / 60;
        units[SECONDS] = secs % 60;
        units[NANOS] = u64::from(d.subsec_nanos());
        Span::from_parts(units, false)
    }
}

/// Exact units only: a week is 7 days and a day 24 hours here, which
/// holds wherever no calendar is applied. A span with years or months,
/// a negative one, or one past what a `Duration` holds is
/// [`ValueError::Unrepresentable`].
impl TryFrom<Span> for Duration {
    type Error = ValueError;

    fn try_from(span: Span) -> Result<Duration, ValueError> {
        let u = &span.units;
        if span.negative || u[YEARS] != 0 || u[MONTHS] != 0 {
            return Err(ValueError::Unrepresentable);
        }
        let secs = [
            (u[WEEKS], 7 * 86_400),
            (u[DAYS], 86_400),
            (u[HOURS], 3600),
            (u[MINUTES], 60),
            (u[SECONDS], 1),
        ]
        .iter()
        .try_fold(0u64, |total, (n, per)| {
            total.checked_add(n.checked_mul(*per)?)
        })
        .ok_or(ValueError::Unrepresentable)?;
        Ok(Duration::new(secs, u[NANOS] as u32))
    }
}

impl From<Span> for String {
    fn from(span: Span) -> String {
        alloc::string::ToString::to_string(&span)
    }
}
