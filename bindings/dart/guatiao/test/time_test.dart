// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `package:guatiao/time.dart` against the shared vectors every
/// implementation reads, then the mappings to Dart's own types.
library;

import 'dart:convert';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/time.dart' as time;
import 'package:test/test.dart';

import 'support.dart';

Future<Map<String, Object?>> _vectors() async => jsonDecode(
      await readRepoText(
          ['crates', 'guatiao', 'tests', 'vectors', 'time.json']),
    ) as Map<String, Object?>;

/// Reads [text] as [form] through the Dart type, answering its fields as
/// the vectors list them and its canonical text.
(List<int>, String) _read(time.TextForm form, String text) {
  switch (form) {
    case time.TextForm.date:
      final d = time.PlainDate.parse(text);
      return ([d.year, d.month, d.day], '$d');
    case time.TextForm.time:
      final t = time.PlainTime.parse(text);
      return ([t.hour, t.minute, t.second, t.nanosecond], '$t');
    case time.TextForm.datetime:
      final at = time.PlainDateTime.parse(text);
      final (d, t) = (at.date, at.time);
      return (
        [d.year, d.month, d.day, t.hour, t.minute, t.second, t.nanosecond],
        '$at'
      );
    case time.TextForm.span:
      final s = time.Span.parse(text);
      return (
        [
          s.years,
          s.months,
          s.weeks,
          s.days,
          s.hours,
          s.minutes,
          s.seconds,
          s.nanoseconds,
        ],
        '$s'
      );
    case time.TextForm.timestamp:
      // The DateTime mapping drops nanoseconds: the exact text comes from
      // normalize, the fields from what DateTime keeps.
      final t = time.parseTimestamp(text);
      final millis = t.millisecondsSinceEpoch;
      final below = millis % 1000;
      return (
        [(millis - below) ~/ 1000, below * 1000000 + t.microsecond * 1000],
        time.normalize(form, text)
      );
  }
}

/// Whether every field is an integer this platform's `int` holds exactly:
/// all of them on the VM and dart2wasm, those within 2^53 under dart2js.
bool _exactHere(List<num> fields) =>
    !jsNumbers || fields.every((f) => f.abs() <= 9007199254740992);

/// Says how many vectors a test passed over as past 2^53.
void _report(int passedOver) {
  if (passedOver > 0) {
    print('skipped $passedOver vectors with a number past 2^53, which a '
        'dart2js int does not hold');
  }
}

/// A text spelling a number past 2^53, under dart2js.
bool _pastJsIntegers(String text) =>
    jsNumbers && RegExp(r'\d{16,}').hasMatch(text);

