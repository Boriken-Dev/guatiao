// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The remote protocol's shared vectors: every frame both ways, every
/// argument list and answer typed by the greeter kinds' descriptions,
/// every malformed body refused where the file says, every proof; and the
/// wire encoding's own, which a peer's values are read against.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart' show BadValue, absent;
import 'package:guatiao/src/remote/args.dart';
import 'package:guatiao/src/remote/codec.dart';
import 'package:guatiao/src/remote/described.dart';
import 'package:guatiao/src/remote/proof.dart';
import 'package:guatiao/src/tape.dart'
    show dartToWire, wireToDart, wireToDartStrict;
import 'package:guatiao/src/value_common.dart' show Numbers, numberFromText;
import 'package:test/test.dart';

import 'support.dart';

late final Map<String, Object?> _vectors;
late final Map<String, Object?> _wire;
late final Map<String, RemoteKind> _kinds;

List<Map<String, Object?>> _list(String key) =>
    (_vectors[key] as List).cast<Map<String, Object?>>();

Uint8List _hex(String text) => Uint8List.fromList([
  for (var i = 0; i < text.length; i += 2)
    int.parse(text.substring(i, i + 2), radix: 16),
]);

String _toHex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();

/// The file's text with every integer of 16 digits or more quoted:
/// `jsonDecode` keeps no integer past 2^53 exact, and the vectors hold
/// the 64-bit limits.
String _quoteWide(String json) => json.replaceAllMapped(
  RegExp(r'(?<=[\[,]\s*)(-?\d{16,})(?=\s*[\],])'),
  (m) => '"${m[1]}"',
);

/// An integer the file writes, quoted when wide.
Object _int(Object? v) {
  if (v is String) return narrow(BigInt.parse(v));
  return v! as int;
}

/// The plain Dart a vector tree names.
Object? _tree(List<Object?> tree) {
  switch (tree[0] as String) {
    case 'absent':
      return absent;
    case 'null':
      return null;
    case 'bool':
      return tree[1] as bool;
    case 'number':
      final text = tree[1] as String;
      return int.tryParse(text) ?? double.parse(text);
    case 'string':
      return tree[1] as String;
    case 'bytes':
      return _hex(tree[1] as String);
    case 'list':
      return [for (final item in tree[1] as List) _tree(item as List<Object?>)];
    default:
      return {
        for (final pair in tree[1] as List)
          (pair as List)[0] as String: _tree(pair[1] as List<Object?>),
      };
  }
}

const _trust = {'local': 0, 'trusted': 1, 'secure': 2, 'server-tls': 3};

Frame _frameOf(Map<String, Object?> f) {
  switch (f['type']) {
    case 'hello':
      return Hello(
        _trust[f['trust']]!,
        _hex(f['nonce'] as String),
        _tree(f['options'] as List<Object?>),
      );
    case 'welcome':
      return Welcome(
        _hex(f['nonce'] as String),
        _hex(f['proof'] as String),
        _tree(f['options'] as List<Object?>),
      );
    case 'proof':
      return Proof(_hex(f['proof'] as String));
    case 'list':
      return ListCatalog(f['call'] as int);
    case 'open':
      return Open(
        f['call'] as int,
        f['library'] as String,
        f['stamp'] as String,
      );
    case 'close':
      return Close(f['library'] as String, f['stamp'] as String);
    case 'call':
      final target = f['target'] as List;
      final id = target[1] as int;
      return Call(
        f['call'] as int,
        f['parent'] as int,
        target[0] == 'table' ? Target.table(id) : Target.object(id),
        f['slot'] as int,
        _hex(f['args'] as String),
      );
    case 'reply':
      return Reply(
        f['call'] as int,
        f['status'] as int,
        _hex(f['body'] as String),
      );
    case 'cancel':
      return Cancel(f['call'] as int);
    case 'release':
      return Release(f['object'] as int);
    case 'ping':
      return Ping(f['n'] as int);
    case 'pong':
      return Pong(f['n'] as int);
    case 'goodbye':
      return Goodbye(f['status'] as int, f['message'] as String);
  }
  throw StateError('a frame type the vectors do not define: ${f['type']}');
}

