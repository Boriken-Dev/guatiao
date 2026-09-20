// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Round trips every value kind against a real build of `guatiao`.
library;

import 'dart:typed_data';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('every kind round-trips', () {
    expect(Value.nullValue().toDart(), isNull);
    expect(Value.boolean(true).toDart(), isTrue);
    expect(Value.boolean(false).toDart(), isFalse);
    expect(Value.string('hola').toDart(), 'hola');
    expect(
      Value.bytes(Uint8List.fromList([0, 1, 255])).toDart(),
      Uint8List.fromList([0, 1, 255]),
    );
    expect(Value.absent().toDart(), same(absent));
    expect(Value().toDart(), same(absent));

    final nested = Value.fromDart({
      'a': [
        1,
        2,
        <String, Object?>{'b': 'c'}
      ],
      'd': null,
    });
    addTearDown(nested.close);
    expect(nested.toDart(), {
      'a': [
        1,
        2,
        {'b': 'c'}
      ],
      'd': null,
    });
  });

  test('a number keeps its exact text', () {
    final v = Value.number('1.10');
    addTearDown(v.close);
    expect(v.toDart(numbers: Numbers.text), '1.10');
    expect(v.toDart(numbers: Numbers.bigInt), 1.1);
    expect(v.toDart(), 1.1);
    expect(v.tag, Tag.number);
  });

  test('a key holding a NUL is found by its bytes', () {
    final root = Value.map();
    addTearDown(root.close);
    final map = root.asMap();
    map['a\u0000b'] = 1;
    map['a'] = 2;
    expect(map.keys, ['a\u0000b', 'a']);
    expect(map['a\u0000b']!.toDart(), 1);
    expect(map['a']!.toDart(), 2);
  });

  test('a 200-digit integer survives', () {
    final digits = BigInt.parse('7' * 200);
    final v = Value.fromDart(digits);
    addTearDown(v.close);
    expect(v.toDart(numbers: Numbers.bigInt), digits);
    expect(v.toDart(), digits);
    expect(v.toDart(numbers: Numbers.text), '7' * 200);
  });

  test('a 64-bit integer stays an int', () {
    final v = Value.fromDart(9007199254740993);
    addTearDown(v.close);
    expect(v.toDart(), isA<int>());
    expect(v.toDart(), 9007199254740993);
  });

  test('a non-finite double is refused', () {
    expect(() => Value.fromDart(double.nan), throwsArgumentError);
    expect(() => Value.fromDart(double.infinity), throwsArgumentError);
    expect(() => Value.fromDart(double.negativeInfinity), throwsArgumentError);
  });

  test('a Ref taken before 64 writes still reads the right value', () {
    final root = Value.map();
    addTearDown(root.close);
    final map = root.asMap();
    map['a'] = 'first';
    final ref = map['a']!;
    for (var i = 0; i < 64; i++) {
      map['k$i'] = i;
    }
    expect(ref.toDart(), 'first');
    expect(map.length, 65);
  });

  test('a closed value throws on use, and closes again quietly', () {
    final v = Value.string('x');
    v.close();
    expect(v.toDart, throwsStateError);
    expect(v.isClosed, isTrue);
    v.close();
  });

  test('the map protocol', () {
    final root = Value.map();
    addTearDown(root.close);
    final map = root.asMap();
    map['one'] = 1;
    map['two'] = 2;
    expect(map.keys, ['one', 'two']);
    expect(map.length, 2);
    expect(map.containsKey('one'), isTrue);
    expect(map['one']!.toDart(), 1);
    expect(map['nope'], isNull);
    expect(map.entries.map((e) => e.key), ['one', 'two']);
    map.remove('one');
    expect(map.containsKey('one'), isFalse);
    map.clear();
    expect(map.isEmpty, isTrue);
  });

  test('copyFrom brings every entry across', () {
    final src = Value.fromDart({'a': 1, 'b': 2});
    final dst = Value.map();
    addTearDown(src.close);
    addTearDown(dst.close);
    dst.asMap().copyFrom(src);
    expect(dst.toDart(), {'a': 1, 'b': 2});
  });

  test('the list protocol', () {
    final root = Value.list();
    addTearDown(root.close);
    final list = root.asList();
    list
      ..add(1)
      ..add(2)
      ..add(3);
    expect(list.length, 3);
    expect(root.toDart(), [1, 2, 3]);
    expect(list[0].toDart(), 1);
    expect(list[-1].toDart(), 3);
    expect(() => list[3], throwsRangeError);
    list.removeAt(1);
    expect(root.toDart(), [1, 3]);
    list.insert(1, 'mid');
    expect(root.toDart(), [1, 'mid', 3]);
    list[0] = 'first';
    expect(root.toDart(), ['first', 'mid', 3]);
    expect(list.refs.map((r) => r.toDart()), ['first', 'mid', 3]);
    list.clear();
    expect(list.isEmpty, isTrue);
  });

  test('the readers fall back rather than truncate', () {
    final root = Value.fromDart({
      'n': 5,
      's': 'hi',
      'b': true,
      'big': Value.number('99999999999999999999'),
      'frac': Value.number('1.5'),
    });
    addTearDown(root.close);
    final map = root.asMap();
    expect(map['n']!.intOr(-1), 5);
    expect(map['s']!.intOr(-1), -1);
    expect(map['big']!.intOr(-1), -1, reason: 'outside a signed 64-bit range');
    expect(map['frac']!.intOr(-1), -1, reason: 'a fractional spelling');
    expect(map['frac']!.doubleOr(-1), 1.5);
    expect(map['s']!.stringOr(''), 'hi');
    expect(map['b']!.boolOr(false), isTrue);
    expect(map['n']!.boolOr(false), isFalse);
    final missing = Ref(root, ['missing']);
    expect(missing.isAbsent, isTrue);
    expect(missing.intOr(-1), -1);
    expect(missing.toDart(), same(absent));
  });

  test('a clone is independent', () {
    final src = Value.fromDart([1, 2, 3]);
    final copy = src.clone();
    addTearDown(src.close);
    addTearDown(copy.close);
    copy.asList().add(4);
    expect(src.toDart(), [1, 2, 3]);
    expect(copy.toDart(), [1, 2, 3, 4]);
  });

  test('a nested value is copied in, not aliased', () {
    final inner = Value.fromDart({'a': 'b'});
    final outer = Value.fromDart({
      'host': 'h',
      'test': inner,
      'more': [Value.fromDart(1)],
    });
    addTearDown(outer.close);
    inner.asMap()['a'] = 'changed';
    inner.close();
    expect(outer.toDart(), {
      'host': 'h',
      'test': {'a': 'b'},
      'more': [1],
    });
  });

  test('asMap and asList refuse a node of another kind', () {
    final v = Value.string('x');
    addTearDown(v.close);
    expect(v.asMap, throwsStateError);
    expect(v.asList, throwsStateError);
  });

  test('a type with no conversion is refused', () {
    expect(() => Value.fromDart(DateTime.now()), throwsArgumentError);
    expect(() => Value.fromDart({1: 'a'}), throwsArgumentError);
  });
}
