// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'value.dart';

/// `null` when `form` fits `schema`, otherwise the error map
/// `guatiao_intake_check` writes: `kind` and `at`, plus whichever of
/// `path`, `id`, `field`, `expected` and `message` apply.
Map<String, Object?>? check(
  Ref schema,
  Ref form, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    final status = lib.bindings.guatiao_intake_check(
      schema.requireNode(),
      form.requireNode(),
      allocator,
      out,
    );
    if (status == Status.ok.code) return null;
    // A form that does not fit is the answer, not a failure: only that
    // one status carries the report.
    if (status != Status.badValue.code) checkStatus(status);
    filled = true;
    return (takeValue(out)! as Map).cast<String, Object?>();
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}

/// The schema's fields grouped into sections and put in order.
List<Map<String, Object?>> layout(
  Ref schema,
  Ref form, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    checkStatus(
      lib.bindings.guatiao_intake_layout(
        schema.requireNode(),
        form.requireNode(),
        allocator,
        out,
      ),
    );
    filled = true;
    return <Map<String, Object?>>[
      for (final section in takeValue(out)! as List)
        (section! as Map).cast<String, Object?>(),
    ];
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}

/// Whether the field under `key` is shown, given the `values` entered so
/// far.
bool isVisible(Ref schema, Ref form, String key, Ref values) {
  final lib = native.formLib();
  final out = calloc<Bool>();
  try {
    using((arena) {
      checkStatus(
        lib.bindings.guatiao_intake_is_visible(
          schema.requireNode(),
          form.requireNode(),
          strOf(arena, key).ref,
          values.requireNode(),
          out,
        ),
      );
    });
    return out.value;
  } finally {
    calloc.free(out);
  }
}

/// The value at [path] inside [value], or `null`.
///
/// `agent[1].name[home].host`: a dot is a member, a bracket is a list
/// position or a map key, and which one a bracket means is decided by
/// what it is applied to -- so `env[PATH]` needs no quoting, while
/// `env["1"]` is a key rather than a position.
///
/// What comes back **borrows** from [value] and is read-only: it is a view
/// into that tree, not a copy. It holds [value] alive and is found again on
/// every access, so reading it after [value] is closed throws a
/// [StateError], and a change to [value] is seen as it is now. A path that
/// names nothing and a path that is not a path at all both answer `null`.
Ref? at(Ref value, String path) {
  final lib = native.formLib();
  Pointer<guatiao_value> find() => using(
        (arena) => lib.bindings.guatiao_intake_path_get(
          value.requireNode(),
          strOf(arena, path).ref,
        ),
      );
  if (find() == nullptr) return null;
  return Ref.borrowedFrom(value, find);
}

/// The form [schema] implies, for when nobody wrote one.
///
/// An object is a form: the schema already says which section each field
/// belongs to. What comes back carries one section per distinct
/// `x-section` in first-appearance order and **ids only** -- what a
/// section is called is a form's business, and a schema has no opinion --
/// plus each member's own form where there is something in it. A schema
/// that groups nothing gives an empty map, which is a complete form.
///
/// The caller owns what comes back and closes it.
Value forSchema(Ref schema, {Pointer<guatiao_alloc>? alloc}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    checkStatus(
      lib.bindings.guatiao_intake_for_schema(
        schema.requireNode(),
        allocator,
        out,
      ),
    );
    filled = true;
    return Value.adopt(out, alloc: allocator);
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}

/// The form to show the field at [path] with: the one [form] assigns, or
/// the one its schema implies.
///
/// `null` when the path names no field, or names one with no members and
/// no form assigned -- a text is shown by a control, not by a form.
///
/// The caller owns what comes back and closes it.
Value? formFor(
  Ref form,
  Ref schema,
  String path, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    using((arena) {
      checkStatus(
        lib.bindings.guatiao_intake_form_for(
          form.requireNode(),
          schema.requireNode(),
          strOf(arena, path).ref,
          allocator,
          out,
        ),
      );
    });
    // Nothing to show it with is an answer, not a failure: the slot keeps
    // the absent marker the boundary wrote.
    if (out.ref.tag == Tag.absent.code) return null;
    filled = true;
    return Value.adopt(out, alloc: allocator);
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}

