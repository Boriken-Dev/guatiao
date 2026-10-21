# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Dates, times, instants, spans and exact lengths of time, as Python's
own types.

Each form is text in a value, fixed on the encodings page: a date
`2026-10-21`, a time of day `13:45:00.5`, a civil date and time
`2026-10-21T13:45:00.5`, an instant `2026-10-21T13:45:00.5Z` and a span
`P1Y2M3W4DT5H6M7.5S`; an exact length is the list `[seconds,
nanoseconds]`. Every text is read and written by the library's own
functions, so this module accepts exactly what any other reader does.

`datetime` counts microseconds and starts at year 1: reading a form into
one drops the nanoseconds below a microsecond, toward the past, and a
form in year 0000 raises `ValueError`. Writing one back is exact.
`normalize` and `Span` keep every nanosecond.

Importing this module touches no native library.
"""

from __future__ import annotations

import ctypes
import datetime as _dt
from typing import Any, Union

from . import _abi, _lib
from .errors import Unparsed, check, raise_for_status
from .value import Ref, Value, _make_str, _string_view

__all__ = [
    "Span",
    "normalize",
    "parse_date",
    "format_date",
    "parse_time",
    "format_time",
    "parse_datetime",
    "format_datetime",
    "parse_timestamp",
    "format_timestamp",
    "to_timedelta",
    "from_timedelta",
]

_UTC = _dt.timezone.utc
_EPOCH = _dt.datetime(1970, 1, 1, tzinfo=_UTC)

#: form -> its fields struct.
_STRUCTS = {
    "date": _abi.DateFields,
    "time": _abi.TimeFields,
    "datetime": _abi.DateTimeFields,
    "timestamp": _abi.TimestampFields,
    "span": _abi.SpanFields,
}


def _struct_of(form: str) -> type:
    try:
        return _STRUCTS[form]
    except KeyError:
        raise ValueError(f"unknown form: {form!r}") from None


def _take_text(raw: "_abi.Value") -> str:
    """A string value's text; the value is freed here."""
    try:
        if raw.tag != _abi.Tag.STRING:
            return ""
        return _string_view(raw.payload.text).decode("utf-8")
    finally:
        _lib.core().guatiao_value_free(ctypes.byref(raw))


def _finish(status: int, error: "_abi.TextError", form: str) -> None:
    """Frees the error's message and raises for a failing `status`: an
    `Unparsed` for text the grammar refused, anything else as its own
    exception."""
    expected = _take_text(error.expected)
    if status == _abi.Status.OK:
        return
    if status == _abi.Status.ERR_BAD_VALUE:
        raise Unparsed(expected or "a valid text", form=form, at=error.at)
    raise_for_status(status, expected)


def _parse(form: str, text: str):
    """`text` read into `form`'s fields struct."""
    struct = _struct_of(form)
    lib = _lib.core()
    view, _buf = _make_str(text.encode("utf-8"))
    out = struct()
    error = _abi.TextError()
    status = getattr(lib, f"guatiao_{form}_parse")(
        ctypes.byref(lib.alloc), view, ctypes.byref(out), ctypes.byref(error)
    )
    _finish(status, error, form)
    return out


def _format(form: str, fields) -> str:
    """`fields` written as `form`'s canonical text."""
    lib = _lib.core()
    out = _abi.Value()
    status = getattr(lib, f"guatiao_{form}_format")(
        ctypes.byref(lib.alloc), ctypes.byref(fields), ctypes.byref(out)
    )
    text = _take_text(out)
    if status == _abi.Status.ERR_BAD_VALUE:
        raise_for_status(status, f"the fields are not a {form}")
    check(status)
    return text


def normalize(form: str, text: str) -> str:
    """`text`, read as `form` (`"date"`, `"time"`, `"datetime"`,
    `"timestamp"` or `"span"`), in its canonical spelling: an instant in
    UTC, a fraction without trailing zeros, a span in ISO 8601. Exact to
    the nanosecond. Raises `Unparsed` for text the grammar refuses."""
    _struct_of(form)
    lib = _lib.core()
    view, _buf = _make_str(text.encode("utf-8"))
    out = _abi.Value()
    error = _abi.TextError()
    status = getattr(lib, f"guatiao_{form}_normalize")(
        ctypes.byref(lib.alloc), view, ctypes.byref(out), ctypes.byref(error)
    )
    result = _take_text(out)
    _finish(status, error, form)
    return result


# ---- dates and times -------------------------------------------------------


def parse_date(text: str) -> _dt.date:
    """`YYYY-MM-DD` as a `datetime.date`. Raises `Unparsed` for text the
    grammar refuses, `ValueError` for the year 0000."""
    f = _parse("date", text)
    return _dt.date(f.year, f.month, f.day)


