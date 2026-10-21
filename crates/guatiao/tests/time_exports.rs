// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The date, time, span and length exports against
//! `tests/vectors/time.json`, called the way a foreign caller calls them:
//! raw pointers, statuses, and a refusal coming back as an offset and a
//! message.

use guatiao::exports::span::*;
use guatiao::exports::time::*;
use guatiao::value::alloc::Allocator;
use guatiao::value::status::Status;
use guatiao::value::types::{Str, Tag, Value};
use guatiao::{Alloc, Map};

#[path = "vectors/read.rs"]
mod vectors;

type Parse<F> = unsafe extern "C" fn(*const Allocator, Str, *mut F, *mut TextError) -> Status;
type Format<F> = unsafe extern "C" fn(*const Allocator, *const F, *mut Value) -> Status;
type Normalize = unsafe extern "C" fn(*const Allocator, Str, *mut Value, *mut TextError) -> Status;

fn text_of(v: &Value) -> &str {
    <&str>::try_from(v).expect("a string")
}

/// Every vector of `form` through its three exports: an accepted text
/// parses to `fields`, formats and normalises to its canonical spelling;
/// a refused one is refused at its offset with a message that does not
/// quote it, through both readers.
fn each<F: Copy + Default>(
    form: &str,
    parse: Parse<F>,
    format: Format<F>,
    normalize: Normalize,
    fields: impl Fn(&F) -> Vec<i64>,
) {
    let alloc = Alloc::rust();
    let doc = vectors::file();
    for case in vectors::texts(&doc, form) {
        let text = Str::new(&case.text);
        let mut parsed = F::default();
        let mut error = TextError {
            at: 0,
            expected: Value::absent(),
        };
        // SAFETY: a readable view, writable outs, a live allocator.
        let status = unsafe { parse(alloc.as_raw(), text, &mut parsed, &mut error) };
        let mut normal = Value::absent();
        let mut normal_error = TextError {
            at: 0,
            expected: Value::absent(),
        };
        // SAFETY: as above.
        let n_status = unsafe { normalize(alloc.as_raw(), text, &mut normal, &mut normal_error) };
        match &case.expect {
            Ok((canonical, expected)) => {
                assert_eq!(status, Status::GUATIAO_OK, "{form} {:?}", case.text);
                assert_eq!(&fields(&parsed), expected, "{form} {:?}", case.text);
                assert!(error.expected.tag() == Ok(Tag::GUATIAO_ABSENT));
                let mut written = Value::absent();
                // SAFETY: a readable struct, a writable out.
                let f_status = unsafe { format(alloc.as_raw(), &parsed, &mut written) };
                assert_eq!(f_status, Status::GUATIAO_OK, "{form} {:?}", case.text);
                assert_eq!(text_of(&written), canonical, "{form} format");
                assert_eq!(n_status, Status::GUATIAO_OK, "{form} {:?}", case.text);
                assert_eq!(text_of(&normal), canonical, "{form} normalize");
            }
            Err(at) => {
                for (status, error) in [(status, &error), (n_status, &normal_error)] {
                    assert_eq!(
                        status,
                        Status::GUATIAO_ERR_BAD_VALUE,
                        "{form} {:?}",
                        case.text
                    );
                    assert_eq!(error.at, *at, "{form} {:?}", case.text);
                    let expected = text_of(&error.expected);
                    assert!(!expected.is_empty());
                    if case.text.len() > 2 {
                        assert!(!expected.contains(case.text.as_str()), "{expected}");
                    }
                }
                assert!(normal.tag() == Ok(Tag::GUATIAO_ABSENT));
            }
        }
    }
}

#[test]
fn dates() {
    each::<DateFields>(
        "date",
        guatiao_date_parse,
        guatiao_date_format,
        guatiao_date_normalize,
        |d| vec![d.year.into(), d.month.into(), d.day.into()],
    );
}

#[test]
fn times_of_day() {
    each::<TimeFields>(
        "time",
        guatiao_time_parse,
        guatiao_time_format,
        guatiao_time_normalize,
        |t| {
            vec![
                t.hour.into(),
                t.minute.into(),
                t.second.into(),
                t.nanosecond.into(),
            ]
        },
    );
}