/// A vector's argument or answer as the codec takes it.
Object? _valueOf(List<Object?> a) {
  switch (a[0] as String) {
    case 'str' || 'text':
      return a[1] as String;
    case 'int' || 'uint':
      return _int(a[1]);
    case 'bool':
      return a[1] as bool;
    case 'buffer':
      final x = a[1];
      return x is String ? _hex(x) : Uint8List(x as int);
    case 'map':
      return _tree(a[1] as List<Object?>);
    case 'object':
      return ObjectRef(a[2] as int, receivers: a[1] == 'receivers');
    case 'nothing':
      return null;
  }
  throw StateError('an argument form the vectors do not define: ${a[0]}');
}

/// Values compared by their wire bytes, everything else as it is.
Matcher _same(Object? expected) {
  if (expected is Map || expected is List && expected is! Uint8List) {
    return predicate<Object?>(
      (got) => _toHex(dartToWire(got)) == _toHex(dartToWire(expected)),
      'the same tree as $expected',
    );
  }
  return equals(expected);
}

void main() {
  setUpAll(() async {
    _vectors =
        jsonDecode(
              _quoteWide(
                await readRepoText([
                  'crates',
                  'guatiao-source-remote',
                  'tests',
                  'vectors',
                  'remote.json',
                ]),
              ),
            )
            as Map<String, Object?>;
    final document =
        jsonDecode(
              await readRepoText([
                'examples',
                'greeter_kind',
                'include',
                'greeter_kind.kinds.json',
              ]),
            )
            as Map<String, Object?>;
    _kinds = {
      for (final k in document['kinds'] as List)
        (k as Map)['name'] as String: RemoteKind.fromDescription(k),
    };
    _wire =
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

  group("the wire encoding's shared vectors, as a peer's values are read", () {
    // A tree as plain Dart, a number as the docs say it reads.
    Object? plain(List<Object?> tree) => switch (tree[0] as String) {
      'number' => numberFromText(tree[1] as String, Numbers.auto),
      'list' => [
        for (final item in tree[1] as List) plain(item as List<Object?>),
      ],
      'map' => {
        for (final pair in tree[1] as List)
          (pair as List)[0] as String: plain(pair[1] as List<Object?>),
      },
      _ => _tree(tree),
    };

    List<Map<String, Object?>> wire(String key) =>
        (_wire[key] as List).cast<Map<String, Object?>>();

    test('every value decodes to its tree', () {
      final cases = wire('values');
      expect(cases, hasLength(21));
      for (final c in cases) {
        final data = _hex(c['hex'] as String);
        final got = wireToDartStrict(data);
        final name = c['name'] as String;
        expect(got, _same(plain(c['tree'] as List<Object?>)), reason: name);
        expect(got, _same(wireToDart(data)), reason: name);
      }
    });

    test('every malformed value is refused, naming the byte', () {
      final cases = wire('malformed');
      expect(cases, hasLength(12));
      for (final c in cases) {
        final name = c['name'] as String;
        expect(
          () => wireToDartStrict(_hex(c['hex'] as String)),
          throwsA(
            isA<BadValue>().having(
              (e) => e.message,
              'message',
              endsWith(' at ${c['at']}'),
            ),
          ),
          reason: 'accepted bytes the vectors refuse: $name',
        );
      }
    });

    test("a peer's malformed value is a malformed frame", () {
      final body = (FrameWriter()..bytes(_hex('0207'))).done();
      expect(() => FrameReader(body).value(), throwsA(isA<MalformedFrame>()));
    });

    test('an integer past 53 bits is a BigInt under JavaScript', () {
      // 2^60 is a JavaScript number exactly, and still past what one holds
      // safely: the frames' own integers answer a BigInt for it there.
      for (final text in [
        '1152921504606846976',
        '-1152921504606846976',
        '9007199254740993',
      ]) {
        final big = BigInt.parse(text);
        final data = dartToWire(big);
        for (final got in [wireToDartStrict(data), wireToDart(data)]) {
          expect(got, jsNumbers ? big : big.toInt(), reason: text);
          expect(got, jsNumbers ? isA<BigInt>() : isA<int>(), reason: text);
        }
      }
      final safe = dartToWire(9007199254740991);
      expect(wireToDartStrict(safe), isA<int>());
    });

    test('a leading U+FEFF is text, kept as text', () {
      for (final text in ['\ufeffabc', '\ufeff\ufeff', '\ufeff']) {
        final data = dartToWire(text);
        expect(wireToDartStrict(data), text);
        expect(wireToDart(data), text);
        expect((wireToDartStrict(dartToWire({text: 1})) as Map).keys, [text]);
        expect(FrameReader((FrameWriter()..text(text)).done()).text(), text);
      }
    });
  });

  test('every frame decodes to its fields and encodes to its bytes', () {
    final cases = _list('frames');
    expect(cases, hasLength(20));
    for (final c in cases) {
      final name = c['name'] as String;
      final whole = _hex(c['hex'] as String);
      final length = ByteData.sublistView(whole).getUint32(0, Endian.little);
      final body = Uint8List.sublistView(whole, 4);
      expect(length, body.length, reason: name);
      final expected = _frameOf(c['frame'] as Map<String, Object?>);
      expect(_toHex(expected.encode()), _toHex(body), reason: name);
      final decoded = Frame.decode(body);
      expect(decoded.runtimeType, expected.runtimeType, reason: name);
      expect(_toHex(decoded.encode()), _toHex(body), reason: name);
      switch ((decoded, expected)) {
        case (Hello d, Hello e):
          expect(d.trust, e.trust, reason: name);
          expect(d.options, _same(e.options), reason: name);
        case (Welcome d, Welcome e):
          expect(d.proof, e.proof, reason: name);
          expect(d.options, _same(e.options), reason: name);
        case (Call d, Call e):
          expect(
            [d.call, d.parent, d.target, d.slot],
            [e.call, e.parent, e.target, e.slot],
            reason: name,
          );
        case (Open d, Open e):
          expect([d.call, d.library, d.stamp], [e.call, e.library, e.stamp]);
        case (Goodbye d, Goodbye e):
          expect([d.status, d.message], [e.status, e.message], reason: name);
        default:
      }
    }
  });

  test('every malformed body is refused where the file says', () {
    final cases = _list('malformed');
    expect(cases, hasLength(8));
    for (final c in cases) {
      final name = c['name'] as String;
      try {
        Frame.decode(_hex(c['body'] as String));
        fail('accepted a body the vectors refuse: $name');
      } on MalformedFrame catch (e) {
        expect(e.fault.label, c['error'], reason: name);
        expect(e, isA<BadValue>(), reason: name);
        if (c.containsKey('at')) expect(e.at, c['at'], reason: name);
      }
    }
  });

  RemoteSlot slotOf(Map<String, Object?> c) =>
      _kinds[c['kind'] as String]!.slot(c['slot'] as String);

  test(
    'every argument list encodes to its bytes and decodes to its values',
    () {
      final cases = _list('args');
      expect(cases, hasLength(11));
      for (final c in cases) {
        final name = c['name'] as String;
        final slot = slotOf(c);
        final args = [
          for (final a in c['args'] as List) _valueOf(a as List<Object?>),
        ];
        final bytes = encodeArgs(slot, args);
        expect(_toHex(bytes), c['hex'], reason: name);
        final back = decodeArgs(slot, _hex(c['hex'] as String));
        expect(back, hasLength(args.length), reason: name);
        for (var i = 0; i < args.length; i++) {
          final want = args[i];
          if (want is Uint8List) {
            expect((back[i]! as Uint8List).length, want.length, reason: name);
          } else {
            expect(back[i], _same(want), reason: name);
          }
        }
      }
    },
  );

  test(
    'every answer encodes to its bytes and decodes with its write-backs',
    () {
      final cases = _list('answers');
      expect(cases, hasLength(7));
      for (final c in cases) {
        final name = c['name'] as String;
        final slot = slotOf(c);
        final answer = _valueOf(c['answer'] as List<Object?>);
        final writebacks = [
          for (final w in c['writebacks'] as List) _valueOf(w as List<Object?>),
        ];
        // What the callee holds after the call: its write-backs in the places
        // of the lent trees and buffers.
        final args = List<Object?>.filled(slot.args.length, null);
        var w = 0;
        for (var i = 0; i < slot.args.length; i++) {
          final a = slot.args[i];
          final lentTree =
              {'value', 'map', 'list'}.contains(a.type) && a.pass == 'lent';
          if (lentTree || a.type == 'bytes_mut') args[i] = writebacks[w++];
        }
        expect(
          _toHex(encodeAnswer(slot, answer, args)),
          c['hex'],
          reason: name,
        );
        final back = decodeAnswer(slot, _hex(c['hex'] as String));
        expect(back.answer, _same(answer), reason: name);
        expect(back.writebacks, hasLength(writebacks.length), reason: name);
        for (var i = 0; i < writebacks.length; i++) {
          expect(back.writebacks[i], _same(writebacks[i]), reason: name);
        }
      }
    },
  );

  test('every proof is the HMAC-SHA256 the file holds', () {
    final cases = _list('proofs');
    expect(cases, hasLength(3));
    for (final c in cases) {
      final name = c['name'] as String;
      final key = _hex(c['key'] as String);
      final client = _hex(c['client'] as String);
      final server = _hex(c['server'] as String);
      final label = c['label'] as String;
      final proof = proofOf(key, label, client, server);
      expect(_toHex(proof), c['hex'], reason: name);
      expect(proves(key, label, client, server, proof), isTrue, reason: name);
      final wrong = Uint8List.fromList(proof)..[0] ^= 1;
      expect(proves(key, label, client, server, wrong), isFalse, reason: name);
      expect(proves(key, label, server, client, proof), isFalse, reason: name);
    }
    expect(utf8.encode(serverLabel), utf8.encode('guatiao-remote server'));
  });

  test('integers keep every bit at the 64-bit limits on every platform', () {
    final edges = <Object>[
      0,
      127,
      128,
      (1 << 31) * 4,
      9007199254740991,
      BigInt.parse('9007199254740993'),
      BigInt.parse('18446744073709551615'),
    ];
    for (final n in edges) {
      final w = FrameWriter()..uint(n);
      final r = FrameReader(w.done());
      expect(bigOf(r.uint()), bigOf(n), reason: '$n');
      r.end();
    }
    final signed = <Object>[
      -1,
      -9007199254740991,
      BigInt.parse('-9223372036854775808'),
      BigInt.parse('9223372036854775807'),
    ];
    for (final n in signed) {
      final w = FrameWriter()..int_(n);
      expect(bigOf(FrameReader(w.done()).int_()), bigOf(n), reason: '$n');
    }
    // Zigzagged, -2^62 is 2^63 - 1: the largest odd number a VM int holds.
    final minus62 = narrow(-(BigInt.one << 62));
    final w = FrameWriter()..int_(minus62);
    expect(FrameReader(w.done()).int_(), minus62);
    // Past 2^53 a JavaScript number is no longer exact, though 2^60 is one.
    final twoTo60 = BigInt.one << 60;
    final read = FrameReader((FrameWriter()..uint(twoTo60)).done()).uint();
    expect(read, jsNumbers ? twoTo60 : twoTo60.toInt());
    expect(read, jsNumbers ? isA<BigInt>() : isA<int>());
    expect(() => FrameWriter().uint(-1), throwsArgumentError);
    expect(() => FrameWriter().uint(BigInt.one << 64), throwsArgumentError);
  });
}
