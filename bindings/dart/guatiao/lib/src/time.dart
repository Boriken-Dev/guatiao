// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'value.dart';

/// The text forms the library reads and writes.
enum TextForm { date, time, datetime, timestamp, span }

/// A date, time, instant or span whose text its grammar refused.
///
/// A [BadValue] and a [FormatException]. [form] is the form being read,
/// [at] the byte offset in the UTF-8 text where reading stopped, and
/// [expected] what would have been accepted there. Nothing quotes the
/// text, so [source] and [offset] are `null`.
class Unparsed extends BadValue implements FormatException {
  Unparsed(this.expected, {required this.form, required this.at})
      : super(Status.badValue.code, '$expected at byte $at');

  final TextForm form;
  final int at;
  final String expected;

  @override
  dynamic get source => null;

  @override
  int? get offset => null;

  @override
  String toString() => 'Unparsed (${form.name}): $expected at byte $at';
}

/// `guatiao_text_error.expected`, the last field: it starts where the
/// struct's size less a value's does.
Pointer<guatiao_value> _expectedOf(Pointer<guatiao_text_error> error) =>
    (error.cast<Uint8>() +
            (sizeOf<guatiao_text_error>() - sizeOf<guatiao_value>()))
        .cast<guatiao_value>();

/// Runs a call that may refuse text, then throws [Unparsed] for a
/// refusal; the message the library built is freed either way.
void _read(TextForm form, int Function(Pointer<guatiao_text_error>) call) {
  final error = calloc<guatiao_text_error>();
  try {
    final status = call(error);
    final message = _expectedOf(error);
    final expected = Ref.borrowed(message).stringOr('');
    native.core().bindings.guatiao_value_free(message);
    if (status == Status.ok.code) return;
    if (status == Status.badValue.code) {
      throw Unparsed(
        expected.isEmpty ? 'a valid text' : expected,
        form: form,
        at: error.ref.at,
      );
    }
    checkStatus(status, expected);
  } finally {
    calloc.free(error);
  }
}

/// Runs a `_format` call and answers its text.
String _write(
  TextForm form,
  int Function(Pointer<guatiao_alloc>, Pointer<guatiao_value>) call,
) {
  final lib = native.core();
  final out = calloc<guatiao_value>();
  final status = call(lib.alloc, out);
  final text = takeValue(out);
  if (status == Status.badValue.code) {
    throw BadValue(status, 'the fields are not a ${form.name}');
  }
  checkStatus(status);
  return text! as String;
}

/// Throws [BadValue] unless every field fits its C width.
void _fits(TextForm form, List<(int, int, bool)> fields) {
  for (final (value, bits, signed) in fields) {
    final ok = signed
        ? (bits >= 64 ||
            (value >= -(1 << (bits - 1)) && value < (1 << (bits - 1))))
        : (value >= 0 && value < (1 << bits));
    if (!ok) {
      throw BadValue(Status.badValue.code, 'the fields are not a ${form.name}');
    }
  }
}

/// [text] read as [form] and rewritten in its canonical spelling: an
/// instant in UTC, a fraction without trailing zeros, a span in ISO 8601.
/// Exact to the nanosecond. Throws [Unparsed] for text the grammar
/// refuses.
String normalize(TextForm form, String text) {
  final lib = native.core();
  final b = lib.bindings;
  final out = calloc<guatiao_value>();
  try {
    using((arena) {
      final view = strOf(arena, text).ref;
      _read(
        form,
        (error) => switch (form) {
          TextForm.date =>
            b.guatiao_date_normalize(lib.alloc, view, out, error),
          TextForm.time =>
            b.guatiao_time_normalize(lib.alloc, view, out, error),
          TextForm.datetime =>
            b.guatiao_datetime_normalize(lib.alloc, view, out, error),
          TextForm.timestamp =>
            b.guatiao_timestamp_normalize(lib.alloc, view, out, error),
          TextForm.span =>
            b.guatiao_span_normalize(lib.alloc, view, out, error),
        },
      );
    });
    return Ref.borrowed(out).stringOr('');
  } finally {
    b.guatiao_value_free(out);
    calloc.free(out);
  }
}