#[test]
fn civil_dates_and_times() {
    each::<DateTimeFields>(
        "datetime",
        guatiao_datetime_parse,
        guatiao_datetime_format,
        guatiao_datetime_normalize,
        |at| {
            vec![
                at.date.year.into(),
                at.date.month.into(),
                at.date.day.into(),
                at.time.hour.into(),
                at.time.minute.into(),
                at.time.second.into(),
                at.time.nanosecond.into(),
            ]
        },
    );
}

#[test]
fn instants() {
    each::<TimestampFields>(
        "timestamp",
        guatiao_timestamp_parse,
        guatiao_timestamp_format,
        guatiao_timestamp_normalize,
        |t| vec![t.seconds, t.nanoseconds.into()],
    );
}

#[test]
fn spans() {
    each::<SpanFields>(
        "span",
        guatiao_span_parse,
        guatiao_span_format,
        guatiao_span_normalize,
        |s| {
            vec![
                s.years,
                s.months,
                s.weeks,
                s.days,
                s.hours,
                s.minutes,
                s.seconds,
                s.nanoseconds.into(),
            ]
        },
    );
}

/// Parses `text` with `parse`, which must accept it.
fn parsed<F: Default>(parse: Parse<F>, text: &str) -> F {
    let mut out = F::default();
    // SAFETY: a readable view, a writable out, no error slot.
    let status = unsafe {
        parse(
            core::ptr::null(),
            Str::new(text),
            &mut out,
            core::ptr::null_mut(),
        )
    };
    assert_eq!(status, Status::GUATIAO_OK, "{text}");
    out
}

#[test]
fn spans_applied_to_dates_and_date_times() {
    let doc = vectors::file();
    for case in vectors::additions(&doc, "date_add_span") {
        let start = parsed(guatiao_date_parse, &case.start);
        let span = parsed(guatiao_span_parse, &case.span);
        let mut out = DateFields::default();
        // SAFETY: readable inputs, a writable out.
        let status = unsafe { guatiao_date_add_span(&start, &span, &mut out) };
        match &case.result {
            Some(r) => {
                assert_eq!(status, Status::GUATIAO_OK, "{} + {}", case.start, case.span);
                assert_eq!(out, parsed(guatiao_date_parse, r));
            }
            None => assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{}", case.span),
        }
    }
    for case in vectors::additions(&doc, "datetime_add_span") {
        let mut at = parsed(guatiao_datetime_parse, &case.start);
        let span = parsed(guatiao_span_parse, &case.span);
        let start = at;
        // SAFETY: readable inputs, a writable out.
        let status = unsafe { guatiao_datetime_add_span(&start, &span, &mut at) };
        match &case.result {
            Some(r) => {
                assert_eq!(status, Status::GUATIAO_OK, "{} + {}", case.start, case.span);
                assert_eq!(at, parsed(guatiao_datetime_parse, r));
            }
            None => assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{}", case.span),
        }
    }
}

#[test]
fn a_date_added_to_in_place_reads_before_it_writes() {
    let mut date = parsed(guatiao_date_parse, "2026-01-31");
    let span = parsed(guatiao_span_parse, "P1M");
    let p: *mut DateFields = &mut date;
    // SAFETY: `date` and `out` are the same struct, which the export allows.
    let status = unsafe { guatiao_date_add_span(p, &span, p) };
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(date, parsed(guatiao_date_parse, "2026-02-28"));
}

#[test]
fn exact_lengths() {
    let doc = vectors::file();
    for case in vectors::durations(&doc) {
        let value = vectors::to_value(&case.value);
        let (mut secs, mut nanos) = (7u64, 7u32);
        // SAFETY: a well-formed value, writable outs.
        let status = unsafe { guatiao_duration_read(&value, &mut secs, &mut nanos) };
        match case.expect {
            Some(expected) => {
                assert_eq!(status, Status::GUATIAO_OK, "{}", case.value);
                assert_eq!((secs, nanos), expected, "{}", case.value);
            }
            None => {
                assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE, "{}", case.value);
                assert_eq!((secs, nanos), (0, 0));
            }
        }
    }
}

