// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Sources from Dart: a watched directory refreshed as files come and go,
/// and a file that is no source refused with words.
@TestOn('vm')
library;

import 'dart:io';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

String libraryName(String base) {
  if (Platform.isWindows) return '$base.dll';
  if (Platform.isMacOS) return 'lib$base.dylib';
  return 'lib$base.so';
}

String join(List<String> parts) => parts.join(Platform.pathSeparator);

void main() {
  setUpAll(libraryDir);

  test('a watched directory follows the files added and removed', () {
    final root = Directory.systemTemp.createTempSync('guatiao-dart-watch');
    addTearDown(() {
      try {
        root.deleteSync(recursive: true);
      } on FileSystemException {
        // A library still mapped on Windows cannot be deleted; the
        // directory is a temporary one.
      }
    });
    final watched = Directory(join([root.path, 'watched']))..createSync();
    final aside = Directory(join([root.path, 'aside']))..createSync();
    final reg = Registry('dart-sources', '0.1');
    addTearDown(reg.close);

    reg.watchDir(watched.path, rules: 'kind=greeter');
    expect(() => reg.watchDir(watched.path), throwsA(isA<BadValue>()));
    expect(reg.refresh()['loaded'], isEmpty);

    final copy = File(
      join([libraryDir(), libraryName('hello_library')]),
    ).copySync(join([watched.path, libraryName('hello_dart_watched')]));
    final added = reg.refresh();
    expect(
      [for (final s in added['loaded']! as List) (s! as Map)['key']],
      ['hello_library'],
    );

    // A mapped image cannot be deleted on Windows; moved aside, it is gone.
    copy.renameSync(join([aside.path, libraryName('hello_dart_watched')]));
    final removed = reg.refresh();
    expect(
      [for (final s in removed['retired']! as List) (s! as Map)['key']],
      ['hello_library'],
    );
    expect(reg.libraries(), isEmpty);
  });

  test('the same library from two sources is kept apart by its source', () {
    final root = Directory.systemTemp.createTempSync('guatiao-dart-sources');
    addTearDown(() {
      try {
        root.deleteSync(recursive: true);
      } on FileSystemException {
        // A library still mapped on Windows cannot be deleted.
      }
    });
    final reg = Registry('dart-sources', '0.1')
      ..librariesKeyedBy('%source/%id')
      ..keyedBy('%source/%id');
    addTearDown(reg.close);
    final dirs = [
      for (final name in ['one', 'two'])
        Directory(join([root.path, name]))..createSync(),
    ];
    for (final (i, dir) in dirs.indexed) {
      File(
        join([libraryDir(), libraryName('hello_library')]),
      ).copySync(join([dir.path, libraryName('hello_dart_source_$i')]));
      reg.watchDir(dir.path, rules: 'kind=greeter');
    }

    final loaded = reg.refresh()['loaded']! as List;
    expect(
      [for (final s in loaded) (s! as Map)['key']],
      [for (final dir in dirs) '${dir.path}/hello_library'],
    );
    expect(
      [for (final lib in reg.libraries()) lib['source']],
      [for (final dir in dirs) dir.path],
    );
    expect(
      reg.provider('${dirs[1].path}/hello_library_greeter')['source'],
      dirs[1].path,
    );
  });

  test('a file that is no source is refused with words', () {
    final reg = Registry('dart-sources', '0.1');
    addTearDown(reg.close);
    expect(
      () => reg.addSourceFile(
        join([libraryDir(), libraryName('derived_greeter')]),
      ),
      throwsA(
        isA<WrongKind>().having(
          (e) => e.message,
          'message',
          contains('guatiao_source'),
        ),
      ),
    );
    expect(
      () => reg.addSourceFile(join([libraryDir(), 'no-such-file'])),
      throwsA(isA<NotFound>()),
    );
  });
}
