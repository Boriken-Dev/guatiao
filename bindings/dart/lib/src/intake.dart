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
/// What comes back **borrows** from [value]: it is a view into that
/// tree, not a copy, and it must not outlive it. A path that names
/// nothing and a path that is not a path at all both answer `null`.
Ref? at(Ref value, String path) {
  final lib = native.formLib();
  return using((arena) {
    final found = lib.bindings.guatiao_intake_path_get(
      value.requireNode(),
      strOf(arena, path).ref,
    );
    return found == nullptr ? null : Ref.borrowed(found);
  });
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
