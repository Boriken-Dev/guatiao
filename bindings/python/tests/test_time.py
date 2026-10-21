# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.time` against the shared vectors every implementation reads,
then the mappings to Python's own types and what they lose."""

from __future__ import annotations

import datetime as dt
import decimal
import json
from pathlib import Path

import pytest

from guatiao import BadValue, Unparsed, Value
from guatiao import time as gt

_VECTORS = json.loads(
    (
        Path(__file__).resolve().parents[3]
        / "crates"
        / "guatiao"
        / "tests"
        / "vectors"
        / "time.json"
    ).read_text(encoding="utf-8"),
    parse_float=decimal.Decimal,
)

FORMS = ["date", "time", "datetime", "timestamp", "span"]


def _fields(form, struct):
    if form == "datetime":
        return _fields("date", struct.date) + _fields("time", struct.time)
    return [getattr(struct, name) for name in _VECTORS["fields"][form]]


@pytest.mark.parametrize("form", FORMS)
def test_every_vector(form):
    cases = _VECTORS[form]
    assert cases
    for case in cases:
        text = case["text"]
        if "refused_at" in case:
            for read in (lambda: gt._parse(form, text), lambda: gt.normalize(form, text)):
                with pytest.raises(Unparsed) as excinfo:
                    read()
                assert excinfo.value.at == case["refused_at"], text
                assert excinfo.value.form == form
                assert excinfo.value.expected
                if len(text) > 2:
                    assert text not in str(excinfo.value)
        else:
            struct = gt._parse(form, text)
            assert _fields(form, struct) == case["fields"], text
            assert gt._format(form, struct) == case["canonical"], text
            assert gt.normalize(form, text) == case["canonical"], text


@pytest.mark.parametrize("form", ["date_add_span", "datetime_add_span"])
def test_every_span_applied(form):
    kind = form.split("_")[0]
    for case in _VECTORS[form]:
        start = gt._parse(kind, case["start"])
        span = gt._parse("span", case["span"])
        out = type(start)()
        lib = gt._lib.core()
        status = getattr(lib, f"guatiao_{kind}_add_span")(
            gt.ctypes.byref(start), gt.ctypes.byref(span), gt.ctypes.byref(out)
        )
        if case["result"] is None:
            assert status == gt._abi.Status.ERR_BAD_VALUE, case
        else:
            assert status == gt._abi.Status.OK, case
            assert gt._format(kind, out) == case["result"], case


def test_every_exact_length():
    for case in _VECTORS["duration"]:
        expected = case["duration"]
        if expected is None:
            with pytest.raises(BadValue):
                gt.to_timedelta(case["value"])
            continue
        secs, nanos = expected
        if secs > 999_999_999 * 86400:
            with pytest.raises(OverflowError):
                gt.to_timedelta(case["value"])
            continue
        assert gt.to_timedelta(case["value"]) == dt.timedelta(
            seconds=secs, microseconds=nanos // 1000
        ), case


# ---- the mappings ------------------------------------------------------------


def test_a_date_is_a_date():
    assert gt.parse_date("2024-02-29") == dt.date(2024, 2, 29)
    assert gt.format_date(dt.date(2026, 10, 21)) == "2026-10-21"
    assert gt.format_date(dt.date(1, 1, 1)) == "0001-01-01"
    with pytest.raises(ValueError):
        gt.parse_date("0000-01-01")  # Python's dates start at year 1
    with pytest.raises(TypeError):
        gt.format_date(dt.datetime(2026, 10, 21))


def test_a_time_of_day_loses_what_is_below_a_microsecond():
    assert gt.parse_time("13:45:00.5") == dt.time(13, 45, 0, 500000)
    assert gt.parse_time("12:00:00.000000999") == dt.time(12, 0, 0, 0)
    assert gt.format_time(dt.time(8, 5, 3, 120000)) == "08:05:03.12"
    with pytest.raises(ValueError):
        gt.format_time(dt.time(1, 0, tzinfo=dt.timezone.utc))


def test_a_civil_date_and_time_is_naive():
    at = gt.parse_datetime("2026-10-21T13:45:00.5")
    assert at == dt.datetime(2026, 10, 21, 13, 45, 0, 500000)
    assert at.tzinfo is None
    assert gt.format_datetime(at) == "2026-10-21T13:45:00.5"
    with pytest.raises(ValueError):
        gt.format_datetime(dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc))


