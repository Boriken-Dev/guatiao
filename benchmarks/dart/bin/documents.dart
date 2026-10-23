// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A document to plain Dart and back, three ways: through a `Value`
/// (`loads(...).toDart()`, `dumps(Value.fromDart(...))`), through the
/// wire bytes as a tape decoded in Dart (`parse`, `render`), and through
/// dart:ffi callbacks (a visitor table the library calls per event, a
/// walker calling the library's visitor per event).
///
///     GUATIAO_LIBRARY=<target>/release dart run bin/documents.dart [--save NAME]
///
/// writes `benchmarks/results/NAME.json` in the runner's shape.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
import 'package:guatiao/ffi.dart';
// The raw entry points, for the callback paths the package does not take.
// ignore: implementation_imports
import 'package:guatiao_serde/src/bindings.g.dart' show GuatiaoSerdeBindings;
import 'package:guatiao_serde/guatiao_serde.dart' as serde;

// ---- parse through callbacks ------------------------------------------

final List<Object?> _stack = [];
final List<String?> _keys = [];
String? _key;
Object? _root;

int _put(Object? v) {
  if (_stack.isEmpty) {
    _root = v;
  } else {
    final top = _stack.last;
    if (top is Map<String, Object?>) {
      top[_key!] = v;
    } else {
      (top as List<Object?>).add(v);
    }
  }
  return 0;
}

int _begin(Object c) {
  _keys.add(_key);
  _stack.add(c);
  return 0;
}

int _end(Pointer<Void> _) {
  final c = _stack.removeLast();
  _key = _keys.removeLast();
  return _put(c);
}

Uint8List _view(guatiao_str s) =>
    s.len == 0 ? Uint8List(0) : s.ptr.asTypedList(s.len);

int _null(Pointer<Void> _) => _put(null);
int _bool(Pointer<Void> _, bool b) => _put(b);
int _number(Pointer<Void> _, guatiao_str s) {
  final text = String.fromCharCodes(_view(s));
  return _put(
    text.contains('.') || text.contains('e') || text.contains('E')
        ? double.parse(text)
        : int.parse(text),
  );
}

int _string(Pointer<Void> _, guatiao_str s) => _put(utf8.decode(_view(s)));
int _bytes(Pointer<Void> _, guatiao_bytes b) => _put(
  b.len == 0 ? Uint8List(0) : Uint8List.fromList(b.ptr.asTypedList(b.len)),
);
int _listBegin(Pointer<Void> _, int n) => _begin(<Object?>[]);
int _mapBegin(Pointer<Void> _, int n) => _begin(<String, Object?>{});
int _setKey(Pointer<Void> _, guatiao_str s) {
  _key = utf8.decode(_view(s));
  return 0;
}

typedef _A0 = Uint32 Function(Pointer<Void>);
typedef _AB = Uint32 Function(Pointer<Void>, Bool);
typedef _AS = Uint32 Function(Pointer<Void>, guatiao_str);
typedef _AY = Uint32 Function(Pointer<Void>, guatiao_bytes);
typedef _AN = Uint32 Function(Pointer<Void>, Uint64);

final class _Callbacks {
  _Callbacks() {
    table = calloc<guatiao_visitor>();
    table.ref
      ..struct_size = sizeOf<guatiao_visitor>()
      ..absent = Pointer.fromFunction<_A0>(_null, 7)
      ..null$ = Pointer.fromFunction<_A0>(_null, 7)
      ..boolean = Pointer.fromFunction<_AB>(_bool, 7)
      ..number = Pointer.fromFunction<_AS>(_number, 7)
      ..string = Pointer.fromFunction<_AS>(_string, 7)
      ..bytes = Pointer.fromFunction<_AY>(_bytes, 7)
      ..list_begin = Pointer.fromFunction<_AN>(_listBegin, 7)
      ..list_end = Pointer.fromFunction<_A0>(_end, 7)
      ..map_begin = Pointer.fromFunction<_AN>(_mapBegin, 7)
      ..key = Pointer.fromFunction<_AS>(_setKey, 7)
      ..map_end = Pointer.fromFunction<_A0>(_end, 7);
  }

  late final Pointer<guatiao_visitor> table;
}

Object? parseCallbacks(GuatiaoSerdeBindings b, _Callbacks c, String text) {
  _stack.clear();
  _keys.clear();
  _root = null;
  return using((arena) {
    final view = strOf(arena, text).ref;
    final lib = serde.library();
    checkStatus(b.guatiao_json_visit(view, 0, lib.alloc, c.table, nullptr));
    return _root;
  });
}

