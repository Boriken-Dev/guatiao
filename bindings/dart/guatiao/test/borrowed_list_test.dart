// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A list a slot borrows: the callee reads it through a [ListRef] it
/// cannot change, and the caller's list is as it was after the call.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/kind_generator.dart';
import 'package:guatiao/src/kind_runtime.dart' show hold, release;
import 'package:test/test.dart';

import 'fixtures/summer_kind.dart';
import 'support.dart';

final class Adds implements Summer {
  /// What each attempt to change the borrowed list came to.
  final List<String> refused = [];

  @override
  int sum(ListRef items) {
    try {
      items.add(0);
    } on StateError catch (e) {
      refused.add(e.message);
    }
    var total = 0;
    for (final item in items.toDart()! as List) {
      total += item as int;
    }
    return total;
  }
}

void main() {
  late SummerRemote summer;
  late Adds adds;
  late int ctx;
  setUp(() {
    adds = Adds();
    ctx = hold(adds);
    summer = Summer.kind.remote(
      Summer.kind.tableFor(adds),
      Summer.kind.tableSize,
      Pointer<Void>.fromAddress(ctx),
    );
  });
  tearDown(() => release(ctx));

  test('the callee reads a borrowed list and cannot change it', () {
    expect(summer.sum([40, 2]), 42);
    final held = Value([1, 2, 3]);
    addTearDown(held.close);
    expect(summer.sum(held.asList()), 6);
    expect(held.toDart(), [1, 2, 3]);
    expect(adds.refused, [
      'a borrowed value is read-only',
      'a borrowed value is read-only',
    ]);
    expect(() => summer.sum({'a': 1}), throwsArgumentError);
  });

  test('the generated file is what the description writes', () {
    final committed =
        jsonDecode(
              File(
                repoPath([
                  'bindings',
                  'dart',
                  'guatiao',
                  'test',
                  'fixtures',
                  'summer.kinds.json',
                ]),
              ).readAsStringSync(),
            )
            as Map<String, Object?>;
    final generated = generateKinds(
      committed,
      source: 'summer.kinds.json',
      command:
          'dart run guatiao:kind test/fixtures/summer.kinds.json -o '
          'test/fixtures/summer_kind.dart',
    );
    final scratch = Directory.systemTemp.createTempSync('summer_kind');
    addTearDown(() => scratch.deleteSync(recursive: true));
    final written = File('${scratch.path}/summer_kind.dart')
      ..writeAsStringSync(generated);
    final formatted = Process.runSync(Platform.resolvedExecutable, [
      'format',
      '--output=show',
      written.path,
    ]);
    expect(formatted.exitCode, 0, reason: '${formatted.stderr}');
    expect(
      (formatted.stdout as String).replaceAll('\r\n', '\n'),
      startsWith(
        File(
          repoPath([
            'bindings',
            'dart',
            'guatiao',
            'test',
            'fixtures',
            'summer_kind.dart',
          ]),
        ).readAsStringSync(),
      ),
    );
  });
}
