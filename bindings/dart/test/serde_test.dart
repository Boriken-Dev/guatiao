// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `guatiao_serde` against a real `--all-features` build.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/serde.dart' as serde;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('a JSON round trip keeps key order and integers', () {
    final doc = Value.fromDart({
      'z': 3,
      'a': 1,
      'list': [1, 'two', null, true],
    });
    addTearDown(doc.close);
    final parsed = serde.loads(serde.dumps(doc));
    addTearDown(parsed.close);
    final map = parsed.asMap();
    expect(map.keys, ['z', 'a', 'list']);
    expect(map['z']!.toDart(), 3);
    expect(map['list']!.toDart(), [1, 'two', null, true]);
  });

  test('a number keeps its exact text through JSON', () {
    final doc = Value.fromDart({'n': Value.number('1.10')});
    addTearDown(doc.close);
    final parsed = serde.loads(serde.dumps(doc));
    addTearDown(parsed.close);
    expect(parsed.asMap()['n']!.toDart(numbers: Numbers.text), '1.10');
  });

  test('pretty JSON is longer than compact', () {
    final doc = Value.fromDart({'a': 1});
    addTearDown(doc.close);
    expect(
      serde.dumps(doc, pretty: true).length,
      greaterThan(serde.dumps(doc).length),
    );
  });

  test('a TOML round trip', () {
    final doc = Value.fromDart({'a': 1, 'b': 'x'});
    addTearDown(doc.close);
    final parsed = serde.loads(
      serde.dumps(doc, format: Format.toml),
      format: Format.toml,
    );
    addTearDown(parsed.close);
    expect(parsed.toDart(), {'a': 1, 'b': 'x'});
  });

  test('a YAML round trip', () {
    final doc = Value.fromDart({'a': 1, 'b': 'x'});
    addTearDown(doc.close);
    final parsed = serde.loads(
      serde.dumps(doc, format: Format.yaml),
      format: Format.yaml,
    );
    addTearDown(parsed.close);
    expect(parsed.toDart(), {'a': 1, 'b': 'x'});
  });

  test('a TOML document must be a map', () {
    final doc = Value.fromDart([1, 2]);
    addTearDown(doc.close);
    expect(
      () => serde.dumps(doc, format: Format.toml),
      throwsA(isA<WrongKind>()),
    );
  });

  test('parsing text that is not the format fails', () {
    expect(() => serde.loads('{not json'), throwsA(isA<GuatiaoException>()));
  });
}
