// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! `Span`: its ISO 8601 text, the friendly form, the refusals, and the
//! conversions to and from `Duration`.

use core::time::Duration;

use guatiao::{Span, ValueError};

fn parse(text: &str) -> Result<Span, ValueError> {
    text.parse()
}

/// A small deterministic generator, so the round trip covers many spans
/// without a dependency.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Mostly zero, sometimes small, sometimes as wide as a unit gets.
    fn unit(&mut self) -> i64 {
        match self.next() % 4 {
            0 | 1 => 0,
            2 => (self.next() % 100) as i64,
            _ => (self.next() >> 1) as i64,
        }
    }
}

#[test]
fn display_then_parse_gives_back_the_span() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for _ in 0..10_000 {
        let span = Span::new()
            .years(rng.unit())
            .months(rng.unit())
            .weeks(rng.unit())
            .days(rng.unit())
            .hours(rng.unit())
            .minutes(rng.unit())
            .seconds(rng.unit())
            .nanoseconds((rng.next() % 1_000_000_000) as i64 * (rng.next() % 2) as i64);
        let span = if rng.next().is_multiple_of(2) { span } else { -span };
        let text = span.to_string();
        assert_eq!(parse(&text), Ok(span), "{text}");
        assert_eq!(format!("{span:?}"), format!("Span({text})"));
    }
}

#[test]
fn the_canonical_text() {
    let all = Span::new()
        .years(1)
        .months(2)
        .weeks(3)
        .days(4)
        .hours(5)
        .minutes(6)
        .seconds(7)
        .nanoseconds(500_000_000);
    assert_eq!(all.to_string(), "P1Y2M3W4DT5H6M7.5S");
    assert_eq!(Span::new().to_string(), "PT0S");
    assert_eq!(Span::new().days(-1).to_string(), "-P1D");
    assert_eq!(Span::new().minutes(1).to_string(), "PT1M");
    assert_eq!(Span::new().months(1).to_string(), "P1M");
    assert_eq!(Span::new().nanoseconds(1).to_string(), "PT0.000000001S");
    assert_eq!(
        Span::new().nanoseconds(1_500_000_000).to_string(),
        "PT1.5S",
        "a billion nanoseconds carry into a second"
    );
}

#[test]
fn iso_8601_reads_every_unit_in_either_case() {
    let all = Span::new()
        .years(1)
        .months(2)
        .weeks(3)
        .days(4)
        .hours(5)
        .minutes(6)
        .seconds(7)
        .nanoseconds(500_000_000);
    assert_eq!(parse("P1Y2M3W4DT5H6M7.5S"), Ok(all));
    assert_eq!(parse("p1y2m3w4dt5h6m7.5s"), Ok(all));
    assert_eq!(parse("+P1D"), Ok(Span::new().days(1)));
    assert_eq!(parse("-PT0.25S"), Ok(Span::new().nanoseconds(-250_000_000)));
    assert_eq!(parse("PT0S"), Ok(Span::new()));
    assert_eq!(parse("-PT0S"), Ok(Span::new()), "zero has no sign");
    assert_eq!(parse("P0D"), Ok(Span::new()));
    assert_eq!(parse("PT1M"), Ok(Span::new().minutes(1)));
    assert_eq!(parse("P1M"), Ok(Span::new().months(1)));
}