// ---- a date ------------------------------------------------------------------

/// A day on the calendar with no time and no zone, `2026-10-21`: years 0
/// to 9999. Dart's `DateTime` always carries a time and a zone, so a date
/// alone is its own type, named after JavaScript's Temporal.
///
/// The constructor is checked by the library: a day the month does not
/// have throws [BadValue]. Ordered by the calendar.
final class PlainDate implements Comparable<PlainDate> {
  factory PlainDate(int year, int month, int day) {
    _fits(
        TextForm.date, [(year, 32, true), (month, 8, false), (day, 8, false)]);
    final text = using((arena) {
      final f = arena<guatiao_date>();
      f.ref
        ..year = year
        ..month = month
        ..day = day;
      return _write(
        TextForm.date,
        (alloc, out) =>
            native.core().bindings.guatiao_date_format(alloc, f, out),
      );
    });
    return PlainDate._(year, month, day, text);
  }

  PlainDate._(this.year, this.month, this.day, this._text);

  /// The date [moment] shows, in whichever zone it is in.
  factory PlainDate.fromDateTime(DateTime moment) =>
      PlainDate(moment.year, moment.month, moment.day);

  /// `YYYY-MM-DD`. Throws [Unparsed] for text the grammar refuses.
  static PlainDate parse(String text) {
    final lib = native.core();
    return using((arena) {
      final view = strOf(arena, text).ref;
      final out = arena<guatiao_date>();
      _read(
        TextForm.date,
        (error) => lib.bindings.guatiao_date_parse(lib.alloc, view, out, error),
      );
      return PlainDate(out.ref.year, out.ref.month, out.ref.day);
    });
  }

  final int year;
  final int month;
  final int day;
  final String _text;

  /// This date moved by [span]: years and months first, a day past the
  /// month's end becoming its last day, then weeks and days. Throws
  /// [BadValue] for a span with time units or a result outside the
  /// years 0000 to 9999.
  PlainDate add(Span span) => using((arena) {
        final f = arena<guatiao_date>();
        f.ref
          ..year = year
          ..month = month
          ..day = day;
        final out = arena<guatiao_date>();
        checkStatus(
          native
              .core()
              .bindings
              .guatiao_date_add_span(f, span._struct(arena), out),
        );
        return PlainDate(out.ref.year, out.ref.month, out.ref.day);
      });

  @override
  int compareTo(PlainDate other) => year != other.year
      ? year.compareTo(other.year)
      : month != other.month
          ? month.compareTo(other.month)
          : day.compareTo(other.day);

  @override
  bool operator ==(Object other) =>
      other is PlainDate &&
      other.year == year &&
      other.month == month &&
      other.day == day;

  @override
  int get hashCode => Object.hash(year, month, day);

  /// The canonical text, `YYYY-MM-DD`.
  @override
  String toString() => _text;
}

// ---- a time of day -------------------------------------------------------------

/// A time of day to the nanosecond with no date and no zone,
/// `13:45:00.5`. Checked by the library; ordered by the clock.
final class PlainTime implements Comparable<PlainTime> {
  factory PlainTime(int hour, int minute, int second, [int nanosecond = 0]) {
    _fits(TextForm.time, [
      (hour, 8, false),
      (minute, 8, false),
      (second, 8, false),
      (nanosecond, 32, false),
    ]);
    final text = using((arena) {
      final f = _timeStruct(arena, hour, minute, second, nanosecond);
      return _write(
        TextForm.time,
        (alloc, out) =>
            native.core().bindings.guatiao_time_format(alloc, f, out),
      );
    });
    return PlainTime._(hour, minute, second, nanosecond, text);
  }

  PlainTime._(this.hour, this.minute, this.second, this.nanosecond, this._text);

  /// The time of day [moment] shows, to its microsecond.
  factory PlainTime.fromDateTime(DateTime moment) => PlainTime(
        moment.hour,
        moment.minute,
        moment.second,
        moment.millisecond * 1000000 + moment.microsecond * 1000,
      );

