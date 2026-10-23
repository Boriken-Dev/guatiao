// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
import 'package:guatiao/ffi.dart';
import 'package:guatiao/guatiao.dart' show Numbers;

import 'bindings.g.dart'
    hide guatiao_alloc, guatiao_bytes, guatiao_str, guatiao_value;
import 'serde_common.dart';

export 'serde_common.dart'
    hide emitSymbol, fromWireSymbol, parseSymbol, toWireSymbol;

/// The `guatiao_serde` library, opened on first use: the one registered
/// for it with `useLibrary` (package:guatiao), else
/// `GUATIAO_SERDE_LIBRARY`, else `GUATIAO_LIBRARY` (a directory holding
/// it, or a file exporting its functions or sitting beside it), else
/// `libraryDirectory`, else the platform's search. Throws
/// `LibraryNotFound`.
NativeLib library() => openSurface('guatiao_serde');

GuatiaoSerdeBindings? _bindings;
NativeLib? _bound;

GuatiaoSerdeBindings _bindingsOf(NativeLib lib) {
  final cached = _bindings;
  if (cached != null && identical(_bound, lib)) return cached;
  _bound = lib;
  return _bindings = GuatiaoSerdeBindings(lib.library);
}

NativeLib _resolved(String symbol, String? feature) {
  final lib = library();
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
///
/// A document that does not parse throws [ParseError].
Value loads(
  String text, {
  Format format = Format.json,
  int how = 0,
  Pointer<guatiao_alloc>? alloc,
}) {
  final symbol = parseSymbol(format);
  final lib = _resolved(symbol, format.feature);
  final serde = _bindingsOf(lib);
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  final error = calloc<guatiao_parse_error>();
  try {
    using((arena) {
      final view = strOf(arena, text).ref;
      final status = switch (format) {
        Format.json => serde.guatiao_json_parse_ex(
          view,
          how,
          allocator,
          out,
          error,
        ),
        Format.toml => serde.guatiao_toml_parse_ex(
          view,
          how,
          allocator,
          out,
          error,
        ),
        Format.yaml => serde.guatiao_yaml_parse_ex(
          view,
          how,
          allocator,
          out,
          error,
        ),
      };
      finishParse(status, error);
    });
  } catch (_) {
    freeNode(out);
    calloc.free(out);
    rethrow;
  } finally {
    _releaseMessage(error);
    calloc.free(error);
  }
  return Value.adopt(out, alloc: allocator);
}

/// Throws for a failing [status]: a [ParseError] for a document that does
/// not parse, any other refusal as its own exception with the message the
/// library gave it. Not part of the package's API.
void finishParse(int status, Pointer<guatiao_parse_error> error) {
  if (status == Status.ok.code) return;
  final message = _takeMessage(error);
  if (status == Status.badValue.code) {
    throw ParseError(
      message,
      format: Format.values[error.ref.format],
      line: error.ref.line,
      column: error.ref.column,
    );
  }
  checkStatus(status, message);
}

/// `guatiao_parse_error.message`, the last field: it ends the struct, so
/// it starts where the struct's size less a value's does.
Pointer<guatiao_value> _messageOf(Pointer<guatiao_parse_error> error) =>
    (error.cast<Uint8>() +
            (sizeOf<guatiao_parse_error>() - sizeOf<guatiao_value>()))
        .cast<guatiao_value>();

String _takeMessage(Pointer<guatiao_parse_error> error) =>
    Ref.borrowed(_messageOf(error)).stringOr('');

void _releaseMessage(Pointer<guatiao_parse_error> error) =>
    freeNode(_messageOf(error));

/// Writes `value` (a [Value] or any [Ref]) as `format`. A TOML document
/// must be a map.
String dumps(
  Ref value, {
  Format format = Format.json,
  bool pretty = false,
  int how = 0,
  Pointer<guatiao_alloc>? alloc,
}) {
  // JSON has a dedicated pretty entry point; the other formats take the
  // flag and ignore it.
  final wantsJsonPretty = pretty && format == Format.json;
  final symbol = wantsJsonPretty
      ? 'guatiao_json_emit_pretty'
      : emitSymbol(format);
  final lib = _resolved(symbol, format.feature);
  final serde = _bindingsOf(lib);
  final allocator = alloc ?? lib.alloc;
  final flags = pretty && !wantsJsonPretty ? how | prettyFlag : how;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    final node = value.requireNode();
    final status = wantsJsonPretty
        ? serde.guatiao_json_emit_pretty(node, flags, allocator, out)
        : switch (format) {
            Format.json => serde.guatiao_json_emit(node, flags, allocator, out),
            Format.toml => serde.guatiao_toml_emit(node, flags, allocator, out),
            Format.yaml => serde.guatiao_yaml_emit(node, flags, allocator, out),
          };
    checkStatus(status);
    filled = true;
    return takeValue(out)! as String;
  } finally {
    if (!filled) {
      freeNode(out);
      calloc.free(out);
    }
  }
}

