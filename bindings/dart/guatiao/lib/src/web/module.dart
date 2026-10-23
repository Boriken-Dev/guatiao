// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import '../status.dart';

/// An address in the module's linear memory: a `guatiao_value *`, a
/// `guatiao_alloc *`, or any other pointer the C surface takes. Zero is
/// null.
extension type const Address(int value) implements Object {
  /// The null address.
  static const Address none = Address(0);

  bool get isNull => value == 0;

  /// The address [bytes] further on.
  Address operator +(int bytes) => Address(value + bytes);
}

/// A 64-bit integer argument: the module's `uint64_t`/`int64_t`
/// parameters take a JavaScript BigInt.
final class I64 {
  const I64(this.value);

  final int value;
}

// The wasm32 layout of the structs read and written here: four-byte
// pointers and `size_t`, every struct aligned to its widest field.

/// Bytes in a pointer or a `size_t`.
const int wordSize = 4;

/// `sizeof(guatiao_value)`: a tag, a reserved word, then a payload of
/// four words.
const int valueSize = 8 + 4 * wordSize;

/// `sizeof(guatiao_entry)`: an owned key string, then the value.
const int entrySize = 4 * wordSize + valueSize;

/// Where an entry's value starts.
const int entryValueAt = 4 * wordSize;

/// `sizeof(guatiao_alloc)`: a `uint32_t` and four pointers.
const int allocSize = 4 + 4 * wordSize;

/// `sizeof(guatiao_str)` and `sizeof(guatiao_bytes)`.
const int viewSize = 2 * wordSize;

@JS('WebAssembly.instantiate')
external JSPromise<JSObject> _instantiate(JSUint8Array bytes, JSObject imports);

@JS('fetch')
external JSPromise<JSObject> _fetch(JSString url);

@JS('BigInt')
external JSBigInt _bigInt(JSString text);

/// No module carries this surface: none is loaded, or the loaded one was
/// built without it.
class LibraryNotFound implements Exception {
  LibraryNotFound(this.basename, this.filename, [this.cause, this.variable]);

  final String basename;

  /// The module's filename when each surface is its own build.
  final String filename;

  final Object? cause;

  /// Always `null` on the web: a page has no environment.
  final String? variable;

  @override
  String toString() {
    final tail = cause == null ? '' : ' ($cause)';
    return 'LibraryNotFound: no loaded module carries $basename: load one '
        'with loadModule(url) or loadModuleBytes(bytes)$tail';
  }
}

/// [name] is not exported by the loaded module.
class MissingSymbol implements Exception {
  MissingSymbol(this.path, this.name, [this.feature]);

  final String path;
  final String name;
  final String? feature;

  @override
  String toString() {
    final where = feature == null ? '' : " (feature '$feature')";
    return 'MissingSymbol: $path was not built with $name$where';
  }
}

/// One instantiated WebAssembly module carrying guatiao's C surfaces:
/// the core's, and the serde and intake ones when it was built with them.
///
/// A page loads one, and every package uses it: each module has a memory
/// of its own, and a value built in one cannot be read through another.
final class WasmModule {
  WasmModule._(this.source, this._exports)
      : _memory = _exports.getProperty<JSObject>('memory'.toJS);

  /// Where the module came from, for an error message.
  final String source;

  final JSObject _exports;
  final JSObject _memory;
  final Map<String, JSFunction?> _functions = <String, JSFunction?>{};
  ByteData? _view;
  Address? _alloc;

  static WasmModule? _loaded;

  /// The loaded module, or [LibraryNotFound] when there is none.
  static WasmModule get current =>
      _loaded ?? (throw LibraryNotFound('guatiao', 'guatiao.wasm'));

  /// The loaded module, or `null`.
  static WasmModule? get loaded => _loaded;

  static void forget() => _loaded = null;