  /// `HH:MM:SS[.fraction]`, no offset. Throws [Unparsed] for text the
  /// grammar refuses.
  static PlainTime parse(String text) {
    final lib = native.core();
    return using((arena) {
      final view = strOf(arena, text).ref;
      final out = arena<guatiao_time>();
      _read(
        TextForm.time,
        (error) => lib.bindings.guatiao_time_parse(lib.alloc, view, out, error),
      );
      final t = out.ref;
      return PlainTime(t.hour, t.minute, t.second, t.nanosecond);
    });
  }

  final int hour;
  final int minute;
  final int second;
  final int nanosecond;
  final String _text;

  @override
  int compareTo(PlainTime other) {
    for (final (a, b) in [
      (hour, other.hour),
      (minute, other.minute),
      (second, other.second),
      (nanosecond, other.nanosecond),
    ]) {
      if (a != b) return a.compareTo(b);
    }
    return 0;
  }

  @override
  bool operator ==(Object other) => other is PlainTime && compareTo(other) == 0;

  @override
  int get hashCode => Object.hash(hour, minute, second, nanosecond);

  /// The canonical text, `HH:MM:SS` and the fraction when there is one.
  @override
  String toString() => _text;
}

Pointer<guatiao_time> _timeStruct(
  Arena arena,
  int hour,
  int minute,
  int second,
  int nanosecond,
) {
  final f = arena<guatiao_time>();
  f.ref
    ..hour = hour
    ..minute = minute
    ..second = second
    ..nanosecond = nanosecond;
  return f;
}

// ---- a civil date and time -------------------------------------------------------

/// A date and a time of day as a wall clock shows them, with no zone:
/// `2026-10-21T13:45:00.5`. Dart's `DateTime` would read one in UTC or the
/// machine's zone, and a local one moves a time a daylight-saving change
/// skips; this keeps the fields as written, to the nanosecond.
final class PlainDateTime implements Comparable<PlainDateTime> {
  factory PlainDateTime(PlainDate date, PlainTime time) {
    final text = using((arena) {
      final f = _struct(arena, date, time);
      return _write(
        TextForm.datetime,
        (alloc, out) =>
            native.core().bindings.guatiao_datetime_format(alloc, f, out),
      );
    });
    return PlainDateTime._(date, time, text);
  }

  PlainDateTime._(this.date, this.time, this._text);

  /// The date and time [moment] shows, to its microsecond.
  factory PlainDateTime.fromDateTime(DateTime moment) => PlainDateTime(
        PlainDate.fromDateTime(moment),
        PlainTime.fromDateTime(moment),
      );

  /// The date, `T` or `t`, the time, no offset. Throws [Unparsed] for
  /// text the grammar refuses.
  static PlainDateTime parse(String text) {
    final lib = native.core();
    return using((arena) {
      final view = strOf(arena, text).ref;
      final out = arena<guatiao_datetime>();
      _read(
        TextForm.datetime,
        (error) =>
            lib.bindings.guatiao_datetime_parse(lib.alloc, view, out, error),
      );
      return _of(out.ref);
    });
  }

  static PlainDateTime _of(guatiao_datetime f) => PlainDateTime(
        PlainDate(f.date.year, f.date.month, f.date.day),
        PlainTime(f.time.hour, f.time.minute, f.time.second, f.time.nanosecond),
      );

  static Pointer<guatiao_datetime> _struct(
    Arena arena,
    PlainDate date,
    PlainTime time,
  ) {
    final f = arena<guatiao_datetime>();
    f.ref.date
      ..year = date.year
      ..month = date.month
      ..day = date.day;
    f.ref.time
      ..hour = time.hour
      ..minute = time.minute
      ..second = time.second
      ..nanosecond = time.nanosecond;
    return f;
  }

  final PlainDate date;
  final PlainTime time;
  final String _text;

