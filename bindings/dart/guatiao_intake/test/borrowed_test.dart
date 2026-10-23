// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A view that borrows from a schema or a value: it keeps its owner alive,
/// refuses once the owner is closed, and cannot change what it views.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
import 'package:test/test.dart';

import 'support.dart';

const _schema = <String, Object?>{
  'type': 'object',
  'properties': {
    'host': {'type': 'string', 'title': 'Host'},
  },
};

/// Reuses freed heap blocks, so a read through a stale address shows.
List<Value> _churn() => [
  for (var i = 0; i < 200; i++)
    Value.fromDart({'k' * 40: List.filled(8, 'x' * 64)}),
];

void main() {
  setUpAll(setUpLibrary);

  test('a resolved field is refused once its schema is closed', () {
    final schema = Value.fromDart(_schema);
    final host = intake.resolve(schema, 'host')!;
    schema.close();
    final junk = _churn();
    addTearDown(() {
      for (final item in junk) {
        item.close();
      }
    });
    expect(host.toDart, throwsA(isA<StateError>()));
    expect(() => host.asMap(), throwsA(isA<StateError>()));
  });

  test('an at view is refused once its value is closed', () {
    final value = Value.fromDart({
      'a': {'b': 'c' * 50},
    });
    final view = intake.at(value, 'a')!;
    value.close();
    final junk = _churn();
    addTearDown(() {
      for (final item in junk) {
        item.close();
      }
    });
    expect(view.toDart, throwsA(isA<StateError>()));
  });

  test('a view onto a view keeps the first owner', () {
    final value = Value.fromDart({
      'a': {'b': 'c'},
    });
    final inner = intake.at(intake.at(value, 'a')!, 'b')!;
    expect(inner.toDart(), 'c');
    value.close();
    expect(inner.toDart, throwsA(isA<StateError>()));
  });

  test('a temporary schema lives as long as the field resolved from it', () {
    final host = intake.resolve(Value.fromDart(_schema), 'host')!;
    final junk = _churn();
    addTearDown(() {
      for (final item in junk) {
        item.close();
      }
    });
    expect(host.asMap()['title']!.toDart(), 'Host');
  });

  test('a resolved field follows a change to its schema', () {
    final schema = Value.fromDart(_schema);
    addTearDown(schema.close);
    final host = intake.resolve(schema, 'host')!;
    schema.asMap()['properties']!.asMap()['host'] = {'title': 'Moved'};
    expect(host.asMap()['title']!.toDart(), 'Moved');
  });

  test('nothing can be changed through a borrowed view', () {
    final schema = Value.fromDart(_schema);
    addTearDown(schema.close);
    final map = intake.resolve(schema, 'host')!.asMap();
    expect(() => map.remove('title'), throwsStateError);
    expect(() => map['title'] = 'x', throwsStateError);
    expect(map.clear, throwsStateError);
    expect(() => map.copyFrom(map), throwsStateError);
    expect(map['title']!.toDart(), 'Host');

    final value = Value.fromDart({
      'tags': ['a', 'b'],
    });
    addTearDown(value.close);
    final tags = intake.at(value, 'tags')!.asList();
    expect(() => tags.add('c'), throwsStateError);
    expect(() => tags.removeAt(0), throwsStateError);
    expect(() => tags[0] = 'z', throwsStateError);
    expect(() => tags.insert(0, 'z'), throwsStateError);
    expect(tags.clear, throwsStateError);
    expect(value.toDart(), {
      'tags': ['a', 'b'],
    });

    // Reading, and copying out, are not changes.
    final copy = tags.clone();
    addTearDown(copy.close);
    expect(copy.toDart(), ['a', 'b']);
  });
}
