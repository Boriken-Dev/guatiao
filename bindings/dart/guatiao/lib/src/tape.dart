// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The wire encoding read into plain Dart and written from it in Dart,
// with no library: one crossing carries a whole document as bytes, where
// walking a tree costs a crossing per node. Bytes a library wrote were
// checked as it wrote them, so reading them guards against nothing worse
// than a cut-short body; bytes a peer sent are read strictly.

import 'dart:convert';
import 'dart:typed_data';

import 'status.dart';
import 'value.dart' if (dart.library.js_interop) 'web/value_web.dart' show Ref;
import 'value_common.dart';

/// How deep containers nest: what every walk in the library follows.
const int _maxDepth = 128;

/// The plain Dart value wire bytes hold, as `Ref.toDart` answers it for
/// the same tree, with no tree built. For bytes a library just wrote,
/// such as a document read straight into wire bytes: it checks no more
/// than that each body is whole ([BadValue] otherwise), so bytes from
/// anywhere else go through `Value.fromWire`.
Object? wireToDart(Uint8List data, {Numbers numbers = Numbers.auto}) {
  final reader = _Reader(data, numbers, strict: false);
  try {
    final value = reader.value(0);
    if (reader.at != data.length) {
      throw BadValue(
        Status.badValue.code,
        'bytes after the value, from ${reader.at}',
      );
    }
    return value;
  } on RangeError {
    throw BadValue(Status.badValue.code, 'the bytes end inside a node');
  } on FormatException {
    throw BadValue(Status.badValue.code, 'text that is not UTF-8');
  }
}

/// [wireToDart] for bytes from anywhere, a peer's included: refuses, as
/// the wire encoding does, a length not in its shortest form or past what
/// remains, a number outside the JSON number grammar, a bool byte other
/// than 0 or 1 and a key repeated, each [BadValue] naming the byte.
Object? wireToDartStrict(Uint8List data, {Numbers numbers = Numbers.auto}) {
  final reader = _Reader(data, numbers, strict: true);
  final Object? value;
  try {
    value = reader.value(0);
  } on RangeError {
    throw _refused('the bytes end inside a node', reader.at);
  }
  if (reader.at != data.length) {
    throw _refused('bytes after the value', reader.at);
  }
  return value;
}

/// Bytes that are not a value: why, and the byte it went wrong at.
BadValue _refused(String why, int at) =>
    BadValue(Status.badValue.code, '$why at $at');

/// The JSON number grammar, which a number's text follows.
final RegExp _number = RegExp(
  r'^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$',
);

/// UTF-8 text, a leading U+FEFF kept as the text it is: Dart's decoder
/// drops one. [FormatException] for bytes that are not UTF-8.
String utf8Text(Uint8List bytes) {
  var marks = 0;
  while (bytes.length >= 3 * (marks + 1) &&
      bytes[3 * marks] == 0xef &&
      bytes[3 * marks + 1] == 0xbb &&
      bytes[3 * marks + 2] == 0xbf) {
    marks++;
  }
  final rest = utf8.decode(Uint8List.sublistView(bytes, 3 * marks));
  return marks == 0 ? rest : '${'\ufeff' * marks}$rest';
}

final class _Reader {
  _Reader(this.data, this.numbers, {required this.strict});

  final Uint8List data;
  final Numbers numbers;
  final bool strict;
  int at = 0;

  int byte() {
    if (strict && at >= data.length) {
      throw _refused('the bytes end inside a node', at);
    }
    return data[at++];
  }

  int count() {
    if (strict) return _strictCount();
    var b = data[at++];
    if (b < 0x80) return b;
    var n = b & 0x7f;
    var shift = 7;
    while (true) {
      b = data[at++];
      n |= (b & 0x7f) << shift;
      if (b < 0x80) return n;
      shift += 7;
    }
  }

  /// A length or a count in its shortest form, at most what is left: every
  /// item takes a byte at least.
  int _strictCount() {
    final start = at;
    // A double, as on every platform: a length past 2^53 is past what
    // remains anyway.
    var n = 0.0;
    for (var shift = 0; ; shift += 7) {
      if (shift >= 64) throw _refused('a length past 64 bits', start);
      final b = byte();
      if (shift == 63 && (b & 0x7f) > 1) {
        throw _refused('a length past 64 bits', start);
      }
      n += (b & 0x7f) * _pow2(shift);
      if (b < 0x80) {
        if (b == 0 && shift > 0) {
          throw _refused('a length not in its shortest form', start);
        }
        if (n > data.length - at) {
          throw _refused('a length past what remains', at);
        }
        return n.toInt();
      }
    }
  }

  static double _pow2(int shift) {
    var p = 1.0;
    for (var i = 0; i < shift; i++) {
      p *= 2;
    }
    return p;
  }

  Uint8List body() {
    final n = count();
    if (at + n > data.length) throw RangeError('cut short');
    final out = Uint8List.sublistView(data, at, at + n);
    at += n;
    return out;
  }

  String text() {
    final start = at;
    final raw = body();
    try {
      return utf8Text(raw);
    } on FormatException {
      if (strict) throw _refused('text that is not UTF-8', start);
      rethrow;
    }
  }

