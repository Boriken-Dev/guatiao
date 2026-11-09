// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `toInlineText` against the shared vectors every implementation reads.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

Uint8List _hex(String text) => Uint8List.fromList([
  for (var i = 0; i < text.length; i += 2)
    int.parse(text.substring(i, i + 2), radix: 16),
]);

/// The value a vector tree names.
Value _build(List<Object?> tree) {
  switch (tree[0] as String) {
    case 'absent':
      return Value.absent();
    case 'null':
      return Value.nullValue();
    case 'bool':
      return Value.boolean(tree[1] as bool);
    case 'number':
      return Value.number(tree[1] as String);
    case 'string':
      return Value.string(tree[1] as String);
    case 'bytes':
      return Value.bytes(_hex(tree[1] as String));
    case 'list':
      final value = Value.list();
      for (final item in tree[1] as List) {
        final child = _build(item as List<Object?>);
        value.asList().add(child);
        child.close();
      }
      return value;
    default:
      final value = Value.map();
      for (final pair in tree[1] as List) {
        final child = _build((pair as List)[1] as List<Object?>);
        value.asMap()[pair[0] as String] = child;
        child.close();
      }
      return value;
  }
}

void main() {
  late final List<Map<String, Object?>> cases;
  setUpAll(() async {
    await setUpLibrary();
    final doc =
        jsonDecode(
              await readRepoText([
                'crates',
                'guatiao',
                'tests',
                'vectors',
                'inline.json',
              ]),
            )
            as Map<String, Object?>;
    cases = (doc['cases']! as List).cast<Map<String, Object?>>();
  });

  test('every vector prints exactly its line', () {
    expect(cases.length, greaterThanOrEqualTo(20));
    for (final c in cases) {
      final value = _build(c['tree']! as List<Object?>);
      try {
        expect(value.toInlineText(), c['text'], reason: '${c['name']}');
      } finally {
        value.close();
      }
    }
  });

  test('a child prints alone', () {
    final value = Value.fromDart({
      'outer': {
        'inner': [1, 'two'],
      },
    });
    addTearDown(value.close);
    expect(value.asMap()['outer']!.toInlineText(), '{"inner":[1,"two"]}');
  });

  test('a secret is not shown', () {
    final secret = Value.sensitive('hunter2');
    addTearDown(secret.close);
    expect(secret.toInlineText(), '[sensitive]');
    final outer = Value.sensitive({'user': 'alice', 'password': 'hunter2'});
    addTearDown(outer.close);
    expect(outer.toInlineText(), '[sensitive]');
  });
}
