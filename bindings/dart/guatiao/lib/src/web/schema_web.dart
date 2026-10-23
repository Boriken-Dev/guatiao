// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import '../status.dart';
import 'module.dart';
import 'value_web.dart';

/// A copy of [value] with every secret replaced by [placeholder]; see the
/// VM's `redact`, which this matches.
Value redact(
  Object? schema,
  Object? value, {
  String placeholder = '[redacted]',
  Address? alloc,
}) {
  final m = WasmModule.current;
  final allocator = alloc ?? m.defaultAlloc;
  final built = <Value>[];
  Address nodeOf(Object? item) {
    if (item is Ref) return item.requireNode();
    final v = Value.fromDart(item);
    built.add(v);
    return v.pointer;
  }

  final out = m.alloc(valueSize);
  try {
    final schemaNode = nodeOf(schema);
    final valueNode = nodeOf(value);
    withScratch(
      (s) => checkStatus(
        m.call('guatiao_schema_redact', [
          schemaNode,
          valueNode,
          s.str(placeholder),
          allocator,
          out,
        ]),
      ),
    );
  } catch (_) {
    freeRoot(out);
    rethrow;
  } finally {
    for (final v in built) {
      v.close();
    }
  }
  return Value.adopt(out, alloc: allocator);
}
