// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The calls behind `time.dart`, over the loaded WebAssembly module.
library;

import '../status.dart';
import '../time.dart';
import 'module.dart';
import 'value_web.dart';

// The wasm32 layouts: `guatiao_date` {i32 year; u8 month, day} is 8
// bytes, `guatiao_time` {u8 hour, minute, second; u32 nanosecond} 8,
// `guatiao_datetime` the two, `guatiao_timestamp` {i64; u32} 16,
// `guatiao_span` seven i64 then an i32, 64; `guatiao_text_error` is a
// size_t then a value.
const int _dateSize = 8;
const int _timeSize = 8;
const int _timestampSize = 16;
const int _spanSize = 64;
const int _textErrorSize = wordSize + valueSize;

String _table(TextForm form) => switch (form) {
  TextForm.date => 'date',
  TextForm.time => 'time',
  TextForm.datetime => 'datetime',
  TextForm.timestamp => 'timestamp',
  TextForm.span => 'span',
};

/// Runs a call that may refuse text through [error], then throws
/// [Unparsed] for a refusal; the message is freed either way.
void _read(TextForm form, Address error, int status) {
  final m = WasmModule.current;
  final message = error + wordSize;
  final expected = Ref.borrowed(message).stringOr('');
  final at = m.u32(error);
  m.call('guatiao_value_free', [message]);
  refusedText(form, status, expected, at);
}

String _write(TextForm form, Scratch s, Address fields) {
  final m = s.module;
  final out = m.alloc(valueSize);
  final status = m.call('guatiao_${_table(form)}_format', [
    m.defaultAlloc,
    fields,
    out,
  ]);
  final text = takeValue(out);
  refusedFields(form, status);
  return text! as String;
}

/// Parses [text] into a struct of [size] bytes, answering where it is.
Address _parse(TextForm form, Scratch s, String text, int size) {
  final m = s.module;
  final out = s.alloc(size);
  final error = s.alloc(_textErrorSize);
  final status = m.call('guatiao_${_table(form)}_parse', [
    m.defaultAlloc,
    s.str(text),
    out,
    error,
  ]);
  _read(form, error, status);
  return out;
}

String normalizeText(TextForm form, String text) => withScratch((s) {
  final m = s.module;
  final out = s.value();
  final error = s.alloc(_textErrorSize);
  try {
    final status = m.call('guatiao_${_table(form)}_normalize', [
      m.defaultAlloc,
      s.str(text),
      out,
      error,
    ]);
    _read(form, error, status);
    return Ref.borrowed(out).stringOr('');
  } finally {
    m.call('guatiao_value_free', [out]);
  }
});

void _setDate(WasmModule m, Address at, List<int> d) {
  m.setI32(at, d[0]);
  m.setU8(at + 4, d[1]);
  m.setU8(at + 5, d[2]);
}

List<int> _date(WasmModule m, Address at) => [
  m.i32(at),
  m.u8(at + 4),
  m.u8(at + 5),
];

void _setTime(WasmModule m, Address at, List<int> t) {
  m.setU8(at, t[0]);
  m.setU8(at + 1, t[1]);
  m.setU8(at + 2, t[2]);
  m.setU32(at + 4, t[3]);
}

List<int> _time(WasmModule m, Address at) => [
  m.u8(at),
  m.u8(at + 1),
  m.u8(at + 2),
  m.u32(at + 4),
];

Address _span(Scratch s, List<int> f) {
  final at = s.alloc(_spanSize);
  for (var i = 0; i < 7; i++) {
    s.module.setI64(at + 8 * i, f[i]);
  }
  s.module.setI32(at + 56, f[7]);
  return at;
}

String formatDate(List<int> d) => withScratch((s) {
  final f = s.alloc(_dateSize);
  _setDate(s.module, f, d);
  return _write(TextForm.date, s, f);
});

List<int> parseDate(String text) => withScratch(
  (s) => _date(s.module, _parse(TextForm.date, s, text, _dateSize)),
);

List<int> dateAddSpan(List<int> d, List<int> span) => withScratch((s) {
  final m = s.module;
  final f = s.alloc(_dateSize);
  _setDate(m, f, d);
  final out = s.alloc(_dateSize);
  checkStatus(m.call('guatiao_date_add_span', [f, _span(s, span), out]));
  return _date(m, out);
});

String formatTime(List<int> t) => withScratch((s) {
  final f = s.alloc(_timeSize);
  _setTime(s.module, f, t);
  return _write(TextForm.time, s, f);
});

List<int> parseTime(String text) => withScratch(
  (s) => _time(s.module, _parse(TextForm.time, s, text, _timeSize)),
);

void _setDateTime(WasmModule m, Address at, List<int> f) {
  _setDate(m, at, f.sublist(0, 3));
  _setTime(m, at + _dateSize, f.sublist(3));
}

List<int> _dateTime(WasmModule m, Address at) => [
  ..._date(m, at),
  ..._time(m, at + _dateSize),
];

String formatDateTime(List<int> at) => withScratch((s) {
  final f = s.alloc(_dateSize + _timeSize);
  _setDateTime(s.module, f, at);
  return _write(TextForm.datetime, s, f);
});

List<int> parseDateTime(String text) => withScratch(
  (s) => _dateTime(
    s.module,
    _parse(TextForm.datetime, s, text, _dateSize + _timeSize),
  ),
);

List<int> dateTimeAddSpan(List<int> at, List<int> span) => withScratch((s) {
  final m = s.module;
  final f = s.alloc(_dateSize + _timeSize);
  _setDateTime(m, f, at);
  final out = s.alloc(_dateSize + _timeSize);
  checkStatus(m.call('guatiao_datetime_add_span', [f, _span(s, span), out]));
  return _dateTime(m, out);
});

(int, int) parseTimestampFields(String text) => withScratch((s) {
  final out = _parse(TextForm.timestamp, s, text, _timestampSize);
  return (s.module.i64(out), s.module.u32(out + 8));
});

String formatTimestampFields(int seconds, int nanoseconds) => withScratch((s) {
  final f = s.alloc(_timestampSize);
  s.module.setI64(f, seconds);
  s.module.setU32(f + 8, nanoseconds);
  return _write(TextForm.timestamp, s, f);
});

String formatSpan(List<int> fields) =>
    withScratch((s) => _write(TextForm.span, s, _span(s, fields)));

List<int> parseSpan(String text) => withScratch((s) {
  final m = s.module;
  final at = _parse(TextForm.span, s, text, _spanSize);
  return [for (var i = 0; i < 7; i++) m.i64(at + 8 * i), m.i32(at + 56)];
});

/// `guatiao_duration_read` over [value]: its status, the whole seconds
/// (a `uint64_t` past 2^63 - 1 reads back negative) and the nanoseconds.
(int, int, int) readDuration(Object? value) => withScratch((s) {
  final m = s.module;
  final secs = s.alloc(8);
  final nanos = s.alloc(4, 4);
  int read(Address node) =>
      m.call('guatiao_duration_read', [node, secs, nanos]);
  final int status;
  if (value is Ref) {
    status = read(value.requireNode());
  } else {
    final built = Value.fromDart(value);
    try {
      status = read(built.pointer);
    } finally {
      built.close();
    }
  }
  return (status, m.i64(secs), m.u32(nanos));
});
