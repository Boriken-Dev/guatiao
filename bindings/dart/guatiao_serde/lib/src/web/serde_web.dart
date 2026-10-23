// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Reading and writing a value as JSON, TOML or YAML on the web, through
/// the serde surface of the one module the page loaded.
library;

import 'package:guatiao/wasm.dart';

import '../serde_common.dart';

export '../serde_common.dart' hide emitSymbol, parseSymbol;

// `guatiao_parse_error` on wasm32: format, line, column, then the message.
const int _messageAt = 12;
const int _parseErrorSize = _messageAt + valueSize;

/// The loaded module, carrying the serde surface, or `LibraryNotFound`
/// when it was built without `guatiao-serde`.
WasmModule library() => openSurface('guatiao_serde');

WasmModule _resolved(String symbol, String? feature) {
  final m = library();
  if (!m.provides(symbol)) {
    throw UnsupportedError(
      feature == null
          ? '${m.source} does not export $symbol'
          : "${m.source} was not built with $symbol (feature '$feature')",
    );
  }
  return m;
}

/// Parses [text] as [format] into a new [Value]. A document that does not
/// parse throws [ParseError].
Value loads(
  String text, {
  Format format = Format.json,
  int how = 0,
  Address? alloc,
}) {
  final symbol = parseSymbol(format);
  final m = _resolved(symbol, format.feature);
  final allocator = alloc ?? m.defaultAlloc;
  final out = m.alloc(valueSize);
  try {
    withScratch((s) {
      final error = s.alloc(_parseErrorSize);
      try {
        final status =
            m.call(symbol, [s.str(text), how, allocator, out, error]);
        finishParse(status, error);
      } finally {
        freeNode(error + _messageAt);
      }
    });
  } catch (_) {
    freeRoot(out);
    rethrow;
  }
  return Value.adopt(out, alloc: allocator);
}

/// Throws for a failing [status]: [ParseError] for a document that does
/// not parse, else the status's own exception. Not part of the API.
void finishParse(int status, Address error) {
  if (status == Status.ok.code) return;
  final m = WasmModule.current;
  final message = Ref.borrowed(error + _messageAt).stringOr('');
  if (status == Status.badValue.code) {
    throw ParseError(
      message,
      format: Format.values[m.u32(error)],
      line: m.u32(error + 4),
      column: m.u32(error + 8),
    );
  }
  checkStatus(status, message);
}

/// Writes [value] (a [Value] or any [Ref]) as [format]. A TOML document
/// must be a map.
String dumps(
  Ref value, {
  Format format = Format.json,
  bool pretty = false,
  int how = 0,
  Address? alloc,
}) {
  final wantsJsonPretty = pretty && format == Format.json;
  final symbol =
      wantsJsonPretty ? 'guatiao_json_emit_pretty' : emitSymbol(format);
  final m = _resolved(symbol, format.feature);
  final allocator = alloc ?? m.defaultAlloc;
  final flags = pretty && !wantsJsonPretty ? how | prettyFlag : how;
  final out = m.alloc(valueSize);
  var filled = false;
  try {
    checkStatus(m.call(symbol, [value.requireNode(), flags, allocator, out]));
    filled = true;
    return takeValue(out)! as String;
  } finally {
    if (!filled) freeRoot(out);
  }
}
