// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The calls behind `time.dart`, through the loaded native library.
library;

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'time.dart';
import 'value.dart';

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
    refusedText(form, status, expected, error.ref.at);
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
  refusedFields(form, status);
  return text! as String;
}

String normalizeText(TextForm form, String text) {
  final lib = native.core();
  final b = lib.bindings;
  final out = calloc<guatiao_value>();
  try {
    using((arena) {
      final view = strOf(arena, text).ref;
      _read(
        form,
        (error) => switch (form) {
          TextForm.date => b.guatiao_date_normalize(
            lib.alloc,
            view,
            out,
            error,
          ),
          TextForm.time => b.guatiao_time_normalize(
            lib.alloc,
            view,
            out,
            error,
          ),
          TextForm.datetime => b.guatiao_datetime_normalize(
            lib.alloc,
            view,
            out,
            error,
          ),
          TextForm.timestamp => b.guatiao_timestamp_normalize(
            lib.alloc,
            view,
            out,
            error,
          ),
          TextForm.span => b.guatiao_span_normalize(
            lib.alloc,
            view,
            out,
            error,
          ),
        },
      );
    });
    return Ref.borrowed(out).stringOr('');
  } finally {
    b.guatiao_value_free(out);
    calloc.free(out);
  }
}

void _date(Pointer<guatiao_date> f, List<int> d) => f.ref
  ..year = d[0]
  ..month = d[1]
  ..day = d[2];

void _time(Pointer<guatiao_time> f, List<int> t) => f.ref
  ..hour = t[0]
  ..minute = t[1]
  ..second = t[2]
  ..nanosecond = t[3];

void _dateTime(Pointer<guatiao_datetime> f, List<int> at) {
  f.ref.date
    ..year = at[0]
    ..month = at[1]
    ..day = at[2];
  f.ref.time
    ..hour = at[3]
    ..minute = at[4]
    ..second = at[5]
    ..nanosecond = at[6];
}

List<int> _dateTimeOf(guatiao_datetime f) => [
  f.date.year,
  f.date.month,
  f.date.day,
  f.time.hour,
  f.time.minute,
  f.time.second,
  f.time.nanosecond,
];

Pointer<guatiao_span> _span(Arena arena, List<int> f) {
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

String formatDate(List<int> d) => using((arena) {
  final f = arena<guatiao_date>();
  _date(f, d);
  return _write(
    TextForm.date,
    (alloc, out) => native.core().bindings.guatiao_date_format(alloc, f, out),
  );
});

List<int> parseDate(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_date>();
    _read(
      TextForm.date,
      (error) => lib.bindings.guatiao_date_parse(lib.alloc, view, out, error),
    );
    return [out.ref.year, out.ref.month, out.ref.day];
  });
}

List<int> dateAddSpan(List<int> d, List<int> span) => using((arena) {
  final f = arena<guatiao_date>();
  _date(f, d);
  final out = arena<guatiao_date>();
  checkStatus(
    native.core().bindings.guatiao_date_add_span(f, _span(arena, span), out),
  );
  return [out.ref.year, out.ref.month, out.ref.day];
});

String formatTime(List<int> t) => using((arena) {
  final f = arena<guatiao_time>();
  _time(f, t);
  return _write(
    TextForm.time,
    (alloc, out) => native.core().bindings.guatiao_time_format(alloc, f, out),
  );
});

List<int> parseTime(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_time>();
    _read(
      TextForm.time,
      (error) => lib.bindings.guatiao_time_parse(lib.alloc, view, out, error),
    );
    final t = out.ref;
    return [t.hour, t.minute, t.second, t.nanosecond];
  });
}

String formatDateTime(List<int> at) => using((arena) {
  final f = arena<guatiao_datetime>();
  _dateTime(f, at);
  return _write(
    TextForm.datetime,
    (alloc, out) =>
        native.core().bindings.guatiao_datetime_format(alloc, f, out),
  );
});

List<int> parseDateTime(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_datetime>();
    _read(
      TextForm.datetime,
      (error) =>
          lib.bindings.guatiao_datetime_parse(lib.alloc, view, out, error),
    );
    return _dateTimeOf(out.ref);
  });
}

List<int> dateTimeAddSpan(List<int> at, List<int> span) => using((arena) {
  final f = arena<guatiao_datetime>();
  _dateTime(f, at);
  final out = arena<guatiao_datetime>();
  checkStatus(
    native.core().bindings.guatiao_datetime_add_span(
      f,
      _span(arena, span),
      out,
    ),
  );
  return _dateTimeOf(out.ref);
});

(int, int) parseTimestampFields(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_timestamp>();
    _read(
      TextForm.timestamp,
      (error) =>
          lib.bindings.guatiao_timestamp_parse(lib.alloc, view, out, error),
    );
    return (out.ref.seconds, out.ref.nanoseconds);
  });
}

String formatTimestampFields(int seconds, int nanoseconds) => using((arena) {
  final f = arena<guatiao_timestamp>();
  f.ref
    ..seconds = seconds
    ..nanoseconds = nanoseconds;
  return _write(
    TextForm.timestamp,
    (alloc, out) =>
        native.core().bindings.guatiao_timestamp_format(alloc, f, out),
  );
});

String formatSpan(List<int> fields) => using((arena) {
  final f = _span(arena, fields);
  return _write(
    TextForm.span,
    (alloc, out) => native.core().bindings.guatiao_span_format(alloc, f, out),
  );
});

List<int> parseSpan(String text) {
  final lib = native.core();
  return using((arena) {
    final view = strOf(arena, text).ref;
    final out = arena<guatiao_span>();
    _read(
      TextForm.span,
      (error) => lib.bindings.guatiao_span_parse(lib.alloc, view, out, error),
    );
    final s = out.ref;
    return [
      s.years,
      s.months,
      s.weeks,
      s.days,
      s.hours,
      s.minutes,
      s.seconds,
      s.nanoseconds,
    ];
  });
}

/// `guatiao_duration_read` over [value]: its status, the whole seconds
/// (a `uint64_t` past 2^63 - 1 reads back negative) and the nanoseconds.
(int, int, int) readDuration(Object? value) {
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
    return (status, secs.value, nanos.value);
  } finally {
    calloc.free(secs);
    calloc.free(nanos);
  }
}
