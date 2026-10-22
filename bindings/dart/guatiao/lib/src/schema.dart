// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'value.dart';

/// A copy of [value] with every secret replaced by [placeholder].
///
/// A secret is a field whose schema sets `x-sensitive` or `writeOnly`, at
/// any depth: in a nested object, in every element of a list, in a
/// variant's arm. Where the schema stops describing the value (a key it
/// does not declare, say) the value is hidden too, since nothing says it
/// is not a secret. Local `$ref`s are followed.
///
/// [schema] and [value] are [Ref]s or [Value]s, or what [Value.fromDart]
/// takes. The copy is a new [Value] the caller closes. Throws [WrongKind]
/// for a schema that is not a map.
Value redact(
  Object? schema,
  Object? value, {
  String placeholder = '[redacted]',
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.core();
  final allocator = alloc ?? lib.alloc;
  final built = <Value>[];
  Pointer<guatiao_value> nodeOf(Object? item) {
    if (item is Ref) return item.requireNode();
    final v = Value.fromDart(item);
    built.add(v);
    return v.pointer;
  }

  final out = calloc<guatiao_value>();
  try {
    final schemaNode = nodeOf(schema);
    final valueNode = nodeOf(value);
    using((arena) {
      checkStatus(
        lib.bindings.guatiao_schema_redact(
          schemaNode,
          valueNode,
          strOf(arena, placeholder).ref,
          allocator,
          out,
        ),
      );
    });
  } catch (_) {
    lib.bindings.guatiao_value_free(out);
    calloc.free(out);
    rethrow;
  } finally {
    for (final v in built) {
      v.close();
    }
  }
  return Value.adopt(out, alloc: allocator);
}