def format_date(date: _dt.date) -> str:
    """`date` as `YYYY-MM-DD`. A `datetime` is refused: write it with
    `format_datetime` or `format_timestamp`."""
    if isinstance(date, _dt.datetime):
        raise TypeError("a datetime is not a date: use format_datetime or format_timestamp")
    return _format("date", _date_fields(date))


def _date_fields(date: _dt.date) -> "_abi.DateFields":
    return _abi.DateFields(year=date.year, month=date.month, day=date.day)


def _time_fields(t: "_dt.time | _dt.datetime") -> "_abi.TimeFields":
    return _abi.TimeFields(
        hour=t.hour, minute=t.minute, second=t.second, nanosecond=t.microsecond * 1000
    )


def parse_time(text: str) -> _dt.time:
    """`HH:MM:SS[.fraction]` as a `datetime.time` with no `tzinfo`, the
    nanoseconds below a microsecond dropped."""
    f = _parse("time", text)
    return _dt.time(f.hour, f.minute, f.second, f.nanosecond // 1000)


def format_time(time: _dt.time) -> str:
    """`time` as `HH:MM:SS[.fraction]`. A time with a `tzinfo` is refused:
    a time of day has no offset."""
    if time.tzinfo is not None:
        raise ValueError("a time of day has no offset: give a time with no tzinfo")
    return _format("time", _time_fields(time))


def parse_datetime(text: str) -> _dt.datetime:
    """A civil date and time, `YYYY-MM-DDTHH:MM:SS[.fraction]`, as a
    `datetime.datetime` with no `tzinfo`."""
    f = _parse("datetime", text)
    d, t = f.date, f.time
    return _dt.datetime(d.year, d.month, d.day, t.hour, t.minute, t.second, t.nanosecond // 1000)


def format_datetime(datetime: _dt.datetime) -> str:
    """A `datetime` with no `tzinfo` as a civil date and time. One with a
    `tzinfo` is refused: it names an instant, which `format_timestamp`
    writes."""
    if datetime.tzinfo is not None:
        raise ValueError(
            "a civil date and time has no time zone: use format_timestamp for an instant"
        )
    return _format("datetime", _datetime_fields(datetime))


def _datetime_fields(datetime: _dt.datetime) -> "_abi.DateTimeFields":
    return _abi.DateTimeFields(date=_date_fields(datetime), time=_time_fields(datetime))


def parse_timestamp(text: str) -> _dt.datetime:
    """An RFC 3339 instant with any offset as an aware `datetime.datetime`
    in UTC. The offset it was written with is not kept."""
    f = _parse("timestamp", text)
    try:
        return _EPOCH + _dt.timedelta(seconds=f.seconds, microseconds=f.nanoseconds // 1000)
    except OverflowError:
        raise ValueError("an instant before year 1 does not fit a datetime") from None


def format_timestamp(datetime: _dt.datetime) -> str:
    """An aware `datetime` as an instant in UTC, `...Z`. A naive one is
    refused: without a zone it names no instant."""
    if datetime.tzinfo is None or datetime.utcoffset() is None:
        raise ValueError(
            "an instant needs a time zone: give an aware datetime, or use format_datetime"
        )
    since = datetime - _EPOCH
    fields = _abi.TimestampFields(
        seconds=since.days * 86400 + since.seconds, nanoseconds=since.microseconds * 1000
    )
    return _format("timestamp", fields)


# ---- exact lengths ------------------------------------------------------------


def to_timedelta(value: Any) -> _dt.timedelta:
    """An exact length of time as a `datetime.timedelta`.

    `value` is a `Value` or `Ref`, or what `Value(...)` takes: the list
    `[seconds, nanoseconds]` an exact length is written as, a number of
    seconds (an `int`, a `decimal.Decimal`, or a `float`, read from its
    `repr`), or the map `{"secs": ..., "nanos": ...}`. Read by the
    library's own reader, so it refuses (`BadValue`) what any other reader
    refuses. The nanoseconds below a microsecond are dropped; past
    999999999 days raises `OverflowError`."""
    lib = _lib.core()
    secs = ctypes.c_uint64()
    nanos = ctypes.c_uint32()
    if isinstance(value, Ref):
        status = lib.guatiao_duration_read(value._ptr(), ctypes.byref(secs), ctypes.byref(nanos))
    else:
        with Value(value) as built:
            status = lib.guatiao_duration_read(
                ctypes.byref(built._raw), ctypes.byref(secs), ctypes.byref(nanos)
            )
    if status == _abi.Status.ERR_BAD_VALUE:
        raise_for_status(
            status,
            "not an exact length: [seconds, nanoseconds], a number of seconds, "
            "or {secs, nanos}",
        )
    check(status)
    return _dt.timedelta(seconds=secs.value, microseconds=nanos.value // 1000)


def from_timedelta(delta: _dt.timedelta) -> "list[int]":
    """`delta` as `[seconds, nanoseconds]`, the list an exact length is
    written as; `Value(...)` takes it. A negative `timedelta` is refused:
    an exact length is never negative."""
    if delta < _dt.timedelta(0):
        raise ValueError("an exact length of time is never negative")
    return [delta.days * 86400 + delta.seconds, delta.microseconds * 1000]


# ---- spans ---------------------------------------------------------------------

_UNITS = ("years", "months", "weeks", "days", "hours", "minutes", "seconds", "nanoseconds")


class Span:
    """A length of time in calendar units, each counted on its own, with
    one sign for the whole span: `Span(days=1, hours=2)` is `P1DT2H`, and
    `Span(days=-1, hours=-2)` is `-P1DT2H`. Equal unit by unit, so
    `Span(days=1) != Span(hours=24)`: where a calendar applies a span, a
    day is not always 24 hours.

    `str()` is the canonical ISO 8601 text; `Span.parse` also reads the
    friendly form, `1h 30m`. Every field keeps its nanoseconds. Immutable
    and hashable.

    Construction is checked by the library: fields of both signs, a field
    past a signed 64-bit integer, or `nanoseconds` of a second or more
    raise `BadValue`.
    """

    __slots__ = ("_fields", "_text")

    def __init__(
        self,
        *,
        years: int = 0,
        months: int = 0,
        weeks: int = 0,
        days: int = 0,
        hours: int = 0,
        minutes: int = 0,
        seconds: int = 0,
        nanoseconds: int = 0,
    ) -> None:
        values = (years, months, weeks, days, hours, minutes, seconds, nanoseconds)
        for name, n in zip(_UNITS, values):
            if not isinstance(n, int) or isinstance(n, bool):
                raise TypeError(f"{name} must be an int")
        if not all(-(2**63) <= n < 2**63 for n in values[:7]) or not (
            -(2**31) <= nanoseconds < 2**31
        ):
            raise_for_status(int(_abi.Status.ERR_BAD_VALUE), "the fields are not a span")
        fields = _abi.SpanFields(*values)
        object.__setattr__(self, "_text", _format("span", fields))
        object.__setattr__(self, "_fields", values)

    @classmethod
    def parse(cls, text: str) -> "Span":
        """An ISO 8601 duration (`P1Y2M3W4DT5H6M7.5S`, `-P1D`) or the
        friendly form (`1y 2mo 3w 4d 5h 6m 7s 500ms`). Raises `Unparsed`
        for text the grammar refuses."""
        f = _parse("span", text)
        return cls(**{name: getattr(f, name) for name in _UNITS})

    def __getattr__(self, name: str) -> int:
        if name in _UNITS:
            return self._fields[_UNITS.index(name)]
        raise AttributeError(name)

    def __setattr__(self, name: str, value: object) -> None:
        raise AttributeError("a Span is immutable")

    def __str__(self) -> str:
        return self._text

    def __repr__(self) -> str:
        return f"Span({self._text!r})"

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Span):
            return NotImplemented
        return self._fields == other._fields

    def __hash__(self) -> int:
        return hash(self._fields)

    def __neg__(self) -> "Span":
        return Span(**{name: -n for name, n in zip(_UNITS, self._fields)})

    def is_zero(self) -> bool:
        return not any(self._fields)

    def is_negative(self) -> bool:
        return any(n < 0 for n in self._fields)

    def _struct(self) -> "_abi.SpanFields":
        return _abi.SpanFields(*self._fields)

    def add_to(self, when: "Union[_dt.date, _dt.datetime]"):
        """`when` moved by this span, as the library applies one: years and
        months first, a day past the month's end becoming its last day,
        then weeks and days, then (on a `datetime`) the time units carried
        across midnight, a day being 24 hours.

        `when` is a `datetime.date`, or a `datetime.datetime` with no
        `tzinfo`; the answer is the same type. Raises `BadValue` for a span
        with time units on a date, or a result outside the years 1 to
        9999 `datetime` holds."""
        lib = _lib.core()
        span = self._struct()
        if isinstance(when, _dt.datetime):
            if when.tzinfo is not None:
                raise ValueError("a span moves a civil date and time: give one with no tzinfo")
            out = _abi.DateTimeFields()
            check(
                lib.guatiao_datetime_add_span(
                    ctypes.byref(_datetime_fields(when)), ctypes.byref(span), ctypes.byref(out)
                )
            )
            d, t = out.date, out.time
            return _dt.datetime(
                d.year, d.month, d.day, t.hour, t.minute, t.second, t.nanosecond // 1000
            )
        if isinstance(when, _dt.date):
            out_date = _abi.DateFields()
            check(
                lib.guatiao_date_add_span(
                    ctypes.byref(_date_fields(when)), ctypes.byref(span), ctypes.byref(out_date)
                )
            )
            return _dt.date(out_date.year, out_date.month, out_date.day)
        raise TypeError("a span moves a datetime.date or a datetime.datetime")