/// Parses [text] as [format] straight into plain Dart, as
/// `loads(text, format: format, how: how).toDart(numbers: numbers)`
/// answers it, with no [Value] built: the document crosses once, as wire
/// bytes, and is read in Dart. Throws as [loads] does.
Object? parse(
  String text, {
  Format format = Format.json,
  int how = 0,
  Numbers numbers = Numbers.auto,
}) {
  final lib = _resolved(toWireSymbol(format), format.feature);
  final serde = _bindingsOf(lib);
  final out = calloc<guatiao_value>();
  final error = calloc<guatiao_parse_error>();
  try {
    using((arena) {
      final view = strOf(arena, text).ref;
      final status = switch (format) {
        Format.json => serde.guatiao_json_to_wire(
          view,
          how,
          lib.alloc,
          out,
          error,
        ),
        Format.toml => serde.guatiao_toml_to_wire(
          view,
          how,
          lib.alloc,
          out,
          error,
        ),
        Format.yaml => serde.guatiao_yaml_to_wire(
          view,
          how,
          lib.alloc,
          out,
          error,
        ),
      };
      finishParse(status, error);
    });
  } catch (_) {
    freeNode(out);
    calloc.free(out);
    rethrow;
  } finally {
    _releaseMessage(error);
    calloc.free(error);
  }
  return wireToDart(takeValue(out)! as Uint8List, numbers: numbers);
}

/// Writes the plain Dart [obj] as [format], as
/// `dumps(Value.fromDart(obj), ...)` writes it, with no [Value] built: it
/// is written as wire bytes in Dart and crosses once. Takes what
/// [Value.fromDart] takes and throws as it does; a TOML document must be
/// a map.
String render(
  Object? obj, {
  Format format = Format.json,
  bool pretty = false,
  int how = 0,
}) {
  final lib = _resolved(fromWireSymbol(format), format.feature);
  final serde = _bindingsOf(lib);
  final flags = pretty ? how | prettyFlag : how;
  final data = dartToWire(obj);
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    using((arena) {
      final view = arena<guatiao_bytes>();
      if (data.isNotEmpty) {
        final buffer = arena<Uint8>(data.length);
        buffer.asTypedList(data.length).setAll(0, data);
        view.ref
          ..ptr = buffer
          ..len = data.length;
      }
      final status = switch (format) {
        Format.json => serde.guatiao_json_from_wire(
          view.ref,
          flags,
          lib.alloc,
          out,
        ),
        Format.toml => serde.guatiao_toml_from_wire(
          view.ref,
          flags,
          lib.alloc,
          out,
        ),
        Format.yaml => serde.guatiao_yaml_from_wire(
          view.ref,
          flags,
          lib.alloc,
          out,
        ),
      };
      checkStatus(status);
    });
    filled = true;
    return takeValue(out)! as String;
  } finally {
    if (!filled) {
      freeNode(out);
      calloc.free(out);
    }
  }
}
