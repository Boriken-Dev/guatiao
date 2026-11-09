// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `differences` against the shared vectors every implementation reads.
library;

import 'dart:convert';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

void main() {
  late final Map<String, Object?> doc;
  setUpAll(() async {
    await setUpLibrary();
    doc =
        jsonDecode(
              await readRepoText([
                'crates',
                'guatiao',
                'tests',
                'vectors',
                'differences.json',
              ]),
            )
            as Map<String, Object?>;
  });

  test('every vector lists exactly its differences', () {
    final cases = (doc['cases']! as List).cast<Map<String, Object?>>();
    expect(cases.length, greaterThanOrEqualTo(15));
    final placeholder = doc['placeholder']! as String;
    for (final c in cases) {
      final found = differences(
        c['schema'],
        c['value'],
        baseline: c['baseline'],
        placeholder: placeholder,
      );
      try {
        expect(found.toDart(), c['expected'], reason: '${c['name']}');
      } finally {
        found.close();
      }
    }
  });

  test('the placeholder is the caller\'s and a secret is never shown', () {
    final cases = (doc['cases']! as List).cast<Map<String, Object?>>();
    final secret = cases.firstWhere(
      (c) => (c['name']! as String).startsWith('a secret that changed'),
    );
    final found = differences(
      secret['schema'],
      secret['value'],
      placeholder: '[hidden]',
    );
    addTearDown(found.close);
    final shown = found.toDart();
    expect(jsonEncode(shown), isNot(contains('hunter2')));
    expect(jsonEncode(shown), contains('[hidden]'));
  });

  test('a schema that is not a map is refused', () {
    expect(() => differences(5, {}), throwsA(isA<WrongKind>()));
  });
}
