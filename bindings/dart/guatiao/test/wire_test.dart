// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The wire encoding and channels against the shared vectors every
/// implementation reads: each value both ways, each malformed input
/// refused, the frames the builders write, and a conversation with every
/// refusal.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart';
// The tape is exported by ffi.dart and wasm.dart, one per platform.
import 'package:guatiao/src/tape.dart' show dartToWire, wireToDart;
import 'package:guatiao/wire.dart' as wire;
import 'package:test/test.dart';

import 'support.dart';

late final Map<String, Object?> _vectors;

List<Map<String, Object?>> _list(String key) =>
    (_vectors[key] as List).cast<Map<String, Object?>>();

Uint8List _hex(String text) => Uint8List.fromList([
  for (var i = 0; i < text.length; i += 2)
    int.parse(text.substring(i, i + 2), radix: 16),
]);

String _toHex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

/// The channel a frame names, read from its bytes: JSON numbers past 2^53
/// do not survive `jsonDecode`, and a channel is 64 bits, the ones past
/// 2^63 carried as negative `int`s.
int _channelOf(Uint8List frame) {
  var n = BigInt.zero;
  var shift = 0;
  for (var i = 1; i < frame.length; i++) {
    n |= BigInt.from(frame[i] & 0x7f) << shift;
    if (frame[i] & 0x80 == 0) break;
    shift += 7;
  }
  return n.toSigned(64).toInt();
}

/// A node as the vector file writes a tree.
Object describe(Ref ref) {
  switch (ref.tag) {
    case Tag.absent:
      return ['absent'];
    case Tag.nullValue:
      return ['null'];
    case Tag.boolean:
      return ['bool', ref.boolOr(false)];
    case Tag.number:
      return ['number', ref.toDart(numbers: Numbers.text)!];
    case Tag.string:
      return ['string', ref.stringOr('')];
    case Tag.bytes:
      return ['bytes', _toHex(ref.toDart() as Uint8List)];
    case Tag.list:
      final list = ref.asList();
      return [
        'list',
        [for (var i = 0; i < list.length; i++) describe(list[i])],
      ];
    case Tag.map:
      final map = ref.asMap();
      return [
        'map',
        [
          for (final key in map.keys) [key, describe(map[key]!)],
        ],
      ];
  }
}

/// The value a vector tree names.
Value build(List<Object?> tree) {
  final tag = tree[0] as String;
  switch (tag) {
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
        final child = build(item as List<Object?>);
        value.asList().add(child);
        child.close();
      }
      return value;
    default:
      final value = Value.map();
      for (final pair in tree[1] as List) {
        final child = build((pair as List)[1] as List<Object?>);
        value.asMap()[pair[0] as String] = child;
        child.close();
      }
      return value;
  }
}

