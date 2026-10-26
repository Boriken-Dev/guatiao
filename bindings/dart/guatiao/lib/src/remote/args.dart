// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A call's arguments and a reply's answer, typed by the slot's
// description: nothing on the wire repeats what both sides know.
//
//   bool            1 byte, 0 or 1
//   i8..i64, isize  int (zigzag LEB128)    u8..u64, usize  uint
//   f32, f64        4 or 8 bytes, little-endian IEEE 754
//   str, text       text                   bytes  bytes
//   bytes_mut       uint: its length; the reply writes back bytes
//   value/map/list  value (a nullable value that is absent: ABSENT)
//   object          uint: id << 1, | 1 when the receiver's own
//
//   reply body := answer? writeback*   one per lent tree and per bytes_mut

import 'dart:typed_data';

import 'codec.dart';
import 'described.dart';

/// An object as it crosses: [id] in the numbering of the side that holds
/// it, [receivers] when that side is the receiver.
final class ObjectRef {
  const ObjectRef(this.id, {this.receivers = false});

  final int id;
  final bool receivers;

  int get code => receivers ? id * 2 + 1 : id * 2;

  static ObjectRef of(int code) => ObjectRef(code ~/ 2, receivers: code.isOdd);

  @override
  bool operator ==(Object other) =>
      other is ObjectRef && other.id == id && other.receivers == receivers;

  @override
  int get hashCode => Object.hash(id, receivers);

  @override
  String toString() => receivers ? "the receiver's object $id" : 'object $id';
}

/// The bounds of each integer type.
final Map<String, (BigInt, BigInt)> _ranges = () {
  BigInt two(int bits) => BigInt.one << bits;
  final out = <String, (BigInt, BigInt)>{};
  for (final bits in [8, 16, 32, 64]) {
    out['i$bits'] = (-two(bits - 1), two(bits - 1) - BigInt.one);
    out['u$bits'] = (BigInt.zero, two(bits) - BigInt.one);
  }
  out['isize'] = out['i64']!;
  out['usize'] = out['u64']!;
  return out;
}();

bool _signed(String type) => type.startsWith('i');

ArgumentError _mismatch(RemoteSlot slot, int i, String what) {
  final arg = slot.args[i];
  final name = arg.name.isEmpty ? '' : ' (`${arg.name}`)';
  return ArgumentError(
    'argument $i$name of `${slot.name}` is not $what, which the slot takes',
  );
}

void _putScalar(
  FrameWriter out,
  String type,
  Object? v,
  ArgumentError Function(String) wrong,
) {
  if (type == 'bool') {
    if (v is! bool) throw wrong('a bool');
    out.byte(v ? 1 : 0);
  } else if (type == 'f32' || type == 'f64') {
    if (v is! num) throw wrong('a number');
    final b = ByteData(8);
    if (type == 'f32') {
      b.setFloat32(0, v.toDouble(), Endian.little);
      out.raw(Uint8List.sublistView(b, 0, 4));
    } else {
      b.setFloat64(0, v.toDouble(), Endian.little);
      out.raw(Uint8List.sublistView(b));
    }
  } else {
    final range = _ranges[type];
    if (range == null) throw wrong('of a type these tables carry');
    if (v is! int && v is! BigInt) throw wrong('an integer');
    final big = bigOf(v!);
    if (big < range.$1 || big > range.$2) {
      throw wrong('within $type');
    }
    if (_signed(type)) {
      out.int_(v);
    } else {
      out.uint(v);
    }
  }
}

Object? _getScalar(FrameReader r, String type) {
  switch (type) {
    case 'bool':
      final at = r.at;
      final b = r.byte();
      if (b > 1) {
        throw MalformedFrame(
          FrameFault.field,
          'the field at $at is not a bool',
          at: at,
        );
      }
      return b == 1;
    case 'f32':
      return ByteData.sublistView(r.fixed(4)).getFloat32(0, Endian.little);
    case 'f64':
      return ByteData.sublistView(r.fixed(8)).getFloat64(0, Endian.little);
  }
  if (!_ranges.containsKey(type)) {
    throw MalformedFrame(
      FrameFault.field,
      'the field at ${r.at} is not a scalar these tables carry',
      at: r.at,
    );
  }
  return _signed(type) ? r.int_() : r.uint();
}

void _checkTree(String type, Object? v, ArgumentError Function(String) wrong) {
  if (type == 'map' && v is! Map) throw wrong('a map');
  if (type == 'list' && v is! List) throw wrong('a list');
}

Object? _tree(FrameReader r, String type) {
  final at = r.at;
  final v = r.value();
  if (type == 'map' && v is! Map) {
    throw MalformedFrame(
      FrameFault.field,
      'the field at $at is not a map',
      at: at,
    );
  }
  if (type == 'list' && v is! List) {
    throw MalformedFrame(
      FrameFault.field,
      'the field at $at is not a list',
      at: at,
    );
  }
  return v;
}