  static Future<WasmModule> fromBytes(List<int> bytes, String source) async {
    final data = bytes is Uint8List ? bytes : Uint8List.fromList(bytes);
    final JSObject result;
    try {
      result = await _instantiate(data.toJS, JSObject()).toDart;
    } on Object catch (error) {
      throw ArgumentError.value(source, 'module',
          'is not a WebAssembly module guatiao can use ($error)');
    }
    final exports = result
        .getProperty<JSObject>('instance'.toJS)
        .getProperty<JSObject>('exports'.toJS);
    bool isFunction(String name) =>
        exports.getProperty<JSAny?>(name.toJS)?.typeofEquals('function') ??
        false;
    if (!isFunction('guatiao_value_free') ||
        !isFunction('guatiao_memory_alloc') ||
        !exports.has('memory')) {
      throw ArgumentError.value(
        source,
        'module',
        'exports no guatiao surface (looked for guatiao_value_free, '
            'guatiao_memory_alloc and its memory)',
      );
    }
    return _loaded = WasmModule._(source, exports);
  }

  static Future<WasmModule> fromUrl(Uri url) async {
    final response = await _fetch(url.toString().toJS).toDart;
    if (!response.getProperty<JSBoolean>('ok'.toJS).toDart) {
      final status = response.getProperty<JSNumber>('status'.toJS).toDartInt;
      throw ArgumentError.value('$url', 'url', 'answered HTTP $status');
    }
    final buffer = await response
        .callMethod<JSPromise<JSArrayBuffer>>('arrayBuffer'.toJS)
        .toDart;
    return fromBytes(buffer.toDart.asUint8List(), '$url');
  }

  JSFunction? _function(String name) => _functions.putIfAbsent(name, () {
        final found = _exports.getProperty<JSAny?>(name.toJS);
        return found != null && found.typeofEquals('function')
            ? found as JSFunction
            : null;
      });

  /// Whether the module exports [name].
  bool provides(String name) => _function(name) != null;

  /// Throws [Unsupported] unless the module exports [name]; [feature]
  /// is the build feature that brings it.
  void require(String name, {String? feature}) {
    if (provides(name)) return;
    throw Unsupported(
      Status.unsupported.code,
      feature == null
          ? '$source does not export $name'
          : "$source was not built with $name (feature '$feature')",
    );
  }

  /// Calls the export [name]. An [int] or an [Address] goes as a 32-bit
  /// integer, an [I64] as a BigInt, a [bool] as 0 or 1.
  /// Answers the 32-bit result as the module returned it, or 0 for none.
  ///
  /// Any call may grow the memory, which detaches the buffer every view
  /// was made over: the view is dropped here and made again on next use.
  int call(String name, List<Object> args) {
    final f = _function(name);
    if (f == null) {
      throw Unsupported(
          Status.unsupported.code, '$source does not export $name');
    }
    final js = <JSAny?>[null];
    for (final a in args) {
      js.add(switch (a) {
        final int n => n.toJS,
        final bool b => (b ? 1 : 0).toJS,
        final I64 n => _bigInt(n.value.toString().toJS),
        _ => throw ArgumentError.value(
            a, 'args', 'not an int, Address, I64 or bool'),
      });
    }
    try {
      final result = f.callMethodVarArgs<JSAny?>('call'.toJS, js);
      return result == null || !result.typeofEquals('number')
          ? 0
          : (result as JSNumber).toDartInt;
    } finally {
      _view = null;
    }
  }

  /// A call answering a pointer, read as unsigned.
  Address callAddress(String name, List<Object> args) =>
      Address(call(name, args).toUnsigned(32));

  /// The module's memory as it is now. Never keep it across a [call].
  ByteData get data => _view ??=
      ByteData.view(_memory.getProperty<JSArrayBuffer>('buffer'.toJS).toDart);

  int u8(Address at) => data.getUint8(at.value);
  int u32(Address at) => data.getUint32(at.value, Endian.little);
  int i32(Address at) => data.getInt32(at.value, Endian.little);
  Address word(Address at) => Address(u32(at));

