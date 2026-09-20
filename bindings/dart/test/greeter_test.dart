// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A provider's kind table, called live from Dart: fetched by kind,
/// checked against the floor hash the kind's own header names, and asked
/// to greet.
library;

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/bindings.g.dart';
import 'package:guatiao/src/value.dart' show strOf;
import 'package:test/test.dart';

import 'greeter_table.dart';
import 'support.dart';

int greeterFloorHash() {
  final text = File(
    repoPath(['examples', 'greeter_kind', 'include', 'greeter_kind.h']),
  ).readAsStringSync();
  final match =
      RegExp(r'#define greeter_vtable_FLOOR_HASH (\d+)').firstMatch(text);
  expect(match, isNotNull, reason: 'greeter_vtable_FLOOR_HASH in the header');
  return int.parse(match!.group(1)!);
}

/// A `guatiao_map *` that addresses the payload of a map-tagged value, so
/// what a provider writes there is already a whole node.
///
/// `guatiao_value.payload` is at offset 8, pinned by `abi_test.dart`.
Pointer<guatiao_map> mapSlot(Pointer<guatiao_value> node) {
  node.ref.tag = Tag.map.code;
  return (node.cast<Uint8>() + 8).cast<guatiao_map>();
}

void main() {
  setUpAll(libraryDir);

  test('the struct matches the size the kind header declares', () {
    expect(sizeOf<GreeterVtable>(), 8 + 3 * sizeOf<Pointer<Void>>());
    expect(greeterFloorHash(), greaterThan(0));
  });

  test('a table is fetched by kind, checked, and called', () {
    final reg = Registry('dart-tests', '0.1')
      ..scanDir(libraryDir(), rules: 'kind=greeter');
    addTearDown(reg.close);

    final found = reg.providerTable('derived_greeter_hello', kind: 'greeter');
    final table = kindTable<GreeterVtable>(
      found,
      floorHash: greeterFloorHash(),
      floorSize: sizeOf<GreeterVtable>(),
    );

    final out = calloc<guatiao_value>();
    final err = calloc<guatiao_provider_error>();
    try {
      final greet = table.ref.greet.asFunction<GreetDart>();
      final status = using(
        (arena) => greet(found.ctx, strOf(arena, 'ana').ref, mapSlot(out), err),
      );
      expect(status, Status.ok.code);
      final answer = Value.adopt(out);
      addTearDown(answer.close);
      expect(answer.toDart(), {'greeting': 'hello, ana'});
    } finally {
      calloc.free(err);
    }
  });

  test('a table shorter than this binding needs is refused', () {
    final reg = Registry('dart-tests', '0.1')
      ..scanDir(libraryDir(), rules: 'kind=greeter');
    addTearDown(reg.close);
    final found = reg.providerTable('derived_greeter_hello', kind: 'greeter');
    expect(
      () => kindTable<GreeterVtable>(
        found,
        floorHash: greeterFloorHash(),
        floorSize: found.size + 8,
      ),
      throwsA(isA<FloorMismatch>()),
    );
    expect(
      () => kindTable<GreeterVtable>(
        found,
        floorHash: greeterFloorHash() + 1,
        floorSize: sizeOf<GreeterVtable>(),
      ),
      throwsA(isA<FloorMismatch>()),
    );
    expect(
      () => kindTable<GreeterVtable>(
        reg.providerTable('derived_greeter_hello'),
        floorHash: greeterFloorHash(),
        floorSize: sizeOf<GreeterVtable>(),
      ),
      throwsA(isA<FloorMismatch>()),
    );
  });

  test("hello_library's own vtable is called live", () {
    final reg = Registry('dart-tests', '0.1')
      ..scanDir(libraryDir(), rules: 'kind=greeter');
    addTearDown(reg.close);

    final found = reg.providerTable('hello_library_greeter');
    expect(found.isEmpty, isFalse);
    expect(found.size, greaterThanOrEqualTo(sizeOf<HelloGreeterVtable>()));
    final table = found.table.cast<HelloGreeterVtable>();

    final config = Value.fromDart({'name': 'ana'});
    addTearDown(config.close);
    final out = calloc<guatiao_value>();
    final greet = table.ref.greet.asFunction<
        int Function(
          Pointer<Void>,
          Pointer<guatiao_value>,
          Pointer<guatiao_value>,
        )>();
    expect(greet(found.ctx, config.pointer, out), Status.ok.code);
    final answer = Value.adopt(out);
    addTearDown(answer.close);
    expect(answer.toDart(), {'greeting': 'hello, ana'});

    final outstanding =
        table.ref.outstanding.asFunction<int Function(Pointer<Void>)>();
    expect(outstanding(found.ctx), greaterThanOrEqualTo(0));
  });
}
