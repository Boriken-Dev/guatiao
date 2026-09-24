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