void main() {
  setUpAll(() async {
    await setUpLibrary();
    _vectors =
        jsonDecode(
              await readRepoText([
                'crates',
                'guatiao',
                'tests',
                'vectors',
                'wire.json',
              ]),
            )
            as Map<String, Object?>;
  });

  test('every value decodes to its tree and encodes to its bytes', () {
    final cases = _list('values');
    expect(cases, isNotEmpty);
    for (final c in cases) {
      final data = _hex(c['hex'] as String);
      final value = Value.fromWire(data);
      expect(describe(value), c['tree'], reason: c['name'] as String);
      expect(value.toWire(), data, reason: c['name'] as String);
      value.close();
      final built = build(c['tree'] as List<Object?>);
      expect(built.toWire(), data, reason: c['name'] as String);
      built.close();
    }
  });

  test('every value read and written as wire bytes in Dart', () {
    for (final c in _list('values')) {
      final data = _hex(c['hex'] as String);
      final name = c['name'] as String;
      for (final numbers in Numbers.values) {
        final value = Value.fromWire(data);
        final expected = value.toDart(numbers: numbers);
        value.close();
        final got = wireToDart(data, numbers: numbers);
        expect(got, expected, reason: name);
        expect(got.toString(), expected.toString(), reason: name);
        if (numbers == Numbers.text) continue;
        final back = Value.fromDart(got);
        expect(dartToWire(got), back.toWire(), reason: name);
        back.close();
      }
    }
  });

  test('wire bytes cut short are refused in Dart', () {
    final value = Value.fromDart({
      'a': [
        1,
        'two',
        Uint8List.fromList([0]),
      ],
    });
    final data = value.toWire();
    value.close();
    for (var end = 0; end < data.length; end++) {
      expect(
        () => wireToDart(Uint8List.sublistView(data, 0, end)),
        throwsA(isA<BadValue>()),
        reason: '$end',
      );
    }
    expect(
      () => wireToDart(Uint8List.fromList([...data, 1])),
      throwsA(isA<BadValue>()),
    );
  });

  test('Dart written as wire bytes refuses what fromDart refuses', () {
    for (final obj in <Object?>[
      double.nan,
      {1: 2},
      Object(),
    ]) {
      expect(() => Value.fromDart(obj), throwsArgumentError);
      expect(() => dartToWire(obj), throwsArgumentError);
    }
    final inner = Value.fromDart({
      'x': [true, null],
    });
    final whole = Value.fromDart({
      'ref': {
        'x': [true, null],
      },
    });
    expect(dartToWire({'ref': inner}), whole.toWire());
    inner.close();
    whole.close();
  });

  test('every malformed input is refused', () {
    for (final c in _list('malformed')) {
      expect(
        () => Value.fromWire(_hex(c['hex'] as String)),
        throwsA(isA<BadValue>()),
        reason: c['name'] as String,
      );
    }
  });

  test('a child encodes alone', () {
    final value = Value.fromDart({
      'outer': {
        'inner': [1, 'two'],
      },
    });
    final back = Value.fromWire(value.asMap()['outer']!.toWire());
    expect(back.toDart(), {
      'inner': [1, 'two'],
    });
    back.close();
    value.close();
  });

  test('the frame builders write the documented bytes', () {
    for (final f in _list('frames')) {
      final expected = _hex(f['hex'] as String);
      final channel = _channelOf(expected);
      if (f['build'] == 'close') {
        expect(wire.close(channel), expected);
        continue;
      }
      final tree = build(f['tree'] as List<Object?>);
      final built = f['build'] == 'announce'
          ? wire.announce(channel, tree)
          : wire.send(channel, tree);
      expect(built, expected, reason: '$f');
      tree.close();
    }
    expect(
      wire.send(7, {'greeting': 'hello'}),
      _hex(_list('frames')[1]['hex'] as String),
    );
  });

  test('a channel past 2^63 comes back as Dart carries it', () {
    final receiver = wire.Receiver();
    addTearDown(receiver.close);
    // 2^64 - 1, the largest channel: -1 as a Dart int on every platform.
    final event = receiver.accept(wire.close(-1));
    expect(event.kind, wire.Frame.close);
    expect(event.channel, -1);
  });

  test('the conversation runs through one receiver', () {
    final receiver = wire.Receiver();
    final kinds = {
      'malformed': isA<wire.Malformed>(),
      'no_schema': isA<wire.NoSchema>(),
      'not_a_schema': isA<wire.NotASchema>(),
      'invalid': isA<wire.Invalid>(),
    };
    final statuses = {
      'malformed': isA<BadValue>(),
      'no_schema': isA<NotFound>(),
      'not_a_schema': isA<WrongKind>(),
      'invalid': isA<BadValue>(),
    };
    for (final step in _list('conversation')) {
      final frame = _hex(step['frame'] as String);
      final channel = (step['channel'] as int?) ?? 0;
      final event = step['event'] as String?;
      if (event != null) {
        final got = receiver.accept(frame);
        expect(got.kind.name, event, reason: '$step');
        expect(got.channel, channel);
        if (step.containsKey('tree')) {
          expect(describe(got.value!), step['tree'], reason: '$step');
          got.value!.close();
        } else {
          expect(got.value, isNull);
        }
        continue;
      }
      final refusal = step['refusal'] as String;
      try {
        receiver.accept(frame);
        fail('accepted a frame the vectors refuse: $step');
      } on wire.Refused catch (e) {
        expect(e, kinds[refusal], reason: '$step');
        expect(e, statuses[refusal], reason: '$step');
        expect(e.channel, channel, reason: '$step');
        expect(e.message, isNotEmpty);
        expect(e.toString(), isNot(contains('hunter2')));
        if (step.containsKey('kind')) expect(e.kind, step['kind']);
        if (step.containsKey('at')) {
          expect((e as wire.Malformed).at, step['at'], reason: '$step');
        }
        if (step.containsKey('key')) {
          expect((e as wire.Invalid).key, step['key'], reason: '$step');
        }
      }
    }
    receiver.close();
    receiver.close();
    expect(() => receiver.accept(wire.close(1)), throwsStateError);
  });
}