#[test]
fn the_friendly_form() {
    let all = Span::new()
        .years(1)
        .months(2)
        .weeks(3)
        .days(4)
        .hours(5)
        .minutes(6)
        .seconds(7)
        .nanoseconds(500_001_001);
    assert_eq!(parse("1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns"), Ok(all));
    assert_eq!(parse("1y2mo3w4d5h6m7s500ms1us1ns"), Ok(all));
    assert_eq!(parse("1ns 1µs 500ms 7s 6min 5h 4d 3w 2mo 1y"), Ok(all));
    assert_eq!(parse("1h 30m"), Ok(Span::new().hours(1).minutes(30)));
    assert_eq!(parse("90min"), Ok(Span::new().minutes(90)));
    assert_eq!(
        parse("1.5s"),
        Ok(Span::new().seconds(1).nanoseconds(500_000_000))
    );
    assert_eq!(parse("-1d 12h"), Ok(Span::new().days(1).hours(12).negate()));
    assert_eq!(parse("  2d  "), Ok(Span::new().days(2)));
    assert_eq!(
        parse("1500ms"),
        Ok(Span::new().seconds(1).nanoseconds(500_000_000))
    );
    assert_eq!(parse("0s"), Ok(Span::new()));
}

/// Where each refusal stops, and that it says what it wanted.
#[test]
fn refusals_name_the_byte_and_never_the_text() {
    for (text, at) in [
        ("", 0),
        ("P", 1),
        ("PT", 2),
        ("P1", 2),
        ("P1X", 2),
        ("P1D2Y", 4),
        ("P1.5D", 1),
        ("PT1.5H", 2),
        ("PT1H2H", 5),
        ("PT1S2M", 5),
        ("P1DT", 4),
        ("PT1.S", 4),
        ("PT1.1234567891S", 13),
        ("P99999999999999999999D", 1),
        ("1x", 1),
        ("1d 2d", 4),
        ("1.5h", 0),
        ("d", 0),
        ("1H", 1),
        ("-", 1),
        ("+1d", 0),
        ("P1D junk", 3),
    ] {
        match parse(text) {
            Err(ValueError::Unparsed { at: got, expected }) => {
                assert_eq!(got, at, "{text:?}: {expected}");
                let message = parse(text).unwrap_err().to_string();
                assert!(message.contains(expected));
            }
            other => panic!("{text:?} gave {other:?}"),
        }
    }
}

#[test]
fn the_api() {
    let s = Span::new().days(2).hours(-3);
    assert!(
        s.is_negative(),
        "a negative argument sets the whole span's sign"
    );
    assert_eq!(s.get_days(), -2);
    assert_eq!(s.get_hours(), -3);
    assert_eq!(s.to_string(), "-P2DT3H");
    assert_eq!(s.abs().to_string(), "P2DT3H");
    assert_eq!(s.negate(), s.abs());
    assert_eq!(-s, s.abs());
    assert!(Span::new().is_zero());
    assert!(!Span::new().negate().is_negative());
    assert_eq!(Span::new().years(i64::MIN).get_years(), -i64::MAX);
    // Unit by unit: a day is not 24 hours.
    assert_ne!(parse("P1D"), parse("PT24H"));
    assert_eq!(Span::try_from("PT1H"), Ok(Span::new().hours(1)));
    assert_eq!(String::from(Span::new().weeks(2)), "P2W");
}

#[test]
fn duration_converts_where_the_units_are_exact() {
    let d = Duration::new(3661, 500_000_000);
    let span = Span::from(d);
    assert_eq!(span.to_string(), "PT1H1M1.5S");
    assert_eq!(Duration::try_from(span), Ok(d));
    assert_eq!(
        Duration::try_from(parse("P1W1DT1S").unwrap()),
        Ok(Duration::from_secs(8 * 86_400 + 1))
    );
    for refused in ["P1Y", "P1M", "-PT1S"] {
        assert_eq!(
            Duration::try_from(parse(refused).unwrap()),
            Err(ValueError::Unrepresentable),
            "{refused}"
        );
    }
    assert_eq!(
        Duration::try_from(Span::new().weeks(i64::MAX)),
        Err(ValueError::Unrepresentable)
    );
    assert_eq!(Span::from(Duration::ZERO), Span::new());
}

#[test]
fn a_span_is_send_sync_and_copy() {
    fn assert<T: Send + Sync + Copy + core::hash::Hash + Eq>() {}
    assert::<Span>();
}
