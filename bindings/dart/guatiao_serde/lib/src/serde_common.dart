// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The constants are `guatiao_serde.h`'s, written out so the web needs no
// generated bindings; `test/serde_test.dart` compares them on the VM.

import 'package:guatiao/guatiao.dart' show BadValue, Status;

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
const int bytesDataUri = 0;
const int bytesBase64 = 1;
const int bytesArray = 2;
const int bytesRefuse = 3;

/// Turns a `data:;base64,...` string back into bytes when reading.
const int readDataUris = 1 << 8;

/// Indents the output for a person to read. JSON only; ignored elsewhere.
const int prettyFlag = 1 << 9;

/// A document that does not parse, and where it stopped.
///
/// It is a [BadValue] (what a malformed document threw before) and a
/// [FormatException], so either catch finds it. [format] is the format
/// that refused the document; [line] and [column] count from 1 (the
/// column in characters) and are 0 when the parser did not say where,
/// and for text that is not UTF-8 they name its first bad byte.
/// [message] says what the parser expected and never quotes the
/// document, so [source] and [offset] are `null`.
class ParseError extends BadValue implements FormatException {
  ParseError(
    String message, {
    required this.format,
    required this.line,
    required this.column,
  }) : super(Status.badValue.code, message);

  final Format format;
  final int line;
  final int column;

  @override
  dynamic get source => null;

  @override
  int? get offset => null;

  @override
  String toString() => line == 0
      ? 'ParseError (${format.name}): $message'
      : 'ParseError (${format.name}): $message at line $line, column $column';
}

/// The entry point that reads [format], `_ex` for where it stopped.
String parseSymbol(Format format) => 'guatiao_${format.name}_parse_ex';

/// The entry point that writes [format].
String emitSymbol(Format format) => 'guatiao_${format.name}_emit';

/// The entry point that reads [format] into wire bytes.
String toWireSymbol(Format format) => 'guatiao_${format.name}_to_wire';

/// The entry point that writes wire bytes as [format].
String fromWireSymbol(Format format) => 'guatiao_${format.name}_from_wire';