  Object? value(int depth) {
    final tagAt = at;
    final tag = byte();
    switch (tag) {
      case 4:
        return text();
      case 3:
        final start = at;
        final text = body();
        if (strict && !_number.hasMatch(String.fromCharCodes(text))) {
          throw _refused('a number outside the JSON number grammar', start);
        }
        return _numberOf(text);
      case 7:
        final n = count();
        if (n > 0 && depth >= _maxDepth) throw _tooDeep();
        final map = <String, Object?>{};
        for (var i = 0; i < n; i++) {
          final keyAt = at;
          final key = text();
          if (strict && map.containsKey(key)) {
            throw _refused('a key repeated', keyAt);
          }
          map[key] = value(depth + 1);
        }
        return map;
      case 6:
        final n = count();
        if (n > 0 && depth >= _maxDepth) throw _tooDeep();
        return <Object?>[for (var i = 0; i < n; i++) value(depth + 1)];
      case 1:
        return null;
      case 2:
        final b = byte();
        if (strict && b > 1) {
          throw _refused('a bool byte other than 0 or 1', at - 1);
        }
        return b == 1;
      case 5:
        return Uint8List.fromList(body());
      case 0:
        return absent;
    }
    throw strict
        ? _refused('an unknown tag $tag', tagAt)
        : BadValue(Status.badValue.code, 'unknown tag $tag at ${at - 1}');
  }

  Object _numberOf(Uint8List text) {
    // Fifteen digits fit an `int` on every platform, 53-bit ones included.
    if (numbers == Numbers.auto && text.length <= 15) {
      var integral = true;
      for (final c in text) {
        if (c == 0x2e || c == 0x65 || c == 0x45) {
          integral = false;
          break;
        }
      }
      if (integral) {
        // Under dart2js `int.parse('-0')` is -0.0; the integer is 0.
        final n = int.parse(String.fromCharCodes(text));
        return n == 0 ? 0 : n;
      }
    }
    return numberFromText(String.fromCharCodes(text), numbers);
  }
}

BadValue _tooDeep() =>
    BadValue(Status.badValue.code, 'nested deeper than $_maxDepth');

/// The wire bytes of what `Value.fromDart(obj)` would build, written in
/// Dart with no tree built: the same objects accepted and the same errors
/// thrown. For a call that takes wire bytes and writes them out as
/// something else.
Uint8List dartToWire(Object? obj) {
  final out = _Writer();
  out.value(obj, 0);
  return out.done();
}

final class _Writer {
  Uint8List _buf = Uint8List(256);
  int _len = 0;

  void _room(int n) {
    if (_len + n <= _buf.length) return;
    var size = _buf.length * 2;
    while (size < _len + n) {
      size *= 2;
    }
    _buf = Uint8List(size)..setRange(0, _len, _buf);
  }

  void byte(int b) {
    _room(1);
    _buf[_len++] = b;
  }

  void count(int n) {
    _room(10);
    while (n >= 0x80) {
      _buf[_len++] = (n & 0x7f) | 0x80;
      n >>= 7;
    }
    _buf[_len++] = n;
  }

  void bytes(List<int> data) {
    count(data.length);
    _room(data.length);
    _buf.setRange(_len, _len + data.length, data);
    _len += data.length;
  }

  void ascii(String text) {
    count(text.length);
    _room(text.length);
    for (var i = 0; i < text.length; i++) {
      _buf[_len++] = text.codeUnitAt(i);
    }
  }

  Uint8List done() => Uint8List.sublistView(_buf, 0, _len);

  void value(Object? obj, int depth) {
    if (obj is String) {
      byte(4);
      bytes(utf8.encode(obj));
    } else if (obj == null) {
      byte(1);
    } else if (obj is bool) {
      byte(2);
      byte(obj ? 1 : 0);
    } else if (obj is num || obj is BigInt) {
      // Under dart2js an infinity is an `int` as well as a `double`.
      if (obj is double && !obj.isFinite) {
        throw ArgumentError.value(obj, 'value', 'is not a finite number');
      }
      byte(3);
      ascii(obj.toString());
    } else if (obj is TypedData) {
      // Before List, which a Uint8List also is.
      byte(5);
      bytes(
        obj is Uint8List
            ? obj
            : Uint8List.view(obj.buffer, obj.offsetInBytes, obj.lengthInBytes),
      );
    } else if (obj is Map) {
      if (depth >= _maxDepth) throw _tooDeep();
      byte(7);
      count(obj.length);
      for (final entry in obj.entries) {
        final key = entry.key;
        if (key is! String) {
          throw ArgumentError.value(key, 'key', 'a map key must be a String');
        }
        bytes(utf8.encode(key));
        value(entry.value, depth + 1);
      }
    } else if (obj is List) {
      if (depth >= _maxDepth) throw _tooDeep();
      byte(6);
      count(obj.length);
      for (final item in obj) {
        value(item, depth + 1);
      }
    } else if (obj is Absent) {
      byte(0);
    } else if (obj is Ref) {
      final wire = obj.toWire();
      _room(wire.length);
      _buf.setRange(_len, _len + wire.length, wire);
      _len += wire.length;
    } else {
      throw ArgumentError.value(
        obj,
        'value',
        'cannot be converted to a guatiao value',
      );
    }
  }
}
