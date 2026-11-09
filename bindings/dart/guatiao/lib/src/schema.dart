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

/// Where [value] differs from [baseline], or from the schema's defaults
/// when [baseline] is null.
///
/// A list of maps `{path: [segments], key: 'tls.ca', value: .., default:
/// ..}`; `value` and `default` are left out where that side has nothing.
/// An object's declared fields, an open map's keys and the fields of a
/// variant that kept its arm are places of their own; anything else (a
/// number, text, a list, a tuple such as a duration, a variant that
/// changed arm) is one place, compared whole. A field a side leaves out
/// reads as its schema default. A secret that differs is listed with
/// [placeholder] on each side, never its value. Local `$ref`s are
/// followed.
///
/// [schema], [value] and [baseline] are [Ref]s or [Value]s, or what
/// [Value.fromDart] takes. The result is a new [Value] the caller closes.
/// Throws [WrongKind] for a schema that is not a map.
Value differences(
  Object? schema,
  Object? value, {
  Object? baseline,
  String placeholder = '***',
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
    final baselineNode = baseline == null ? nullptr : nodeOf(baseline);
    using((arena) {
      checkStatus(
        lib.bindings.guatiao_schema_differences(
          schemaNode,
          valueNode,
          baselineNode,
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
