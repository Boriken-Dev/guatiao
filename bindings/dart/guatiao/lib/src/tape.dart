// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The wire encoding read into plain Dart and written from it in Dart,
// with no library: one crossing carries a whole document as bytes, where
// walking a tree costs a crossing per node. Read bytes come from a
// library that checked them as it wrote them, so reading guards against
// nothing worse than a cut-short body.

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
  final reader = _Reader(data, numbers);
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

final class _Reader {
  _Reader(this.data, this.numbers);

  final Uint8List data;
  final Numbers numbers;
  int at = 0;

  int count() {
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

  Uint8List body() {
    final n = count();
    if (at + n > data.length) throw RangeError('cut short');
    final out = Uint8List.sublistView(data, at, at + n);
    at += n;
    return out;
  }

  String text() => utf8.decode(body());

  Object? value(int depth) {
    final tag = data[at++];
    switch (tag) {
      case 4:
        return text();
      case 3:
        return _number(body());
      case 7:
        final n = count();
        if (n > 0 && depth >= _maxDepth) throw _tooDeep();
        final map = <String, Object?>{};
        for (var i = 0; i < n; i++) {
          final key = text();
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
        return data[at++] == 1;
      case 5:
        return Uint8List.fromList(body());
      case 0:
        return absent;
    }
    throw BadValue(Status.badValue.code, 'unknown tag $tag at ${at - 1}');
  }

  Object _number(Uint8List text) {
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
