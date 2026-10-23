// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Forms, paths and the flat store on the web, through the intake surface
/// of the one module the page loaded. The same functions as on the VM.
library;

import 'package:guatiao/wasm.dart';

export '../intake_common.dart';

/// The loaded module, carrying the intake surface, or `LibraryNotFound`
/// when it was built without `guatiao-intake`.
WasmModule library() => openSurface('guatiao_intake');

/// Calls [name] with the arguments [args] builds, its last an out slot,
/// and answers the slot as plain Dart; what the call wrote is freed on
/// every path.
Object? _answer(String name, List<Object> Function(Scratch s) args) {
  final m = library();
  final out = m.alloc(valueSize);
  var filled = false;
  try {
    withScratch((s) => checkStatus(m.call(name, [...args(s), out])));
    filled = true;
    return takeValue(out);
  } finally {
    if (!filled) freeRoot(out);
  }
}

/// `null` when `form` fits `schema`, otherwise the error map
/// `guatiao_intake_check` writes.
Map<String, Object?>? check(Ref schema, Ref form, {Address? alloc}) {
  final m = library();
  final allocator = alloc ?? m.defaultAlloc;
  final out = m.alloc(valueSize);
  var filled = false;
  try {
    final status = m.call('guatiao_intake_check',
        [schema.requireNode(), form.requireNode(), allocator, out]);
    if (status == Status.ok.code) return null;
    // A form that does not fit is the answer, not a failure: only that
    // one status carries the report.
    if (status != Status.badValue.code) checkStatus(status);
    filled = true;
    return (takeValue(out)! as Map).cast<String, Object?>();
  } finally {
    if (!filled) freeRoot(out);
  }
}

/// The schema's fields grouped into sections and put in order.
List<Map<String, Object?>> layout(Ref schema, Ref form, {Address? alloc}) {
  final allocator = alloc ?? library().defaultAlloc;
  final sections = _answer('guatiao_intake_layout',
      (s) => [schema.requireNode(), form.requireNode(), allocator]);
  return <Map<String, Object?>>[
    for (final section in sections! as List)
      (section! as Map).cast<String, Object?>(),
  ];
}

/// Whether the field under `key` is shown, given the `values` entered so
/// far.
bool isVisible(Ref schema, Ref form, String key, Ref values) {
  final m = library();
  return withScratch((s) {
    final out = s.alloc(1, 1);
    checkStatus(m.call('guatiao_intake_is_visible', [
      schema.requireNode(),
      form.requireNode(),
      s.str(key),
      values.requireNode(),
      out,
    ]));
    return m.u8(out) != 0;
  });
}

/// The value at [path] inside [value], or `null`: a read-only view that
/// borrows from [value] and is found again on every access.
Ref? at(Ref value, String path) {
  final m = library();
  Address find() => withScratch((s) => m.callAddress(
      'guatiao_intake_path_get', [value.requireNode(), s.str(path)]));
  if (find().isNull) return null;
  return Ref.borrowedFrom(value, find);
}

/// The form [schema] implies, for when nobody wrote one. The caller owns
/// what comes back and closes it.
Value forSchema(Ref schema, {Address? alloc}) {
  final m = library();
  final allocator = alloc ?? m.defaultAlloc;
  final out = m.alloc(valueSize);
  try {
    checkStatus(m.call(
        'guatiao_intake_for_schema', [schema.requireNode(), allocator, out]));
  } catch (_) {
    freeRoot(out);
    rethrow;
  }
  return Value.adopt(out, alloc: allocator);
}

/// The form to show the field at [path] with, or `null`. The caller owns
/// what comes back and closes it.
Value? formFor(Ref form, Ref schema, String path, {Address? alloc}) {
  final m = library();
  final allocator = alloc ?? m.defaultAlloc;
  final out = m.alloc(valueSize);
  var filled = false;
  try {
    withScratch((s) => checkStatus(m.call('guatiao_intake_form_for', [
          form.requireNode(),
          schema.requireNode(),
          s.str(path),
          allocator,
          out,
        ])));
    // Nothing to show it with is an answer, not a failure: the slot keeps
    // the absent marker the boundary wrote.
    if (m.u32(out) == Tag.absent.code) return null;
    filled = true;
    return Value.adopt(out, alloc: allocator);
  } finally {
    if (!filled) freeRoot(out);
  }
}

/// The schema field governing the flat [key], or `null`: a read-only
/// view that borrows from [schema] and is found again on every access.
Ref? resolve(Ref schema, String key) {
  final m = library();
  Address find() => withScratch((s) => m.callAddress(
      'guatiao_intake_resolve', [schema.requireNode(), s.str(key)]));
  if (find().isNull) return null;
  return Ref.borrowedFrom(schema, find);
}

/// The flat keys the field [key] projects onto. Throws [WrongKind] when
/// [key] names no field.
List<String> flatKeys(Ref schema, String key, {Address? alloc}) {
  final allocator = alloc ?? library().defaultAlloc;
  final keys = _answer('guatiao_intake_flat_keys',
      (s) => [schema.requireNode(), s.str(key), allocator]);
  return <String>[for (final item in keys! as List) item! as String];
}

/// [value] as the flat store of `key -> text` the field [key] keeps.
Map<String, String> flatten(Ref schema, String key, Ref value,
    {Address? alloc}) {
  final allocator = alloc ?? library().defaultAlloc;
  final store = _answer(
      'guatiao_intake_flatten',
      (s) =>
          [schema.requireNode(), s.str(key), value.requireNode(), allocator]);
  return <String, String>{
    for (final entry in (store! as Map).entries)
      entry.key as String: entry.value! as String,
  };
}

/// The value the flat store [flat] holds for the field [key], the reverse
/// of [flatten]; the caller owns what comes back and closes it. `null`
/// when nothing of the field is stored; [ArgumentError] for a store that
/// is not text to text.
Value? unflatten(Ref schema, String key, Object flat, {Address? alloc}) {
  final m = library();
  final allocator = alloc ?? m.defaultAlloc;
  Value? owned;
  final Ref store;
  if (flat is Ref) {
    final plain = flat.toDart();
    if (plain is! Map || !plain.values.every((v) => v is String)) {
      throw ArgumentError.value(
          flat, 'flat', 'a flat store maps String to String');
    }
    store = flat;
  } else if (flat is Map &&
      flat.entries.every((e) => e.key is String && e.value is String)) {
    store = owned = Value.fromDart(flat, alloc: allocator);
  } else {
    throw ArgumentError.value(
        flat, 'flat', 'a flat store maps String to String');
  }
  final out = m.alloc(valueSize);
  var filled = false;
  try {
    final status = withScratch((s) => m.call('guatiao_intake_unflatten', [
          schema.requireNode(),
          s.str(key),
          store.requireNode(),
          allocator,
          out,
        ]));
    if (status == Status.wrongKind.code) return null;
    checkStatus(status);
    filled = true;
    return Value.adopt(out, alloc: allocator);
  } finally {
    if (!filled) freeRoot(out);
    owned?.close();
  }
}
