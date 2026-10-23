// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The web's module: one per page, what a browser cannot do refused with a
/// sentence, and values that stay readable while the memory grows.
@TestOn('browser')
library;

import 'dart:typed_data';

// The web's own libraries by name: an analyzer reads `guatiao.dart` and
// `support.dart` as the VM sees them.
import 'package:guatiao/guatiao.dart'
    hide ProviderTable, Ref, Registry, Value, kindTable;
import 'package:guatiao/src/web/registry_web.dart'
    show ProviderTable, Registry, kindTable;
import 'package:guatiao/wasm.dart' show Address, Value, WasmModule;
import 'package:test/test.dart';

import 'support_web.dart';

void main() {
  setUpAll(setUpLibrary);

  test('a second module is refused while one is loaded', () async {
    await expectLater(
      loadModule(await moduleUrl()),
      throwsA(isA<StateError>()
          .having((e) => e.message, 'message', contains('loaded already'))),
    );
  });

  test('the registry and kind tables throw Unsupported, saying why', () {
    final refusal = isA<Unsupported>().having(
        (e) => e.message, 'message', contains('a browser maps no libraries'));
    expect(() => Registry('page', '1.0'), throwsA(refusal));
    expect(
      () => kindTable<Object>(
          const ProviderTable(Address.none, 0, Address.none),
          floorHash: 0,
          floorSize: 0),
      throwsA(refusal),
    );
  });

  test('a value reads the same after the memory grows under it', () {
    final kept = Value.fromDart({
      'name': 'ana',
      'list': [1, 'two', null],
    });
    addTearDown(kept.close);
    final ref = kept.asMap()['list']!;
    final pages = WasmModule.current.data.lengthInBytes;
    // Larger than the memory there is: the module must grow it.
    final big = Value.bytes(Uint8List(pages + (1 << 20)));
    addTearDown(big.close);
    expect(WasmModule.current.data.lengthInBytes, greaterThan(pages));
    expect(kept.toDart(), {
      'name': 'ana',
      'list': [1, 'two', null],
    });
    expect(ref.toDart(), [1, 'two', null]);
    expect((big.toDart()! as Uint8List).length, pages + (1 << 20));
  });

  test('forgetting the module retires the values built in it', () async {
    final old = Value.fromDart({'a': 1});
    forgetLibraries();
    expect(() => old.toDart(), throwsStateError);
    expect(() => Value.map(), throwsA(isA<LibraryNotFound>()));
    old.close();
    await loadModule(await moduleUrl());
    final fresh = Value.fromDart({'b': 2});
    addTearDown(fresh.close);
    expect(fresh.toDart(), {'b': 2});
  });

  test('a module that is no guatiao module is refused', () async {
    forgetLibraries();
    addTearDown(() async => loadModule(await moduleUrl()));
    // The smallest valid module: the magic number and version 1.
    await expectLater(
      loadModuleBytes(const [0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]),
      throwsA(isA<ArgumentError>().having(
          (e) => e.message, 'message', contains('exports no guatiao surface'))),
    );
  });
}
