// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The remote protocol's frames as bytes, with no I/O. A frame here is a
// body: a message transport (a WebSocket) carries it as one message, with
// no length in front.
//
//   uint  := LEB128, shortest form, at most 64 bits   int := uint, zigzagged
//   text  := len:uint UTF-8     bytes := len:uint octets
//   value := len:uint wire-encoded value
//
// Integers stay exact under dart2js: no shift reaches past bit 31, and a
// number past 2^49 is carried as a BigInt until it is known to fit an int.

import 'dart:convert';
import 'dart:typed_data';

import '../status.dart';
import '../tape.dart';

/// Why bytes are not a frame, named as the shared vectors name it.
enum FrameFault {
  unknownFrame('unknown_frame'),
  notThisProtocol('not_this_protocol'),
  version('version'),
  overlong('overlong'),
  truncated('truncated'),
  trailing('trailing'),
  notUtf8('not_utf8'),
  field('field'),
  wire('wire'),
  tooLarge('too_large');

  const FrameFault(this.label);

  /// The name the shared vectors give it.
  final String label;
}

/// Bytes a peer sent that are not a frame of this protocol: [fault] says
/// why, [at] the byte of the body it went wrong at, when it says.
class MalformedFrame extends BadValue {
  MalformedFrame(this.fault, String message, {this.at})
    : super(Status.badValue.code, message);

  final FrameFault fault;
  final int? at;
}

/// The four bytes a handshake starts with.
const List<int> magic = [0x47, 0x54, 0x52, 0x4d];

/// This protocol's version.
const int version = 1;

/// The largest body a peer reads before the handshake ends.
const int handshakeMaxFrame = 64 << 10;

/// Frame types.
abstract final class FrameType {
  static const int hello = 0x01;
  static const int welcome = 0x02;
  static const int proof = 0x03;
  static const int list = 0x04;
  static const int open = 0x05;
  static const int close = 0x06;
  static const int call = 0x10;
  static const int reply = 0x11;
  static const int cancel = 0x12;
  static const int release = 0x13;
  static const int ping = 0x14;
  static const int pong = 0x15;
  static const int goodbye = 0x16;
}

/// 2^0, 2^7, ... 2^42: the place of each LEB128 group an int carries.
final List<int> _places = () {
  final out = <int>[];
  var p = 1;
  for (var i = 0; i <= 6; i++) {
    out.add(p);
    p *= 128;
  }
  return out;
}();

/// 2^52: below it, twice a number is exact on every platform.
const int _twoTo52 = 4503599627370496;

final BigInt _u64Max = (BigInt.one << 64) - BigInt.one;
final BigInt _i64Min = -(BigInt.one << 63);
final BigInt _i64Max = (BigInt.one << 63) - BigInt.one;

/// Under dart2js an int is a JavaScript number, exact to 2^53.
const bool _jsNumbers = identical(1, 1.0);

/// [big] as an int where the platform holds it exactly: within 53 bits
/// under dart2js (where `isValidInt` also takes any number a double holds,
/// 2^60 among them), within 64 elsewhere.
Object narrow(BigInt big) =>
    (_jsNumbers ? big.bitLength <= 53 : big.isValidInt) ? big.toInt() : big;

/// [n] (an int or a BigInt) as a BigInt.
BigInt bigOf(Object n) => n is BigInt ? n : BigInt.from(n as int);

/// A body being written.
final class FrameWriter {
  Uint8List _buf = Uint8List(64);
  int _len = 0;

  void _room(int n) {
    if (_len + n <= _buf.length) return;
    var size = _buf.length * 2;
    while (size < _len + n) {
      size *= 2;
    }
    _buf = Uint8List(size)..setRange(0, _len, _buf);
  }

  void byte(int b) {
    _room(1);
    _buf[_len++] = b;
  }

  void raw(List<int> bytes) {
    _room(bytes.length);
    _buf.setRange(_len, _len + bytes.length, bytes);
    _len += bytes.length;
  }

  /// An unsigned integer, an int or a BigInt from 0 to 2^64 - 1.
  void uint(Object n) {
    if (n is int) {
      if (n < 0) {
        throw ArgumentError.value(
          n,
          'n',
          'is negative: not an unsigned integer',
        );
      }
      var v = n;
      while (v >= 128) {
        byte(v % 128 + 128);
        v = v ~/ 128;
      }
      byte(v);
      return;
    }
    final big = n as BigInt;
    if (big.isNegative || big > _u64Max) {
      throw ArgumentError.value(n, 'n', 'is not an unsigned 64-bit integer');
    }
    var v = big;
    final mask = BigInt.from(0x7f);
    while (v >= BigInt.from(128)) {
      byte((v & mask).toInt() | 0x80);
      v = v >> 7;
    }
    byte(v.toInt());
  }

