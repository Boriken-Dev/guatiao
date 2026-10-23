// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The wire encoding and channels against the shared vectors every
/// implementation reads: each value both ways, each malformed input
/// refused, the frames the builders write, and a conversation with every
/// refusal.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart';
import 'package:guatiao/src/bindings.g.dart';
import 'package:guatiao/wire.dart' as wire;
import 'package:test/test.dart';

import 'support.dart';

Map<String, Object?> _vectors() => jsonDecode(
      File(repoPath(['crates', 'guatiao', 'tests', 'vectors', 'wire.json']))
          .readAsStringSync(),
    ) as Map<String, Object?>;

List<Map<String, Object?>> _list(String key) =>
    (_vectors()[key] as List).cast<Map<String, Object?>>();

Uint8List _hex(String text) => Uint8List.fromList([
      for (var i = 0; i < text.length; i += 2)
        int.parse(text.substring(i, i + 2), radix: 16),
    ]);

String _toHex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

/// The channel a frame names, read from its bytes: JSON numbers past 2^53
/// do not survive `jsonDecode`, and a channel is 64 bits.
int _channelOf(Uint8List frame) {
  var n = 0;
  var shift = 0;
  for (var i = 1; i < frame.length; i++) {
    n |= (frame[i] & 0x7f) << shift;
    if (frame[i] & 0x80 == 0) break;
    shift += 7;
  }
  return n;
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
  setUpAll(libraryDir);

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

  test('the event is laid out as the header says', () {
    expect(sizeOf<guatiao_wire_event>(), 144);
    expect(wire.Frame.values.map((f) => f.code), [0, 1, 2]);
    final header = File(repoPath(['crates', 'guatiao', 'include', 'guatiao.h']))
        .readAsStringSync();
    for (final f in wire.Frame.values) {
      expect(
        header,
        contains('#define GUATIAO_FRAME_${f.name.toUpperCase()} ${f.code}\n'),
      );
    }
  });
}