// ---- render through a walker ------------------------------------------

late Pointer<guatiao_visitor> _into;
late int Function(Pointer<Void>) _vNull;
late int Function(Pointer<Void>, bool) _vBool;
late int Function(Pointer<Void>, guatiao_str) _vNumber;
late int Function(Pointer<Void>, guatiao_str) _vString;
late int Function(Pointer<Void>, int) _vListBegin;
late int Function(Pointer<Void>) _vListEnd;
late int Function(Pointer<Void>, int) _vMapBegin;
late int Function(Pointer<Void>, guatiao_str) _vKey;
late int Function(Pointer<Void>) _vMapEnd;
Object? _walked;
final Pointer<guatiao_str> _scratchStr = calloc<guatiao_str>();
Pointer<Uint8> _scratch = calloc<Uint8>(1 << 16);
int _scratchCap = 1 << 16;

guatiao_str _str(List<int> data) {
  if (data.length > _scratchCap) {
    calloc.free(_scratch);
    _scratchCap = data.length;
    _scratch = calloc<Uint8>(_scratchCap);
  }
  _scratch.asTypedList(data.length).setAll(0, data);
  _scratchStr.ref
    ..ptr = _scratch
    ..len = data.length;
  return _scratchStr.ref;
}

void _ok(int status) {
  if (status != 0) checkStatus(status);
}

void _go(Object? o) {
  final ctx = _into.ref.ctx;
  if (o is String) {
    _ok(_vString(ctx, _str(utf8.encode(o))));
  } else if (o == null) {
    _ok(_vNull(ctx));
  } else if (o is bool) {
    _ok(_vBool(ctx, o));
  } else if (o is num) {
    _ok(_vNumber(ctx, _str(o.toString().codeUnits)));
  } else if (o is Map) {
    _ok(_vMapBegin(ctx, o.length));
    for (final e in o.entries) {
      _ok(_vKey(ctx, _str(utf8.encode(e.key as String))));
      _go(e.value);
    }
    _ok(_vMapEnd(ctx));
  } else if (o is List) {
    _ok(_vListBegin(ctx, o.length));
    for (final item in o) {
      _go(item);
    }
    _ok(_vListEnd(ctx));
  } else {
    throw ArgumentError.value(o);
  }
}

int _walk(Pointer<Void> _, Pointer<guatiao_visitor> into) {
  _into = into;
  final v = into.ref;
  _vNull = v.null$.asFunction();
  _vBool = v.boolean.asFunction();
  _vNumber = v.number.asFunction();
  _vString = v.string.asFunction();
  _vListBegin = v.list_begin.asFunction();
  _vListEnd = v.list_end.asFunction();
  _vMapBegin = v.map_begin.asFunction();
  _vKey = v.key.asFunction();
  _vMapEnd = v.map_end.asFunction();
  try {
    _go(_walked);
    return 0;
  } on GuatiaoException catch (e) {
    return e.code;
  }
}

final Pointer<guatiao_walker> _walker = () {
  final w = calloc<guatiao_walker>();
  w.ref
    ..struct_size = sizeOf<guatiao_walker>()
    ..walk =
        Pointer.fromFunction<
          Uint32 Function(Pointer<Void>, Pointer<guatiao_visitor>)
        >(_walk, 7);
  return w;
}();

String renderCallbacks(GuatiaoSerdeBindings b, Object? obj) {
  _walked = obj;
  final lib = serde.library();
  final out = calloc<guatiao_value>();
  checkStatus(b.guatiao_json_emit_walker(_walker, 0, lib.alloc, out));
  return takeValue(out)! as String;
}

// ---- the cases and the timing ------------------------------------------

Map<String, Object?> _config() => {
  'name': 'fleet',
  'servers': [
    for (var i = 0; i < 50; i++)
      {
        'host': 'node-$i.example',
        'port': 8000 + i,
        'weight': i / 7,
        'tags': ['edge', 'eu', '$i'],
        'enabled': i % 3 != 0,
        'limits': {'rps': 1000 + i, 'burst': 2.5, 'note': null},
      },
  ],
};

Map<String, Object?> _cases() => {
  'config': _config(),
  'floats_10k': [for (var i = 0; i < 10000; i++) i * 1.25 + 0.5],
  'text_1mb': {'blob': 'x' * (1 << 20)},
};