  /// A signed integer, zigzagged: an int or a BigInt within 64 bits.
  void int_(Object n) {
    if (n is int && n > -_twoTo52 && n < _twoTo52) {
      uint(n >= 0 ? n * 2 : -n * 2 - 1);
      return;
    }
    final big = bigOf(n);
    if (big < _i64Min || big > _i64Max) {
      throw ArgumentError.value(n, 'n', 'is not a signed 64-bit integer');
    }
    uint(big.isNegative ? ((-big) << 1) - BigInt.one : big << 1);
  }

  void bytes(List<int> data) {
    uint(data.length);
    raw(data);
  }

  void text(String s) => bytes(utf8.encode(s));

  /// A value in the wire encoding, behind its length.
  void value(Object? v) => bytes(dartToWire(v));

  Uint8List done() => Uint8List.sublistView(_buf, 0, _len);
}

/// A body being read. Every refusal names the byte it went wrong at.
final class FrameReader {
  FrameReader(this.data);

  final Uint8List data;
  int at = 0;

  bool get isEmpty => at == data.length;

  /// What is left, unread.
  Uint8List rest() {
    final out = Uint8List.sublistView(data, at);
    at = data.length;
    return out;
  }

  void end() {
    if (!isEmpty) {
      throw MalformedFrame(
        FrameFault.trailing,
        'bytes follow the last field, from $at',
        at: at,
      );
    }
  }

  MalformedFrame _truncated(int where) => MalformedFrame(
    FrameFault.truncated,
    'the frame ends inside a field at $where',
    at: where,
  );

  int byte() {
    if (at >= data.length) throw _truncated(at);
    return data[at++];
  }

  Uint8List fixed(int n) {
    if (at + n > data.length) throw _truncated(at);
    final out = Uint8List.sublistView(data, at, at + n);
    at += n;
    return out;
  }

  /// An unsigned LEB128: an int, or a BigInt past what an int holds
  /// exactly. Refused when longer than its shortest form or wider than 64
  /// bits.
  Object uint() {
    final start = at;
    var n = 0;
    BigInt? big;
    for (var shift = 0; shift < 64; shift += 7) {
      final b = byte();
      final bits = b & 0x7f;
      if (shift == 63 && bits > 1) throw _overlong(start);
      if (big == null && shift <= 42) {
        n += bits * _places[shift ~/ 7];
      } else {
        big = (big ?? BigInt.from(n)) + (BigInt.from(bits) << shift);
      }
      if (b & 0x80 == 0) {
        if (b == 0 && shift > 0) throw _overlong(start);
        return big == null ? n : narrow(big);
      }
    }
    throw _overlong(start);
  }

  MalformedFrame _overlong(int start) => MalformedFrame(
    FrameFault.overlong,
    'an integer at $start is not in its shortest form',
    at: start,
  );

  /// An unsigned LEB128 that must be an int: an id, a count, a status.
  int small(String what) {
    final start = at;
    final n = uint();
    if (n is int) return n;
    throw MalformedFrame(
      FrameFault.field,
      'the field at $start is not $what',
      at: start,
    );
  }

  /// A zigzagged LEB128: an int, or a BigInt past what an int holds.
  Object int_() {
    final z = uint();
    // `z + 1` would wrap for the largest odd z an int holds.
    if (z is int) return z.isEven ? z ~/ 2 : -(z ~/ 2) - 1;
    final big = z as BigInt;
    return narrow(big.isEven ? big >> 1 : -((big + BigInt.one) >> 1));
  }

  /// A length that must fit what is left.
  int _len() {
    final start = at;
    final n = uint();
    if (n is! int || n > data.length - at) throw _truncated(start);
    return n;
  }

  Uint8List bytes() {
    final n = _len();
    final out = Uint8List.sublistView(data, at, at + n);
    at += n;
    return out;
  }

  String text() {
    final start = at;
    final raw = bytes();
    try {
      return utf8Text(raw);
    } on FormatException {
      throw MalformedFrame(
        FrameFault.notUtf8,
        'text at $start is not UTF-8',
        at: start,
      );
    }
  }

