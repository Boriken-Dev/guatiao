// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'bindings.g.dart';
import 'registry.dart';

/// The table is too small for this binding's struct, or its `floor_hash`
/// does not match the one the kind's own header names.
class FloorMismatch implements Exception {
  FloorMismatch(this.reason);

  final String reason;

  @override
  String toString() => 'FloorMismatch: $reason';
}

/// Takes a [ProviderTable] as the struct a kind's own C header declares.
///
/// `T` is a `Struct` whose first field is a `guatiao_kind_header` --
/// `greeter_vtable` and friends. Both checks happen before the cast:
/// [found]'s size against [floorSize] (a shorter table is an older
/// library, refused here rather than read past its end), and the header's
/// `floor_hash` against [floorHash], the `<table>_FLOOR_HASH` macro the
/// kind's header carries.
///
/// [floorSize] is the caller's `sizeOf<T>()`: `sizeOf` needs a type known
/// where it is written, which a type variable is not.
Pointer<T> kindTable<T extends Struct>(
  ProviderTable found, {
  required int floorHash,
  required int floorSize,
}) {
  if (found.table == nullptr) {
    throw FloorMismatch('no such table');
  }
  if (found.size < floorSize) {
    throw FloorMismatch(
      'table is ${found.size} bytes, this binding needs $floorSize',
    );
  }
  final header = found.table.cast<guatiao_kind_header>().ref;
  if (header.floor_hash != floorHash) {
    throw FloorMismatch(
      'floor_hash ${header.floor_hash} does not match $floorHash',
    );
  }
  return found.table.cast<T>();
}
