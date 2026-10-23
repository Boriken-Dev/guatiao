// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A tree crosses between guatiao_serde and guatiao: built by one library,
/// read and freed through the other.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_serde/guatiao_serde.dart' as serde;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(setUpLibrary);

  test('a tree serde built is changed and freed through the core', () {
    final value = serde.loads('{"a": [1, "two", {"b": null}]}');
    value.asMap()['c'] = 'added through the core';
    expect(value.toDart(), {
      'a': [
        1,
        'two',
        {'b': null},
      ],
      'c': 'added through the core',
    });
    value.close();
    expect(() => value.toDart(), throwsStateError);
  });

  test('a tree the core built is written by serde', () {
    final value = Value.fromDart({
      'port': 5900,
      'tags': ['x'],
    });
    addTearDown(value.close);
    expect(serde.dumps(value), '{"port":5900,"tags":["x"]}');
    expect(serde.dumps(value.asMap()['tags']!), '["x"]');
  });

  test('a tree built in the core allocator by serde is freed by the core', () {
    final core = Value.map();
    addTearDown(core.close);
    for (var i = 0; i < 3; i++) {
      final value = serde.loads('{"k": "v"}', alloc: core.allocator);
      expect(value.toDart(), {'k': 'v'});
      value.close();
    }
  });
}