  /// This date and time moved by [span]: as [PlainDate.add], then hours
  /// to nanoseconds carried across midnight, a day being 24 hours.
  /// Throws [BadValue] for a result outside the years 0000 to 9999.
  PlainDateTime add(Span span) => using((arena) {
        final out = arena<guatiao_datetime>();
        checkStatus(
          native.core().bindings.guatiao_datetime_add_span(
                _struct(arena, date, time),
                span._struct(arena),
                out,
              ),
        );
        return _of(out.ref);
      });

  @override
  int compareTo(PlainDateTime other) {
    final byDate = date.compareTo(other.date);
    return byDate != 0 ? byDate : time.compareTo(other.time);
  }

  @override
  bool operator ==(Object other) =>
      other is PlainDateTime && other.date == date && other.time == time;

  @override
  int get hashCode => Object.hash(date, time);

  /// The canonical text: the date, `T`, the time.
  @override
  String toString() => _text;
}

// ---- an instant ---------------------------------------------------------------------

/// An RFC 3339 instant with any offset as a UTC [DateTime]. The offset it
/// was written with is not kept, and the nanoseconds below a microsecond
/// are dropped, toward the past. Throws [Unparsed] for text the grammar
/// refuses.
DateTime parseTimestamp(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_timestamp>();
    _read(
      TextForm.timestamp,
      (error) =>
          lib.bindings.guatiao_timestamp_parse(lib.alloc, view, out, error),
    );
    return DateTime.fromMicrosecondsSinceEpoch(
      out.ref.seconds * 1000000 + out.ref.nanoseconds ~/ 1000,
      isUtc: true,
    );
  });
}

/// [moment] as an instant in UTC, `...Z`, whichever zone it is in.
String formatTimestamp(DateTime moment) {
  final micros = moment.microsecondsSinceEpoch;
  final below = micros % 1000000;
  return using((arena) {
    final f = arena<guatiao_timestamp>();
    f.ref
      ..seconds = (micros - below) ~/ 1000000
      ..nanoseconds = below * 1000;
    return _write(
      TextForm.timestamp,
      (alloc, out) =>
          native.core().bindings.guatiao_timestamp_format(alloc, f, out),
    );
  });
}

// ---- an exact length ------------------------------------------------------------------

/// The largest number of whole seconds a [Duration] holds.
const int _maxDurationSeconds = 9223372036854;

/// An exact length of time as a [Duration].
///
/// [value] is a [Ref] or [Value], or what [Value.fromDart] takes: the
/// list `[seconds, nanoseconds]` an exact length is written as, a number
/// of seconds, or the map `{"secs": .., "nanos": ..}`. Read by the
/// library's own reader. The nanoseconds below a microsecond are
/// dropped; a length past what a [Duration] holds throws [RangeError],
/// anything that is no exact length [BadValue].
Duration toDuration(Object? value) {
  final secs = calloc<Uint64>();
  final nanos = calloc<Uint32>();
  try {
    int read(Pointer<guatiao_value> node) =>
        native.core().bindings.guatiao_duration_read(node, secs, nanos);
    int readBuilt() {
      final built = Value.fromDart(value);
      try {
        return read(built.pointer);
      } finally {
        built.close();
      }
    }

    final status = value is Ref ? read(value.requireNode()) : readBuilt();
    if (status == Status.badValue.code) {
      throw BadValue(
        status,
        'not an exact length: [seconds, nanoseconds], a number of seconds, '
        'or {secs, nanos}',
      );
    }
    checkStatus(status);
    // A u64 past 2^63 - 1 reads back negative.
    final s = secs.value;
    final micros = nanos.value ~/ 1000;
    if (s < 0 ||
        s > _maxDurationSeconds ||
        (s == _maxDurationSeconds && micros > 775807)) {
      throw RangeError('the length does not fit a Duration');
    }
    return Duration(microseconds: s * 1000000 + micros);
  } finally {
    calloc.free(secs);
    calloc.free(nanos);
  }
}

