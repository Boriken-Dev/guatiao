// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'library.dart' as native;
import 'status.dart';
import 'value.dart';

/// The floor hash of the kind whose description [kind] holds as JSON,
/// computed by the library (`guatiao_kind_floor_hash`); [BadValue] names
/// what in it is not a description.
int kindFloorHash(String kind) {
  final lib = native.core();
  final out = calloc<Uint32>();
  final why = calloc<guatiao_value>();
  try {
    final status = using(
      (arena) => lib.bindings.guatiao_kind_floor_hash(
        strOf(arena, kind).ref,
        lib.alloc,
        out,
        why,
      ),
    );
    checkStatus(status, Ref.borrowed(why).stringOr(''));
    return out.value;
  } finally {
    lib.bindings.guatiao_value_free(why);
    calloc.free(why);
    calloc.free(out);
  }
}
