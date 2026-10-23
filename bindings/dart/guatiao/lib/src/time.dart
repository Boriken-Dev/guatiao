// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:math' as math;

import 'status.dart';
import 'time_native.dart' if (dart.library.js_interop) 'web/time_web.dart'
    as calls;
import 'value.dart' if (dart.library.js_interop) 'web/value_web.dart';

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

/// Throws for a status a text-reading call answered: [Unparsed] for
/// text the grammar refused, else the status's own exception. Not part of
/// the package's API.
void refusedText(TextForm form, int status, String expected, int at) {
  if (status == Status.ok.code) return;
  if (status == Status.badValue.code) {
    throw Unparsed(
      expected.isEmpty ? 'a valid text' : expected,
      form: form,
      at: at,
    );
  }
  checkStatus(status, expected);
}

/// Throws for a status a `_format` call answered: [BadValue] for fields
/// that are not a [form], else the status's own exception. Not part of
/// the package's API.
void refusedFields(TextForm form, int status) {
  if (status == Status.badValue.code) {
    throw BadValue(status, 'the fields are not a ${form.name}');
  }
  checkStatus(status);
}

/// Throws [BadValue] unless every field fits its C width.
void _fits(TextForm form, List<(int, int, bool)> fields) {
  for (final (value, bits, signed) in fields) {
    // A power rather than a shift: dart2js shifts in 32 bits.
    final ok = signed
        ? (bits >= 64 ||
            (value >= -math.pow(2, bits - 1) && value < math.pow(2, bits - 1)))
        : (value >= 0 && value < math.pow(2, bits));
    if (!ok) {
      throw BadValue(Status.badValue.code, 'the fields are not a ${form.name}');
    }
  }
}

/// [text] read as [form] and rewritten in its canonical spelling: an
/// instant in UTC, a fraction without trailing zeros, a span in ISO 8601.
/// Exact to the nanosecond. Throws [Unparsed] for text the grammar
/// refuses.
String normalize(TextForm form, String text) => calls.normalizeText(form, text);

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
    final text = calls.formatDate([year, month, day]);
    return PlainDate._(year, month, day, text);
  }

  PlainDate._(this.year, this.month, this.day, this._text);

  /// The date [moment] shows, in whichever zone it is in.
  factory PlainDate.fromDateTime(DateTime moment) =>
      PlainDate(moment.year, moment.month, moment.day);

  /// `YYYY-MM-DD`. Throws [Unparsed] for text the grammar refuses.
  static PlainDate parse(String text) {
    final d = calls.parseDate(text);
    return PlainDate(d[0], d[1], d[2]);
  }

  final int year;
  final int month;
  final int day;
  final String _text;

  /// This date moved by [span]: years and months first, a day past the
  /// month's end becoming its last day, then weeks and days. Throws
  /// [BadValue] for a span with time units or a result outside the
  /// years 0000 to 9999.
  PlainDate add(Span span) {
    final d = calls.dateAddSpan([year, month, day], span._fields);
    return PlainDate(d[0], d[1], d[2]);
  }

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
    final text = calls.formatTime([hour, minute, second, nanosecond]);
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
    final t = calls.parseTime(text);
    return PlainTime(t[0], t[1], t[2], t[3]);
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

// ---- a civil date and time -------------------------------------------------------

/// A date and a time of day as a wall clock shows them, with no zone:
/// `2026-10-21T13:45:00.5`. Dart's `DateTime` would read one in UTC or the
/// machine's zone, and a local one moves a time a daylight-saving change
/// skips; this keeps the fields as written, to the nanosecond.
final class PlainDateTime implements Comparable<PlainDateTime> {
  factory PlainDateTime(PlainDate date, PlainTime time) {
    final text = calls.formatDateTime(_fieldsOf(date, time));
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
  static PlainDateTime parse(String text) => _of(calls.parseDateTime(text));

  static PlainDateTime _of(List<int> f) => PlainDateTime(
        PlainDate(f[0], f[1], f[2]),
        PlainTime(f[3], f[4], f[5], f[6]),
      );

  static List<int> _fieldsOf(PlainDate date, PlainTime time) => [
        date.year,
        date.month,
        date.day,
        time.hour,
        time.minute,
        time.second,
        time.nanosecond,
      ];

  final PlainDate date;
  final PlainTime time;
  final String _text;

  /// This date and time moved by [span]: as [PlainDate.add], then hours
  /// to nanoseconds carried across midnight, a day being 24 hours.
  /// Throws [BadValue] for a result outside the years 0000 to 9999.
  PlainDateTime add(Span span) =>
      _of(calls.dateTimeAddSpan(_fieldsOf(date, time), span._fields));

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
  final (seconds, nanoseconds) = calls.parseTimestampFields(text);
  // Milliseconds first: microseconds since the epoch pass 2^53 within the
  // years 0000 to 9999, which a dart2js `int` does not hold exactly.
  return DateTime.fromMillisecondsSinceEpoch(
    seconds * 1000 + nanoseconds ~/ 1000000,
    isUtc: true,
  ).add(Duration(microseconds: nanoseconds ~/ 1000 % 1000));
}

/// [moment] as an instant in UTC, `...Z`, whichever zone it is in.
String formatTimestamp(DateTime moment) {
  final millis = moment.millisecondsSinceEpoch;
  final below = millis % 1000;
  return calls.formatTimestampFields(
      (millis - below) ~/ 1000, below * 1000000 + moment.microsecond * 1000);
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
  final (status, s, nanos) = calls.readDuration(value);
  if (status == Status.badValue.code) {
    throw BadValue(
      status,
      'not an exact length: [seconds, nanoseconds], a number of seconds, '
      'or {secs, nanos}',
    );
  }
  checkStatus(status);
  // A u64 past 2^63 - 1 reads back negative.
  final micros = nanos ~/ 1000;
  if (s < 0 ||
      s > _maxDurationSeconds ||
      (s == _maxDurationSeconds && micros > 775807)) {
    throw RangeError('the length does not fit a Duration');
  }
  return Duration(microseconds: s * 1000000 + micros);
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
    return Span._(fields, calls.formatSpan(fields));
  }

  Span._(this._fields, this._text);

  /// An ISO 8601 duration (`P1Y2M3W4DT5H6M7.5S`, `-P1D`) or the friendly
  /// form (`1y 2mo 3w 4d 5h 6m 7s 500ms`). Throws [Unparsed] for text the
  /// grammar refuses.
  static Span parse(String text) {
    final s = calls.parseSpan(text);
    return Span(
      years: s[0],
      months: s[1],
      weeks: s[2],
      days: s[3],
      hours: s[4],
      minutes: s[5],
      seconds: s[6],
      nanoseconds: s[7],
    );
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