List<double> _time(Object? Function() fn, double targetMs, int samples) {
  fn();
  var n = 1;
  while (true) {
    final sw = Stopwatch()..start();
    for (var i = 0; i < n; i++) {
      fn();
    }
    if (sw.elapsedMicroseconds / 1000 >= targetMs) break;
    n *= 2;
  }
  return [
    for (var s = 0; s < samples; s++)
      () {
        final sw = Stopwatch()..start();
        for (var i = 0; i < n; i++) {
          fn();
        }
        return sw.elapsedMicroseconds / 1000 / n;
      }(),
  ];
}

double _median(List<double> xs) {
  final s = [...xs]..sort();
  final m = s.length ~/ 2;
  return s.length.isOdd ? s[m] : (s[m - 1] + s[m]) / 2;
}

void main(List<String> args) {
  String? save;
  var samples = 15;
  var targetMs = 20.0;
  for (var i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '--save':
        save = args[++i];
      case '--samples':
        samples = int.parse(args[++i]);
      case '--target-ms':
        targetMs = double.parse(args[++i]);
    }
  }
  final dir = Platform.environment['GUATIAO_LIBRARY'];
  if (dir != null) libraryDirectory = dir;
  final b = GuatiaoSerdeBindings(serde.library().library);
  final callbacks = _Callbacks();
  const eq = DeepCollectionEquality();
  final metrics = <String, Object? Function()>{};
  for (final MapEntry(key: name, value: obj) in _cases().entries) {
    final text = serde.render(obj);
    final viaValue = serde.loads(text);
    if (!eq.equals(serde.parse(text), obj) ||
        !eq.equals(viaValue.toDart(), obj) ||
        !eq.equals(parseCallbacks(b, callbacks, text), obj)) {
      throw StateError('$name: the paths read different things');
    }
    viaValue.close();
    final value = Value.fromDart(obj);
    final dumped = serde.dumps(value);
    value.close();
    if (dumped != text || renderCallbacks(b, obj) != text) {
      throw StateError('$name: the paths wrote different things');
    }
    metrics['${name}_parse_value'] = () {
      final v = serde.loads(text);
      final out = v.toDart();
      v.close();
      return out;
    };
    metrics['${name}_parse_tape'] = () => serde.parse(text);
    metrics['${name}_parse_callbacks'] = () =>
        parseCallbacks(b, callbacks, text);
    metrics['${name}_render_value'] = () {
      final v = Value.fromDart(obj);
      final out = serde.dumps(v);
      v.close();
      return out;
    };
    metrics['${name}_render_tape'] = () => serde.render(obj);
    metrics['${name}_render_callbacks'] = () => renderCallbacks(b, obj);
  }
  final result = <String, Object?>{
    'name': save ?? 'documents',
    'metrics': <String, Object?>{},
    'raw': <String, Object?>{
      'processor': Abi.current().toString(),
      'os': Platform.operatingSystem,
      'dart': Platform.version.split(' ').first,
      'samples_ms': <String, Object?>{},
    },
  };
  for (final MapEntry(key: name, value: fn) in metrics.entries) {
    final s = _time(fn, targetMs, samples);
    (result['metrics']! as Map)[name] = {
      'min_ms': s.reduce((a, b) => a < b ? a : b),
      'median_ms': _median(s),
      'max_ms': s.reduce((a, b) => a > b ? a : b),
    };
    ((result['raw']! as Map)['samples_ms'] as Map)[name] = s;
    stdout.writeln(
      '${name.padRight(32)} ${(_median(s) * 1000).toStringAsFixed(1).padLeft(12)} us',
    );
  }
  if (save != null) {
    final path = File.fromUri(
      Platform.script.resolve('../../results/$save.json'),
    );
    path.writeAsStringSync(
      '${const JsonEncoder.withIndent('  ').convert(result)}\n',
    );
    stderr.writeln('saved ${path.path}');
  }
}

/// Deep equality over what the paths answer: maps, lists, bytes, scalars.
final class DeepCollectionEquality {
  const DeepCollectionEquality();

  bool equals(Object? a, Object? b) {
    if (a is Map && b is Map) {
      if (a.length != b.length) return false;
      for (final k in a.keys) {
        if (!b.containsKey(k) || !equals(a[k], b[k])) return false;
      }
      return true;
    }
    if (a is List && b is List) {
      if (a.length != b.length) return false;
      for (var i = 0; i < a.length; i++) {
        if (!equals(a[i], b[i])) return false;
      }
      return true;
    }
    return a == b;
  }
}
