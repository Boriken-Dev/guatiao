// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The Rust types against `tests/vectors/time.json`, the vectors the C
//! functions and the Python and Dart bindings are checked against too.

use core::fmt::Debug;
use core::str::FromStr;
use core::time::Duration;
use std::time::SystemTime;

use guatiao::{Date, DateTime, FromValue, Span, Time, Timestamp, ToValue, Value, ValueError};

#[path = "vectors/read.rs"]
mod vectors;

/// Every vector of `form`: an accepted text reads, writes its canonical
/// spelling, reads that back as the same value and has `fields`; a
/// refused one is refused at its offset. Through a value as well.
fn each<T>(form: &str, fields: impl Fn(&T) -> Vec<i64>)
where
    T: FromStr<Err = ValueError> + ToValue + FromValue + Debug + PartialEq + ToString,
{
    let doc = vectors::file();
    let cases = vectors::texts(&doc, form);
    assert!(!cases.is_empty(), "{form} has vectors");
    for case in cases {
        let read = case.text.parse::<T>();
        match &case.expect {
            Ok((canonical, expected)) => {
                let v = read.unwrap_or_else(|e| panic!("{form} {:?}: {e}", case.text));
                assert_eq!(&v.to_string(), canonical, "{form} {:?}", case.text);
                assert_eq!(&fields(&v), expected, "{form} {:?}", case.text);
                assert_eq!(
                    canonical.parse::<T>().as_ref(),
                    Ok(&v),
                    "{form} {canonical:?}"
                );
                let value = v.to_value().expect("writes");
                assert_eq!(<&str>::try_from(&value).ok(), Some(canonical.as_str()));
                assert_eq!(
                    T::from_value(&Value::from(case.text.as_str()))
                        .ok()
                        .as_ref(),
                    Some(&v)
                );
            }
            Err(at) => {
                match read {
                    Err(ValueError::Unparsed { at: got, .. }) => {
                        assert_eq!(got, *at, "{form} {:?} refused at", case.text);
                    }
                    other => panic!("{form} {:?}: expected a refusal, got {other:?}", case.text),
                }
                let e = T::from_value(&Value::from(case.text.as_str()))
                    .expect_err("a value is refused too")
                    .to_string();
                assert!(
                    e.contains(&format!("at byte {at}")),
                    "{form} {:?}: {e}",
                    case.text
                );
            }
        }
    }
}

#[test]
fn dates() {
    each::<Date>("date", |d| {
        vec![d.year().into(), d.month().into(), d.day().into()]
    });
}

#[test]
fn times_of_day() {
    each::<Time>("time", |t| {
        vec![
            t.hour().into(),
            t.minute().into(),
            t.second().into(),
            t.nanosecond().into(),
        ]
    });
}

#[test]
fn civil_dates_and_times() {
    each::<DateTime>("datetime", |at| {
        let (d, t) = (at.date(), at.time());
        vec![
            d.year().into(),
            d.month().into(),
            d.day().into(),
            t.hour().into(),
            t.minute().into(),
            t.second().into(),
            t.nanosecond().into(),
        ]
    });
}

#[test]
fn instants() {
    each::<Timestamp>("timestamp", |t| vec![t.seconds(), t.nanoseconds().into()]);
}

#[test]
fn spans() {
    each::<Span>("span", |s| {
        vec![
            s.get_years(),
            s.get_months(),
            s.get_weeks(),
            s.get_days(),
            s.get_hours(),
            s.get_minutes(),
            s.get_seconds(),
            s.get_nanoseconds(),
        ]
    });
}

/// The fields round trip through the constructors as well.
#[test]
fn fields_rebuild_the_value() {
    let doc = vectors::file();
    for case in vectors::texts(&doc, "date") {
        if let Ok((canonical, f)) = case.expect {
            let d = Date::try_new(f[0] as i32, f[1] as u8, f[2] as u8).unwrap();
            assert_eq!(d.to_string(), canonical);
        }
    }
    for case in vectors::texts(&doc, "time") {
        if let Ok((canonical, f)) = case.expect {
            let t = Time::try_new(f[0] as u8, f[1] as u8, f[2] as u8, f[3] as u32).unwrap();
            assert_eq!(t.to_string(), canonical);
        }
    }
    for case in vectors::texts(&doc, "timestamp") {
        if let Ok((canonical, f)) = case.expect {
            let t = Timestamp::from_unix(f[0], f[1] as u32).unwrap();
            assert_eq!(t.to_string(), canonical);
        }
    }
}

#[test]
fn spans_applied_to_dates() {
    let doc = vectors::file();
    for case in vectors::additions(&doc, "date_add_span") {
        let start: Date = case.start.parse().unwrap();
        let span: Span = case.span.parse().unwrap();
        let expected = case.result.as_deref().map(|r| r.parse::<Date>().unwrap());
        assert_eq!(
            start.checked_add(span),
            expected,
            "{} + {}",
            case.start,
            case.span
        );
    }
}

#[test]
fn spans_applied_to_civil_dates_and_times() {
    let doc = vectors::file();
    for case in vectors::additions(&doc, "datetime_add_span") {
        let start: DateTime = case.start.parse().unwrap();
        let span: Span = case.span.parse().unwrap();
        let expected = case
            .result
            .as_deref()
            .map(|r| r.parse::<DateTime>().unwrap());
        assert_eq!(
            start.checked_add(span),
            expected,
            "{} + {}",
            case.start,
            case.span
        );
    }
}

#[test]
fn exact_lengths() {
    let doc = vectors::file();
    for case in vectors::durations(&doc) {
        let read = Duration::from_value(&vectors::to_value(&case.value));
        match case.expect {
            Some((secs, nanos)) => {
                assert_eq!(
                    read.ok(),
                    Some(Duration::new(secs, nanos)),
                    "{}",
                    case.value
                )
            }
            None => assert!(read.is_err(), "{} is refused", case.value),
        }
    }
}

/// A `SystemTime` crosses as its timestamp, for every instant this
/// platform's clock holds exactly (Windows: from 1601, in 100 ns).
#[test]
fn system_time_is_a_timestamp() {
    let doc = vectors::file();
    let mut checked = 0;
    for case in vectors::texts(&doc, "timestamp") {
        let Ok((canonical, f)) = case.expect else {
            continue;
        };
        let t = Timestamp::from_unix(f[0], f[1] as u32).unwrap();
        let Ok(st) = SystemTime::try_from(t) else {
            continue;
        };
        if Timestamp::try_from(st) != Ok(t) {
            continue;
        }
        let value = st.to_value().unwrap();
        assert_eq!(<&str>::try_from(&value).ok(), Some(canonical.as_str()));
        assert_eq!(SystemTime::from_value(&value).ok(), Some(st));
        checked += 1;
    }
    assert!(checked >= 8, "{checked} instants checked");
}