void main() {
  late final Map<String, Object?> vectors;
  setUpAll(() async {
    await setUpLibrary();
    vectors = await _vectors();
  });

  for (final form in time.TextForm.values) {
    test('every ${form.name} vector', () {
      var passedOver = 0;
      addTearDown(() => _report(passedOver));
      final cases = (vectors[form.name]! as List).cast<Map<String, Object?>>();
      expect(cases, isNotEmpty);
      for (final c in cases) {
        final text = c['text']! as String;
        final refusedAt = c['refused_at'] as int?;
        if (refusedAt != null) {
          for (final read in [
            () => _read(form, text),
            () => time.normalize(form, text),
          ]) {
            expect(
              read,
              throwsA(
                isA<time.Unparsed>()
                    .having((e) => e.at, 'at', refusedAt)
                    .having((e) => e.form, 'form', form)
                    .having((e) => e.expected, 'expected', isNotEmpty),
              ),
              reason: text,
            );
          }
          continue;
        }
        final canonical = c['canonical']! as String;
        final fields = (c['fields']! as List).cast<int>();
        if (!_exactHere(fields)) {
          passedOver++;
          continue;
        }
        final (got, written) = _read(form, text);
        if (form == time.TextForm.timestamp) {
          expect(got[0], fields[0], reason: text);
          expect(got[1], fields[1] ~/ 1000 * 1000, reason: text);
        } else {
          expect(got, fields, reason: text);
        }
        expect(written, canonical, reason: text);
        expect(time.normalize(form, text), canonical, reason: text);
      }
    });
  }

  test('every span applied', () {
    var passedOver = 0;
    addTearDown(() => _report(passedOver));
    for (final c in (vectors['date_add_span']! as List).cast<Map>()) {
      if (_pastJsIntegers(c['span'] as String)) {
        passedOver++;
        continue;
      }
      final start = time.PlainDate.parse(c['start'] as String);
      final span = time.Span.parse(c['span'] as String);
      final result = c['result'] as String?;
      if (result == null) {
        expect(() => start.add(span), throwsA(isA<BadValue>()), reason: '$c');
      } else {
        expect('${start.add(span)}', result, reason: '$c');
      }
    }
    for (final c in (vectors['datetime_add_span']! as List).cast<Map>()) {
      if (_pastJsIntegers(c['span'] as String)) {
        passedOver++;
        continue;
      }
      final start = time.PlainDateTime.parse(c['start'] as String);
      final span = time.Span.parse(c['span'] as String);
      final result = c['result'] as String?;
      if (result == null) {
        expect(() => start.add(span), throwsA(isA<BadValue>()), reason: '$c');
      } else {
        expect('${start.add(span)}', result, reason: '$c');
      }
    }
  });

  test('every exact length', () {
    for (final c in (vectors['duration']! as List).cast<Map>()) {
      final expected = (c['duration'] as List?)?.cast<int>();
      if (expected == null) {
        expect(() => time.toDuration(c['value']), throwsA(isA<BadValue>()),
            reason: '$c');
      } else if (expected[0] > 9223372036854) {
        expect(() => time.toDuration(c['value']), throwsA(isA<RangeError>()),
            reason: '$c');
      } else {
        expect(
          time.toDuration(c['value']),
          Duration(microseconds: expected[0] * 1000000 + expected[1] ~/ 1000),
          reason: '$c',
        );
      }
    }
  });

  test('the Plain types keep nanoseconds, compare and are checked', () {
    final t = time.PlainTime(12, 0, 0, 1);
    expect('$t', '12:00:00.000000001');
    expect(time.PlainTime.parse('12:00:00.000000001'), t);
    expect(time.PlainDate(2026, 9, 30).compareTo(time.PlainDate(2026, 10, 1)),
        lessThan(0));
    expect(() => time.PlainDate(2026, 2, 29), throwsA(isA<BadValue>()));
    expect(() => time.PlainDate(2026, 256, 1), throwsA(isA<BadValue>()));
    expect(() => time.PlainTime(24, 0, 0), throwsA(isA<BadValue>()));
    expect(time.PlainDate(0, 1, 1).toString(), '0000-01-01');
    final local = DateTime(2026, 10, 21, 13, 45, 0, 5, 6);
    expect('${time.PlainDateTime.fromDateTime(local)}',
        '2026-10-21T13:45:00.005006');
    expect(
      {time.PlainDate(2026, 1, 1), time.PlainDate.parse('2026-01-01')},
      hasLength(1),
    );
  });

  test('an instant is a UTC DateTime', () {
    final t = time.parseTimestamp('2026-10-21T19:15:00.5+05:30');
    expect(t.isUtc, isTrue);
    expect(t, DateTime.utc(2026, 10, 21, 13, 45, 0, 500));
    expect(time.parseTimestamp('1969-12-31T23:59:59.9999999Z'),
        DateTime.utc(1969, 12, 31, 23, 59, 59, 999, 999),
        reason: 'toward the past');
    expect(time.formatTimestamp(DateTime.utc(1969, 12, 31, 23, 59, 59, 500)),
        '1969-12-31T23:59:59.5Z');
    expect(time.parseTimestamp('0000-01-01T00:00:00Z'), DateTime.utc(0));
    final local = DateTime(2026, 10, 21, 13, 45);
    expect(time.formatTimestamp(local), time.formatTimestamp(local.toUtc()));
  });

  test('an exact length is a Duration', () {
    expect(time.toDuration([90, 500000000]),
        const Duration(seconds: 90, milliseconds: 500));
    expect(time.toDuration(1.5), const Duration(milliseconds: 1500));
    expect(
        time.toDuration({'secs': 1, 'nanos': 999}), const Duration(seconds: 1));
    final v = Value.fromDart({
      'd': [3, 0]
    });
    addTearDown(v.close);
    expect(time.toDuration(v.asMap()['d']), const Duration(seconds: 3));
    expect(time.fromDuration(const Duration(seconds: 90, milliseconds: 500)),
        [90, 500000000]);
    expect(() => time.fromDuration(const Duration(seconds: -1)),
        throwsArgumentError);
    expect(() => time.toDuration('90'), throwsA(isA<BadValue>()));
  });

  test('a span is its own class, backed by the library', () {
    final span = time.Span.parse('1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns');
    expect('$span', 'P1Y2M3W4DT5H6M7.500001001S');
    expect(span.nanoseconds, 500001001);
    expect(time.Span(days: 1) == time.Span(hours: 24), isFalse);
    expect(time.Span(days: -1, hours: -2), time.Span.parse('-P1DT2H'));
    expect('${-time.Span(days: 1)}', '-P1D');
    expect(time.Span().isZero, isTrue);
    expect('${time.Span()}', 'PT0S');
    expect(time.Span(days: -1).isNegative, isTrue);
    expect(() => time.Span(days: 1, hours: -1), throwsA(isA<BadValue>()));
    expect(() => time.Span(nanoseconds: 1000000000), throwsA(isA<BadValue>()));
    expect(
      time.PlainDate(2026, 1, 31).add(time.Span(months: 1)),
      time.PlainDate(2026, 2, 28),
    );
    expect(
      () => time.PlainDate(2026, 10, 21).add(time.Span(hours: 1)),
      throwsA(isA<BadValue>()),
    );
  });

  test('redaction hides every secret the schema marks', () {
    final schema = {
      'type': 'object',
      'properties': {
        'user': {'type': 'string'},
        'password': {'type': 'string', 'writeOnly': true, 'x-sensitive': true},
        'tls': {
          'type': 'object',
          'properties': {
            'key': {r'$ref': r'#/$defs/Key'},
          },
        },
      },
      r'$defs': {
        'Key': {'type': 'string', 'x-sensitive': true},
      },
    };
    final shown = redact(schema, {
      'user': 'ana',
      'password': 'hunter2',
      'tls': {'key': 'k'},
      'extra': 'x',
    });
    addTearDown(shown.close);
    expect(shown.toDart(), {
      'user': 'ana',
      'password': '[redacted]',
      'tls': {'key': '[redacted]'},
      'extra': '[redacted]',
    });
    expect(() => redact(5, {}), throwsA(isA<WrongKind>()));
  });
}