  /// A 64-bit integer: exact on dart2wasm and the VM, a double's
  /// precision under dart2js, whose integers are JavaScript numbers.
  int i64(Address at) {
    final low = u32(at);
    final high = data.getInt32(at.value + 4, Endian.little);
    return _jsNumbers ? high * 4294967296 + low : (high << 32) | low;
  }

  /// A `uint64_t`, as Dart's `int` carries one: past 2^63 negative.
  int u64(Address at) => i64(at);

  void setU8(Address at, int v) => data.setUint8(at.value, v);
  void setU32(Address at, int v) =>
      data.setUint32(at.value, v.toUnsigned(32), Endian.little);
  void setI32(Address at, int v) => data.setInt32(at.value, v, Endian.little);
  void setWord(Address at, Address v) => setU32(at, v.value);

  void setI64(Address at, int v) {
    if (_jsNumbers) {
      final big = BigInt.from(v);
      setU32(at, (big & BigInt.from(0xffffffff)).toInt());
      setU32(at + 4, ((big >> 32) & BigInt.from(0xffffffff)).toInt());
    } else {
      setU32(at, v & 0xffffffff);
      setU32(at + 4, (v >> 32) & 0xffffffff);
    }
  }

  /// A copy of [length] bytes at [at].
  Uint8List bytes(Address at, int length) {
    if (length == 0 || at.isNull) return Uint8List(0);
    return Uint8List.fromList(data.buffer.asUint8List(at.value, length));
  }

  void setBytes(Address at, List<int> source) {
    if (source.isEmpty) return;
    data.buffer.asUint8List(at.value, source.length).setAll(0, source);
  }

  /// Zeroed memory from the module's heap, or [AllocFailed].
  Address alloc(int size, [int align = 8]) {
    final at = callAddress('guatiao_memory_alloc', [size, align]);
    if (at.isNull) throw AllocFailed(Status.allocFailed.code);
    return at;
  }

  void free(Address at, int size, [int align = 8]) {
    if (at.isNull) return;
    call('guatiao_memory_free', [at, size, align]);
  }

  /// The module's default allocator, filled by `guatiao_alloc_default`
  /// once and kept for the module's life: every container built through
  /// it holds its address.
  Address get defaultAlloc {
    final cached = _alloc;
    if (cached != null) return cached;
    final at = alloc(allocSize, wordSize);
    checkStatus(call('guatiao_alloc_default', [at]));
    return _alloc = at;
  }
}

/// `true` under dart2js, where an `int` is a JavaScript number.
const bool _jsNumbers = identical(1, 1.0);

/// Memory taken for one call's arguments and released together.
final class Scratch {
  Scratch(this.module);

  final WasmModule module;
  final List<(Address, int, int)> _taken = <(Address, int, int)>[];

  /// [size] zeroed bytes, released with the rest.
  Address alloc(int size, [int align = 8]) {
    final at = module.alloc(size, align);
    _taken.add((at, size, align));
    return at;
  }

  /// A `guatiao_str`/`guatiao_bytes` over a copy of [data].
  Address view(List<int> data) {
    final at = alloc(viewSize, wordSize);
    if (data.isNotEmpty) {
      final buffer = alloc(data.length, 1);
      module.setBytes(buffer, data);
      module.setWord(at, buffer);
      module.setU32(at + wordSize, data.length);
    }
    return at;
  }

  /// A `guatiao_str` over a copy of [text]'s UTF-8 bytes.
  Address str(String text) => view(utf8.encode(text));

  /// An empty `guatiao_value` slot, ABSENT.
  Address value() => alloc(valueSize);

  void release() {
    for (final (at, size, align) in _taken.reversed) {
      module.free(at, size, align);
    }
    _taken.clear();
  }
}

/// Runs [body] with a [Scratch] over the loaded module, released after.
T withScratch<T>(T Function(Scratch scratch) body) {
  final scratch = Scratch(WasmModule.current);
  try {
    return body(scratch);
  } finally {
    scratch.release();
  }
}
