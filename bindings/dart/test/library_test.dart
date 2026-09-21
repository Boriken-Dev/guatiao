// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `library.dart` against a real build: every library resolves, the
/// default allocator fills once, and a name nothing answers to reports
/// where it looked.
library;

import 'dart:ffi';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/library.dart' as native;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('the core library loads and fills its allocator once', () {
    final lib = native.core();
    final first = lib.alloc;
    expect(identical(first, lib.alloc), isTrue);
    expect(first.ref.struct_size, 40);
    expect(first.ref.alloc, isNot(equals(nullptr)));
    expect(first.ref.free, isNot(equals(nullptr)));
  });

  test('an --all-features build exports the load surface', () {
    final lib = native.core();
    for (final name in [
      'guatiao_registry_new',
      'guatiao_registry_retire',
      'guatiao_registry_unload',
      'guatiao_registry_unload_unchecked',
    ]) {
      expect(lib.provides(name), isTrue, reason: name);
    }
  });

  test('serde and form load, with every format built', () {
    for (final name in [
      'guatiao_json_parse',
      'guatiao_toml_parse',
      'guatiao_yaml_parse',
    ]) {
      expect(native.serdeLib().provides(name), isTrue, reason: name);
    }
    expect(native.formLib().provides('guatiao_form_check'), isTrue);
  });

  test('a missing symbol names the library and the feature', () {
    expect(
      () => native.core().require('guatiao_not_a_symbol', feature: 'load'),
      throwsA(
        isA<MissingSymbol>()
            .having((e) => e.name, 'name', 'guatiao_not_a_symbol')
            .having((e) => e.feature, 'feature', 'load'),
      ),
    );
  });

  test('an unknown base name reports both places it looked', () {
    expect(
      () => native.NativeLib.open('not_a_real_guatiao_library'),
      throwsA(
        isA<LibraryNotFound>().having(
          (e) => e.toString(),
          'message',
          allOf(
            contains('GUATIAO_LIBRARY'),
            contains('libraryDirectory'),
            contains('search path'),
            contains(dllFilename('not_a_real_guatiao_library')),
          ),
        ),
      ),
    );
  });

  group('a library registered by hand', () {
    tearDown(native.forgetLibraries);

    test('answers for every surface it carries, and no other', () {
      final path = native.resolveLibraryPath('guatiao');
      useLibrary(DynamicLibrary.open(path), path: path);

      // The core surface is there; the serde one is a separate library,
      // so it was not claimed on this library's behalf.
      expect(native.core().path, path);
      expect(() => native.serdeLib().path, returnsNormally);
      expect(native.serdeLib().path, isNot(path));
    });

    test('takes the caller word when the surfaces are named', () {
      final path = native.resolveLibraryPath('guatiao');
      useLibrary(
        DynamicLibrary.open(path),
        path: path,
        forSurfaces: const ['guatiao_form'],
      );
      expect(native.formLib().path, path);
    });

    test('refuses a name that is not a surface', () {
      final path = native.resolveLibraryPath('guatiao');
      expect(
        () => useLibrary(
          DynamicLibrary.open(path),
          forSurfaces: const ['guatiao_nonesuch'],
        ),
        throwsArgumentError,
      );
    });

    test('is forgotten on request', () {
      final path = native.resolveLibraryPath('guatiao');
      useLibrary(DynamicLibrary.open(path), path: path);
      expect(native.core().path, path);
      native.forgetLibraries();
      expect(native.core().path, isNot('<registered>'));
    });
  });
}