#[test]
fn a_struct_that_is_no_form_is_refused_when_written() {
    let alloc = Alloc::rust();
    let write_span = |s: SpanFields| {
        let mut out = Value::absent();
        // SAFETY: a readable struct, a writable out.
        let status = unsafe { guatiao_span_format(alloc.as_raw(), &s, &mut out) };
        (status, out)
    };
    let mixed = SpanFields {
        days: 1,
        hours: -2,
        ..SpanFields::default()
    };
    assert_eq!(write_span(mixed).0, Status::GUATIAO_ERR_BAD_VALUE);
    let min = SpanFields {
        days: i64::MIN,
        ..SpanFields::default()
    };
    assert_eq!(write_span(min).0, Status::GUATIAO_ERR_BAD_VALUE);
    let carry = SpanFields {
        nanoseconds: 1_000_000_000,
        ..SpanFields::default()
    };
    assert_eq!(write_span(carry).0, Status::GUATIAO_ERR_BAD_VALUE);
    let negative = SpanFields {
        days: -1,
        hours: -2,
        nanoseconds: -500_000_000,
        ..SpanFields::default()
    };
    let (status, out) = write_span(negative);
    assert_eq!(status, Status::GUATIAO_OK);
    assert_eq!(text_of(&out), "-P1DT2H0.5S");

    let mut out = Value::absent();
    let feb_30 = DateFields {
        year: 2026,
        month: 2,
        day: 30,
    };
    // SAFETY: as above.
    let status = unsafe { guatiao_date_format(alloc.as_raw(), &feb_30, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(out.tag() == Ok(Tag::GUATIAO_ABSENT));
    let leap = TimeFields {
        hour: 23,
        minute: 59,
        second: 60,
        nanosecond: 0,
    };
    // SAFETY: as above.
    let status = unsafe { guatiao_time_format(alloc.as_raw(), &leap, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    let past = TimestampFields {
        seconds: 253_402_300_800,
        nanoseconds: 0,
    };
    // SAFETY: as above.
    let status = unsafe { guatiao_timestamp_format(alloc.as_raw(), &past, &mut out) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
}

#[test]
fn text_that_is_not_utf8_is_refused_at_its_first_bad_byte() {
    static BYTES: [u8; 6] = [b'2', b'0', b'2', 0xff, b'-', b'1'];
    #[repr(C)]
    struct AsC {
        ptr: *const u8,
        len: usize,
    }
    // SAFETY: `Str` is `repr(C)` with exactly these two fields.
    let text = unsafe {
        core::mem::transmute::<AsC, Str<'static>>(AsC {
            ptr: BYTES.as_ptr(),
            len: BYTES.len(),
        })
    };
    let mut out = DateFields::default();
    let mut error = TextError {
        at: 0,
        expected: Value::absent(),
    };
    // SAFETY: a readable view, writable outs.
    let status = unsafe { guatiao_date_parse(Alloc::rust().as_raw(), text, &mut out, &mut error) };
    assert_eq!(status, Status::GUATIAO_ERR_BAD_VALUE);
    assert_eq!(error.at, 3);
    assert_eq!(text_of(&error.expected), "UTF-8 text");
}

#[test]
fn a_null_is_refused_rather_than_dereferenced() {
    let span = SpanFields::default();
    let date = DateFields::default();
    let mut out = DateFields::default();
    // SAFETY: null is the case under test.
    unsafe {
        assert_eq!(
            guatiao_date_parse(
                core::ptr::null(),
                Str::new("2026-10-21"),
                core::ptr::null_mut(),
                core::ptr::null_mut()
            ),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_date_add_span(core::ptr::null(), &span, &mut out),
            Status::GUATIAO_ERR_NULL
        );
        assert_eq!(
            guatiao_date_add_span(&date, core::ptr::null(), &mut out),
            Status::GUATIAO_ERR_NULL
        );
        let mut v = Value::absent();
        assert_eq!(
            guatiao_span_format(Alloc::rust().as_raw(), core::ptr::null(), &mut v),
            Status::GUATIAO_ERR_NULL
        );
        let (mut s, mut n) = (0u64, 0u32);
        assert_eq!(
            guatiao_duration_read(core::ptr::null(), &mut s, &mut n),
            Status::GUATIAO_ERR_NULL
        );
        let map = Value::from(Map::new());
        assert_eq!(
            guatiao_duration_read(&map, core::ptr::null_mut(), &mut n),
            Status::GUATIAO_ERR_NULL
        );
    }
}
