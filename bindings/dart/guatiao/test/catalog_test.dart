// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `Registry.catalog` over `catalog_library.c`: every provider with what a
/// settings screen shows, and none of them built. Every `create` of that
/// library refuses saying whether it was the first it was asked, and
/// `catalog_built` counts them.
@TestOn('vm')
library;

import 'dart:ffi';
import 'dart:io';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

/// `catalog_library.c` built into a directory of its own, or null without
/// a C compiler. Compiles with `CC` where it is set, else the first of
/// clang and cc found.
String? buildCatalogLibrary() {
  final named = Platform.environment['CC'] ?? '';
  final compilers = named.isNotEmpty
      ? [named]
      : ['clang', r'C:\Program Files\LLVM\bin\clang.exe', 'cc'];
  final out = Directory.systemTemp.createTempSync('guatiao-dart-catalog-');
  final file = '${out.path}/${dllFilename('catalog_library')}';
  for (final cc in compilers) {
    try {
      final result = Process.runSync(cc, [
        '-shared',
        '-std=c11',
        if (!Platform.isWindows) '-fPIC',
        '-I',
        repoPath(['crates', 'guatiao', 'include']),
        '-I',
        repoPath(['examples', 'greeter_kind', 'include']),
        '-o',
        file,
        repoPath([
          'crates',
          'guatiao',
          'tests',
          'c_consumer',
          'catalog_library.c',
        ]),
      ]);
      expect(result.exitCode, 0, reason: '${result.stderr}');
      return file;
    } on ProcessException {
      if (named.isNotEmpty) rethrow;
    }
  }
  out.deleteSync();
  return null;
}

void main() {
  setUpAll(libraryDir);

  test('the catalog lists every provider of a directory and builds none', () {
    final file = buildCatalogLibrary();
    if (file == null) {
      markTestSkipped('no C compiler found');
      return;
    }
    final reg = Registry('catalog-host', '1.0');
    addTearDown(reg.close);
    final report = reg.scanDir(File(file).parent.path);
    expect((report['loaded']! as List).length, 1, reason: '$report');

    final catalog = {for (final entry in reg.catalog()) entry['id']: entry};
    expect(catalog.keys.toSet(), {
      'catalog_configured',
      'catalog_implied',
      'catalog_plain',
      'catalog_unavailable',
    });

    final configured = catalog['catalog_configured']!;
    expect(configured['kinds'], ['counter']);
    expect(configured['display_name'], 'Configured, with a form');
    expect(configured['version'], '2.0');
    expect(configured['library'], 'catalog_library');
    expect(configured['library_version'], '1.0');
    expect(configured['available'], isTrue);
    expect(configured.containsKey('reason'), isFalse);
    expect((configured['config']! as Map)['required'], ['host', 'port']);
    expect(((configured['meta']! as Map)['form']! as Map)['sections'], [
      {'id': 'net', 'title': 'Network'},
    ]);
    final digest = configured['digest']! as String;
    expect(digest, matches(RegExp(r'^sha256:[0-9a-f]{64}$')));
    for (final entry in catalog.values) {
      expect(entry['digest'], digest, reason: 'one library, one digest');
    }

    final unavailable = catalog['catalog_unavailable']!;
    expect(unavailable['available'], isFalse);
    expect(unavailable['reason'], 'no device here');
    expect(unavailable.containsKey('config'), isFalse);
    expect(catalog['catalog_implied']!.containsKey('meta'), isFalse);

    // Listing asked the library for nothing: its count says so, and so
    // does its refusal of the first instance asked for afterwards.
    final built = DynamicLibrary.open(
      file,
    ).lookupFunction<Int32 Function(), int Function()>('catalog_built');
    expect(built(), 0);
    expect(
      () =>
          reg.create('catalog_configured', {'host': 'localhost', 'port': 8080}),
      throwsA(
        isA<Unsupported>().having(
          (e) => e.message,
          'message',
          'built nothing: the first create this library was asked',
        ),
      ),
    );
    expect(built(), 1);
  });

  test('the catalog names why a library has no digest', () {
    final file = buildCatalogLibrary();
    if (file == null) {
      markTestSkipped('no C compiler found');
      return;
    }
    final reg = Registry('catalog-host', '1.0');
    addTearDown(reg.close);
    reg.loadFile(file);
    // Another file at the path before anything asked: renaming a mapped
    // file works on every platform, where overwriting one does not.
    File(file).renameSync('${File(file).parent.path}/moved_away.bin');
    File(file).writeAsStringSync('not the library');
    final catalog = reg.catalog();
    expect(catalog, hasLength(4));
    for (final entry in catalog) {
      expect(entry.containsKey('digest'), isFalse);
      expect(entry['digest_error'], 'the file changed since it was loaded');
    }
  });
}