bool _isTree(String type) => type == 'value' || type == 'map' || type == 'list';

/// The bytes of [args] as [slot] types them; an object is its [ObjectRef].
Uint8List encodeArgs(RemoteSlot slot, List<Object?> args) {
  if (args.length != slot.args.length) {
    throw ArgumentError(
      '${args.length} arguments for `${slot.name}`, which takes '
      '${slot.args.length}',
    );
  }
  final out = FrameWriter();
  for (var i = 0; i < args.length; i++) {
    final type = slot.args[i].type;
    final v = args[i];
    ArgumentError wrong(String what) => _mismatch(slot, i, what);
    switch (type) {
      case 'str':
        if (v is! String) throw wrong('text');
        out.text(v);
      case 'bytes':
        if (v is! List<int>) throw wrong('bytes');
        out.bytes(v);
      case 'bytes_mut':
        if (v is! Uint8List) throw wrong('a Uint8List to write into');
        out.uint(v.length);
      case 'value' || 'map' || 'list':
        _checkTree(type, v, wrong);
        out.value(v);
      case 'object':
        if (v is! ObjectRef) throw wrong('an object');
        out.uint(v.code);
      default:
        _putScalar(out, type, v, wrong);
    }
  }
  return out.done();
}

/// The largest buffer a caller may ask a callee to fill: 64 MiB.
const int maxBuffer = 64 << 20;

/// A CALL's arguments as [slot] types them: a buffer as a zeroed
/// `Uint8List` of its length, an object as its [ObjectRef], an absent
/// nullable value as `absent`.
List<Object?> decodeArgs(RemoteSlot slot, Uint8List bytes) {
  final r = FrameReader(bytes);
  final out = <Object?>[];
  for (final arg in slot.args) {
    switch (arg.type) {
      case 'str':
        out.add(r.text());
      case 'bytes':
        out.add(Uint8List.fromList(r.bytes()));
      case 'bytes_mut':
        final at = r.at;
        final n = r.uint();
        if (n is! int || n > maxBuffer) {
          throw MalformedFrame(
            FrameFault.field,
            "the field at $at is not a buffer length within the link's limit",
            at: at,
          );
        }
        out.add(Uint8List(n));
      case 'value' || 'map' || 'list':
        out.add(_tree(r, arg.type));
      case 'object':
        out.add(ObjectRef.of(r.small('an object')));
      default:
        out.add(_getScalar(r, arg.type));
    }
  }
  r.end();
  return out;
}

/// A reply's body: [answer] as [slot] types it (an object as its
/// [ObjectRef]), then what the call wrote into each lent tree and buffer
/// of [args], in order.
Uint8List encodeAnswer(RemoteSlot slot, Object? answer, List<Object?> args) {
  final out = FrameWriter();
  final type = slot.answer?.type;
  ArgumentError wrong(String what) =>
      ArgumentError('`${slot.name}` answered something that is not $what');
  switch (type) {
    case null:
      break;
    case 'text':
      if (answer is! String) throw wrong('text');
      out.text(answer);
    case 'value' || 'map' || 'list':
      _checkTree(type, answer, wrong);
      out.value(answer);
    case 'object':
      if (answer is! ObjectRef) throw wrong('an object');
      out.uint(answer.code);
    default:
      _putScalar(out, type, answer, wrong);
  }
  for (var i = 0; i < slot.args.length; i++) {
    final arg = slot.args[i];
    if (_isTree(arg.type) && arg.pass == 'lent') {
      out.value(args[i]);
    } else if (arg.type == 'bytes_mut') {
      out.bytes(args[i]! as Uint8List);
    }
  }
  return out.done();
}

/// A reply's body read as [slot] types it: the answer (`null` for a slot
/// that answers nothing, an [ObjectRef] for an object), then one
/// write-back per lent tree and per buffer, in the arguments' order.
({Object? answer, List<Object?> writebacks}) decodeAnswer(
  RemoteSlot slot,
  Uint8List body,
) {
  final r = FrameReader(body);
  final type = slot.answer?.type;
  final Object? answer;
  switch (type) {
    case null:
      answer = null;
    case 'text':
      answer = r.text();
    case 'value' || 'map' || 'list':
      answer = _tree(r, type);
    case 'object':
      answer = ObjectRef.of(r.small('an object'));
    default:
      answer = _getScalar(r, type);
  }
  final writebacks = <Object?>[];
  for (final arg in slot.args) {
    if (_isTree(arg.type) && arg.pass == 'lent') {
      writebacks.add(_tree(r, arg.type));
    } else if (arg.type == 'bytes_mut') {
      writebacks.add(Uint8List.fromList(r.bytes()));
    }
  }
  r.end();
  return (answer: answer, writebacks: writebacks);
}