  /// A value in the wire encoding, read into plain Dart as strictly as
  /// the encoding states: a peer wrote it.
  Object? value() {
    final start = at;
    final raw = bytes();
    try {
      return wireToDartStrict(raw);
    } on BadValue catch (e) {
      throw MalformedFrame(
        FrameFault.wire,
        'the value at $start: ${e.message}',
        at: start,
      );
    }
  }
}

/// Who a call is for, in the receiver's numbering: a provider's table or
/// an object it holds.
final class Target {
  const Target.table(this.id) : object = false;
  const Target.object(this.id) : object = true;

  final int id;
  final bool object;

  int get code => object ? id * 2 + 1 : id * 2;

  static Target of(int code) =>
      code.isOdd ? Target.object(code ~/ 2) : Target.table(code ~/ 2);

  @override
  bool operator ==(Object other) =>
      other is Target && other.id == id && other.object == object;

  @override
  int get hashCode => Object.hash(id, object);

  @override
  String toString() => object ? 'object $id' : 'table $id';
}

/// One frame's body, decoded. A call's arguments and a reply's answer stay
/// bytes: they are read against the slot's description.
sealed class Frame {
  const Frame();

  int get type;

  void _fields(FrameWriter out);

  /// The body: the type byte, then the fields.
  Uint8List encode() {
    final out = FrameWriter()..byte(type);
    _fields(out);
    return out.done();
  }

  /// Reads one body.
  static Frame decode(Uint8List body) {
    final r = FrameReader(body);
    final type = r.byte();
    final Frame frame;
    switch (type) {
      case FrameType.hello:
      case FrameType.welcome:
        final m = r.fixed(4);
        for (var i = 0; i < 4; i++) {
          if (m[i] != magic[i]) {
            throw MalformedFrame(
              FrameFault.notThisProtocol,
              "the peer does not speak guatiao's remote protocol",
            );
          }
        }
        final v = r.byte();
        if (v != version) {
          throw MalformedFrame(
            FrameFault.version,
            'the peer speaks version $v, this side $version',
          );
        }
        if (type == FrameType.hello) {
          final at = r.at;
          final trust = r.byte();
          if (trust > 3) {
            throw MalformedFrame(
              FrameFault.field,
              'the field at $at is not a trust level',
              at: at,
            );
          }
          final nonce = Uint8List.fromList(r.fixed(16));
          frame = Hello(trust, nonce, r.value());
        } else {
          final nonce = Uint8List.fromList(r.fixed(16));
          final proof = Uint8List.fromList(r.bytes());
          frame = Welcome(nonce, proof, r.value());
        }
      case FrameType.proof:
        frame = Proof(Uint8List.fromList(r.bytes()));
      case FrameType.list:
        frame = ListCatalog(r.small('a call id'));
      case FrameType.open:
        frame = Open(r.small('a call id'), r.text(), r.text());
      case FrameType.close:
        frame = Close(r.text(), r.text());
      case FrameType.call:
        final call = r.small('a call id');
        final parent = r.small('a call id');
        final target = Target.of(r.small('a target'));
        final slot = r.small('a slot');
        return Call(call, parent, target, slot, r.rest());
      case FrameType.reply:
        final call = r.small('a call id');
        final at = r.at;
        final status = r.uint();
        if (status is! int || status > 0xffffffff) {
          throw MalformedFrame(
            FrameFault.field,
            'the field at $at is not a status',
            at: at,
          );
        }
        return Reply(call, status, r.rest());
      case FrameType.cancel:
        frame = Cancel(r.small('a call id'));
      case FrameType.release:
        frame = Release(r.small('an object id'));
      case FrameType.ping:
        frame = Ping(r.small('a ping number'));
      case FrameType.pong:
        frame = Pong(r.small('a ping number'));
      case FrameType.goodbye:
        final at = r.at;
        final status = r.uint();
        if (status is! int || status > 0xffffffff) {
          throw MalformedFrame(
            FrameFault.field,
            'the field at $at is not a status',
            at: at,
          );
        }
        frame = Goodbye(status, r.text());
      default:
        throw MalformedFrame(
          FrameFault.unknownFrame,
          'frame type 0x${type.toRadixString(16).padLeft(2, '0')} is not one '
          'this version defines',
        );
    }
    r.end();
    return frame;
  }
}

