// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'value.dart';

/// A text format `guatiao_serde` reads and writes.
enum Format {
  json,
  toml,
  yaml;

  /// The build feature the format needs, or `null` when every build has
  /// it.
  String? get feature => this == Format.json ? null : name;
}

/// How a byte string is spelled where the format has none: the low byte
/// of `how`.
const int bytesDataUri = GUATIAO_BYTES_DATA_URI;
const int bytesBase64 = GUATIAO_BYTES_BASE64;
const int bytesArray = GUATIAO_BYTES_ARRAY;
const int bytesRefuse = GUATIAO_BYTES_REFUSE;

/// Turns a `data:;base64,...` string back into bytes when reading.
const int readDataUris = GUATIAO_READ_DATA_URIS;

/// Indents the output for a person to read. JSON only; ignored elsewhere.
const int prettyFlag = GUATIAO_PRETTY;

String _parseSymbol(Format format) => 'guatiao_${format.name}_parse';

String _emitSymbol(Format format) => 'guatiao_${format.name}_emit';

native.NativeLib _resolved(String symbol, String? feature) {
  final lib = native.serdeLib();
  if (!lib.provides(symbol)) {
    throw UnsupportedError(
      feature == null
          ? '${lib.path} does not export $symbol'
          : "${lib.path} was not built with $symbol (feature '$feature')",
    );
  }
  return lib;
}

/// Parses `text` as `format` into a new [Value].
Value loads(
  String text, {
  Format format = Format.json,
  int how = 0,
  Pointer<guatiao_alloc>? alloc,
}) {
  final symbol = _parseSymbol(format);
  final lib = _resolved(symbol, format.feature);
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  try {
    using((arena) {
      final view = strOf(arena, text).ref;
      final status = switch (format) {
        Format.json =>
          lib.bindings.guatiao_json_parse(view, how, allocator, out),
        Format.toml =>
          lib.bindings.guatiao_toml_parse(view, how, allocator, out),
        Format.yaml =>
          lib.bindings.guatiao_yaml_parse(view, how, allocator, out),
      };
      checkStatus(status);
    });
  } catch (_) {
    native.core().bindings.guatiao_value_free(out);
    calloc.free(out);
    rethrow;
  }
  return Value.adopt(out, alloc: allocator);
}

/// Writes `value` as `format`. A TOML document must be a map.
String dumps(
  Value value, {
  Format format = Format.json,
  bool pretty = false,
  int how = 0,
  Pointer<guatiao_alloc>? alloc,
}) {
  // JSON has a dedicated pretty entry point; the other formats take the
  // flag and ignore it.
  final wantsJsonPretty = pretty && format == Format.json;
  final symbol =
      wantsJsonPretty ? 'guatiao_json_emit_pretty' : _emitSymbol(format);
  final lib = _resolved(symbol, format.feature);
  final allocator = alloc ?? lib.alloc;
  final flags = pretty && !wantsJsonPretty ? how | prettyFlag : how;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    final node = value.pointer;
    final status = wantsJsonPretty
        ? lib.bindings.guatiao_json_emit_pretty(node, flags, allocator, out)
        : switch (format) {
            Format.json =>
              lib.bindings.guatiao_json_emit(node, flags, allocator, out),
            Format.toml =>
              lib.bindings.guatiao_toml_emit(node, flags, allocator, out),
            Format.yaml =>
              lib.bindings.guatiao_yaml_emit(node, flags, allocator, out),
          };
    checkStatus(status);
    filled = true;
    return takeValue(out)! as String;
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}