def test_an_instant_is_aware_and_in_utc():
    t = gt.parse_timestamp("2026-10-21T19:15:00.5+05:30")
    assert t == dt.datetime(2026, 10, 21, 13, 45, 0, 500000, tzinfo=dt.timezone.utc)
    assert t.utcoffset() == dt.timedelta(0)
    assert gt.parse_timestamp("1969-12-31T23:59:59.9999999Z") == dt.datetime(
        1969, 12, 31, 23, 59, 59, 999999, tzinfo=dt.timezone.utc
    ), "toward the past"
    plus_two = dt.timezone(dt.timedelta(hours=2))
    assert gt.format_timestamp(dt.datetime(2026, 10, 21, 15, 45, tzinfo=plus_two)) == (
        "2026-10-21T13:45:00Z"
    )
    assert gt.format_timestamp(dt.datetime(1, 1, 1, tzinfo=dt.timezone.utc)) == (
        "0001-01-01T00:00:00Z"
    )
    with pytest.raises(ValueError):
        gt.format_timestamp(dt.datetime(2026, 10, 21))
    with pytest.raises(ValueError):
        gt.parse_timestamp("0000-01-01T00:00:00Z")


def test_an_exact_length_is_a_timedelta():
    assert gt.to_timedelta([90, 500_000_000]) == dt.timedelta(seconds=90.5)
    assert gt.to_timedelta(1.5) == dt.timedelta(seconds=1.5)
    assert gt.to_timedelta(decimal.Decimal("2e-3")) == dt.timedelta(milliseconds=2)
    assert gt.to_timedelta({"secs": 1, "nanos": 999}) == dt.timedelta(seconds=1)
    with Value({"d": [3, 0]}) as v:
        assert gt.to_timedelta(v.as_map()["d"]) == dt.timedelta(seconds=3)
    assert gt.from_timedelta(dt.timedelta(seconds=90.5)) == [90, 500_000_000]
    assert gt.from_timedelta(dt.timedelta(days=1, microseconds=1)) == [86400, 1000]
    with pytest.raises(ValueError):
        gt.from_timedelta(dt.timedelta(seconds=-1))
    with pytest.raises(BadValue):
        gt.to_timedelta("90")


def test_a_span_is_its_own_class_backed_by_the_library():
    span = gt.Span.parse("1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns")
    assert str(span) == "P1Y2M3W4DT5H6M7.500001001S"
    assert span.nanoseconds == 500_001_001
    assert span == gt.Span.parse("P1Y2M3W4DT5H6M7.500001001S")
    assert gt.Span(days=1) != gt.Span(hours=24)
    assert gt.Span(days=-1, hours=-2) == gt.Span.parse("-P1DT2H")
    assert str(-gt.Span(days=1)) == "-P1D"
    assert repr(gt.Span(minutes=90)) == "Span('PT90M')"
    assert gt.Span().is_zero() and str(gt.Span()) == "PT0S"
    assert gt.Span(days=-1).is_negative()
    assert len({gt.Span(days=1), gt.Span.parse("P1D")}) == 1
    with pytest.raises(BadValue):
        gt.Span(days=1, hours=-1)
    with pytest.raises(BadValue):
        gt.Span(nanoseconds=1_000_000_000)
    with pytest.raises(BadValue):
        gt.Span(days=2**63)
    with pytest.raises(TypeError):
        gt.Span(days=1.5)
    with pytest.raises(AttributeError):
        span.days = 2
    with pytest.raises(Unparsed) as excinfo:
        gt.Span.parse("1h 1h")
    assert excinfo.value.at == 4


def test_a_span_moves_a_date_and_a_civil_date_and_time():
    month = gt.Span.parse("P1M")
    assert month.add_to(dt.date(2026, 1, 31)) == dt.date(2026, 2, 28)
    assert gt.Span(hours=1).add_to(dt.datetime(2026, 10, 21, 23, 30)) == dt.datetime(
        2026, 10, 22, 0, 30
    )
    with pytest.raises(BadValue):
        gt.Span(hours=1).add_to(dt.date(2026, 10, 21))
    with pytest.raises(ValueError):
        month.add_to(dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc))
    with pytest.raises(TypeError):
        month.add_to("2026-01-31")


def test_normalize_keeps_every_nanosecond():
    assert gt.normalize("timestamp", "2026-10-21T19:15:00.000000001+05:30") == (
        "2026-10-21T13:45:00.000000001Z"
    )
    with pytest.raises(ValueError):
        gt.normalize("week", "x")