/// The client's first frame: `trust` 0 local, 1 trusted, 2 secure (mutual
/// TLS), 3 server TLS.
final class Hello extends Frame {
  const Hello(this.trust, this.nonce, this.options);
  final int trust;
  final Uint8List nonce;
  final Object? options;

  @override
  int get type => FrameType.hello;

  @override
  void _fields(FrameWriter out) => out
    ..raw(magic)
    ..byte(version)
    ..byte(trust)
    ..raw(nonce)
    ..value(options);
}

/// The server's answer to a HELLO.
final class Welcome extends Frame {
  const Welcome(this.nonce, this.proof, this.options);
  final Uint8List nonce;
  final Uint8List proof;
  final Object? options;

  @override
  int get type => FrameType.welcome;

  @override
  void _fields(FrameWriter out) => out
    ..raw(magic)
    ..byte(version)
    ..raw(nonce)
    ..bytes(proof)
    ..value(options);
}

/// The client's proof of the key.
final class Proof extends Frame {
  const Proof(this.proof);
  final Uint8List proof;

  @override
  int get type => FrameType.proof;

  @override
  void _fields(FrameWriter out) => out.bytes(proof);
}

/// Asks the peer's catalog.
final class ListCatalog extends Frame {
  const ListCatalog(this.call);
  final int call;

  @override
  int get type => FrameType.list;

  @override
  void _fields(FrameWriter out) => out.uint(call);
}

/// Opens a listed library.
final class Open extends Frame {
  const Open(this.call, this.library, this.stamp);
  final int call;
  final String library;
  final String stamp;

  @override
  int get type => FrameType.open;

  @override
  void _fields(FrameWriter out) => out
    ..uint(call)
    ..text(library)
    ..text(stamp);
}

/// Lets an opened library go.
final class Close extends Frame {
  const Close(this.library, this.stamp);
  final String library;
  final String stamp;

  @override
  int get type => FrameType.close;

  @override
  void _fields(FrameWriter out) => out
    ..text(library)
    ..text(stamp);
}

/// A call of a slot; [args] are typed by the slot's description.
final class Call extends Frame {
  const Call(this.call, this.parent, this.target, this.slot, this.args);
  final int call;
  final int parent;
  final Target target;
  final int slot;
  final Uint8List args;

  @override
  int get type => FrameType.call;

  @override
  void _fields(FrameWriter out) => out
    ..uint(call)
    ..uint(parent)
    ..uint(target.code)
    ..uint(slot)
    ..raw(args);
}

/// The answer to a call, a list or an open: the answer (status 0) or the
/// message.
final class Reply extends Frame {
  const Reply(this.call, this.status, this.body);
  final int call;
  final int status;
  final Uint8List body;

  @override
  int get type => FrameType.reply;

  @override
  void _fields(FrameWriter out) => out
    ..uint(call)
    ..uint(status)
    ..raw(body);

  /// A refusal's words.
  String words() {
    final r = FrameReader(body);
    final words = r.text();
    r.end();
    return words;
  }
}

/// The caller no longer waits for a call.
final class Cancel extends Frame {
  const Cancel(this.call);
  final int call;

  @override
  int get type => FrameType.cancel;

  @override
  void _fields(FrameWriter out) => out.uint(call);
}

/// An object the receiver gave is let go.
final class Release extends Frame {
  const Release(this.object);
  final int object;

  @override
  int get type => FrameType.release;

  @override
  void _fields(FrameWriter out) => out.uint(object);
}

/// Asks for a [Pong].
final class Ping extends Frame {
  const Ping(this.n);
  final int n;

  @override
  int get type => FrameType.ping;

  @override
  void _fields(FrameWriter out) => out.uint(n);
}

/// The answer to a [Ping].
final class Pong extends Frame {
  const Pong(this.n);
  final int n;

  @override
  int get type => FrameType.pong;

  @override
  void _fields(FrameWriter out) => out.uint(n);
}

/// The link is ending.
final class Goodbye extends Frame {
  const Goodbye(this.status, this.message);
  final int status;
  final String message;

  @override
  int get type => FrameType.goodbye;

  @override
  void _fields(FrameWriter out) => out
    ..uint(status)
    ..text(message);
}

/// A REPLY refusing [call] with [status] and [message].
Uint8List refusal(int call, int status, String message) =>
    Reply(call, status, (FrameWriter()..text(message)).done()).encode();