/// [duration] as `[seconds, nanoseconds]`, the list an exact length is
/// written as. A negative one throws [ArgumentError]: an exact length is
/// never negative.
List<int> fromDuration(Duration duration) {
  if (duration.isNegative) {
    throw ArgumentError.value(
        duration, 'duration', 'an exact length is never negative');
  }
  final micros = duration.inMicroseconds;
  return [micros ~/ 1000000, (micros % 1000000) * 1000];
}

// ---- a span -------------------------------------------------------------------------------

/// A length of time in calendar units, each counted on its own, with one
/// sign for the whole span: `Span(days: 1, hours: 2)` is `P1DT2H`,
/// `Span(days: -1, hours: -2)` is `-P1DT2H`. Equal unit by unit:
/// `Span(days: 1) != Span(hours: 24)`, since where a calendar applies a
/// span a day is not always 24 hours.
///
/// Checked by the library: fields of both signs, or nanoseconds of a
/// second or more, throw [BadValue]. [toString] is the canonical ISO 8601
/// text; [Span.parse] also reads the friendly form, `1h 30m`.
final class Span {
  factory Span({
    int years = 0,
    int months = 0,
    int weeks = 0,
    int days = 0,
    int hours = 0,
    int minutes = 0,
    int seconds = 0,
    int nanoseconds = 0,
  }) {
    _fits(TextForm.span, [(nanoseconds, 32, true)]);
    final fields = [
      years,
      months,
      weeks,
      days,
      hours,
      minutes,
      seconds,
      nanoseconds
    ];
    final text = using((arena) {
      final f = _structOf(arena, fields);
      return _write(
        TextForm.span,
        (alloc, out) =>
            native.core().bindings.guatiao_span_format(alloc, f, out),
      );
    });
    return Span._(fields, text);
  }

  Span._(this._fields, this._text);

  /// An ISO 8601 duration (`P1Y2M3W4DT5H6M7.5S`, `-P1D`) or the friendly
  /// form (`1y 2mo 3w 4d 5h 6m 7s 500ms`). Throws [Unparsed] for text the
  /// grammar refuses.
  static Span parse(String text) {
    final lib = native.core();
    return using((arena) {
      final view = strOf(arena, text).ref;
      final out = arena<guatiao_span>();
      _read(
        TextForm.span,
        (error) => lib.bindings.guatiao_span_parse(lib.alloc, view, out, error),
      );
      final s = out.ref;
      return Span(
        years: s.years,
        months: s.months,
        weeks: s.weeks,
        days: s.days,
        hours: s.hours,
        minutes: s.minutes,
        seconds: s.seconds,
        nanoseconds: s.nanoseconds,
      );
    });
  }

  final List<int> _fields;
  final String _text;

  int get years => _fields[0];
  int get months => _fields[1];
  int get weeks => _fields[2];
  int get days => _fields[3];
  int get hours => _fields[4];
  int get minutes => _fields[5];
  int get seconds => _fields[6];

  /// Below a second, carrying the sign.
  int get nanoseconds => _fields[7];

  bool get isZero => _fields.every((n) => n == 0);

  bool get isNegative => _fields.any((n) => n < 0);

  /// The same span running the other way.
  Span operator -() => Span(
        years: -years,
        months: -months,
        weeks: -weeks,
        days: -days,
        hours: -hours,
        minutes: -minutes,
        seconds: -seconds,
        nanoseconds: -nanoseconds,
      );

  Pointer<guatiao_span> _struct(Arena arena) => _structOf(arena, _fields);

  static Pointer<guatiao_span> _structOf(Arena arena, List<int> f) {
    final s = arena<guatiao_span>();
    s.ref
      ..years = f[0]
      ..months = f[1]
      ..weeks = f[2]
      ..days = f[3]
      ..hours = f[4]
      ..minutes = f[5]
      ..seconds = f[6]
      ..nanoseconds = f[7];
    return s;
  }

  @override
  bool operator ==(Object other) {
    if (other is! Span) return false;
    for (var i = 0; i < 8; i++) {
      if (_fields[i] != other._fields[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode => Object.hashAll(_fields);

  /// The canonical ISO 8601 text: `-P1DT2H`, `PT0S` for zero.
  @override
  String toString() => _text;
}