/// The schema field governing the flat [key], or `null`.
///
/// Follows one level of projection, so `auth.password` answers the field
/// of that name inside the arm rather than `auth`. Everything a front end
/// shows about a field -- `title`, `required`, `x-widget`, a secret mark
/// -- is read off the node returned. It **borrows** from [schema], is
/// read-only, holds [schema] alive and is found again on every access:
/// reading it after [schema] is closed throws a [StateError].
Ref? resolve(Ref schema, String key) {
  final lib = native.formLib();
  Pointer<guatiao_value> find() => using(
        (arena) => lib.bindings.guatiao_intake_resolve(
          schema.requireNode(),
          strOf(arena, key).ref,
        ),
      );
  if (find() == nullptr) return null;
  return Ref.borrowedFrom(schema, find);
}

/// Calls a flat-projection export that fills one value, and returns that
/// value read as plain Dart. What the call wrote is freed on every path.
Object? _flat(int Function(Arena arena, Pointer<guatiao_value> out) call) {
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    checkStatus(using((arena) => call(arena, out)));
    filled = true;
    return takeValue(out);
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
  }
}

/// The flat keys the field [key] projects onto: its own, then one per
/// field of every arm. A member that is itself a container stores its
/// leaves below its key, and those are not listed. Throws [WrongKind]
/// when [key] names no field.
List<String> flatKeys(
  Ref schema,
  String key, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final keys = _flat(
    (arena, out) => lib.bindings.guatiao_intake_flat_keys(
      schema.requireNode(),
      strOf(arena, key).ref,
      allocator,
      out,
    ),
  );
  return <String>[for (final item in keys! as List) item! as String];
}

/// [value] as the flat store of `key -> text` the field [key] keeps: one
/// entry per scalar leaf (a tagged field's discriminant under its own key
/// and its payload under `key.member`, a list indexed as `agent[0].name`,
/// an open map bracketed as `env[PATH]`). An empty container stores
/// nothing. Throws [WrongKind] when [key] names no field or the value's
/// shape contradicts it.
Map<String, String> flatten(
  Ref schema,
  String key,
  Ref value, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
  final store = _flat(
    (arena, out) => lib.bindings.guatiao_intake_flatten(
      schema.requireNode(),
      strOf(arena, key).ref,
      value.requireNode(),
      allocator,
      out,
    ),
  );
  return <String, String>{
    for (final entry in (store! as Map).entries)
      entry.key as String: entry.value! as String,
  };
}

/// The value the flat store [flat] holds for the field [key], the reverse
/// of [flatten]; the caller owns what comes back and closes it.
///
/// [flat] is a map of `String` to `String`, or a [Ref] onto one. `null`
/// when [key] names no field, nothing is stored under it, or the stored
/// shape cannot be its kind (a tagged field whose discriminant is missing
/// or names no arm). An entry the schema does not declare is dropped.
///
/// A store that is not text to text, in either form, throws an
/// [ArgumentError]: `null` answers "nothing stored", never a malformed
/// store.
Value? unflatten(
  Ref schema,
  String key,
  Object flat, {
  Pointer<guatiao_alloc>? alloc,
}) {
  final lib = native.formLib();
  final allocator = alloc ?? lib.alloc;
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
  final out = calloc<guatiao_value>();
  var filled = false;
  try {
    final status = using(
      (arena) => lib.bindings.guatiao_intake_unflatten(
        schema.requireNode(),
        strOf(arena, key).ref,
        store.requireNode(),
        allocator,
        out,
      ),
    );
    if (status == Status.wrongKind.code) return null;
    checkStatus(status);
    filled = true;
    return Value.adopt(out, alloc: allocator);
  } finally {
    if (!filled) {
      native.core().bindings.guatiao_value_free(out);
      calloc.free(out);
    }
    owned?.close();
  }
}
