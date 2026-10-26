// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A link over WebSocket to guatiao-remote-server on loopback, serving
/// `derived_greeter`: on the VM, and in a browser as a page would. Then
/// the link against a peer the suite plays itself, for what a well-behaved
/// server never does.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart'
    show BadValue, Gone, NotFound, Unsupported, WrongKind;
import 'package:guatiao/remote.dart';
import 'package:guatiao/src/remote/args.dart';
import 'package:guatiao/src/remote/codec.dart';
import 'package:test/test.dart';

import 'remote_key.dart';
import 'support.dart';

late final Uri _url;
Future<void> Function()? _stop;

Future<RemoteLink> _connect({String key = remoteKey}) =>
    RemoteLink.connect(_url, key: utf8.encode(key));

/// A link with `derived_greeter` opened, and its `Hello` provider.
Future<(RemoteLink, RemoteProvider)> _hello() async {
  final link = await _connect();
  final catalog = await link.list();
  final library = catalog.libraries.singleWhere(
    (l) => l.id == 'derived_greeter',
  );
  await link.open(library);
  final hello = library.providers.singleWhere(
    (p) => p.id == 'derived_greeter_hello',
  );
  return (link, hello);
}

/// Both ends of a link the suite plays the server of.
final class _Pipe implements RemoteChannel {
  final _toPeer = StreamController<Uint8List>();
  final _fromPeer = StreamController<Uint8List>();
  late final StreamIterator<Uint8List> sent = StreamIterator(_toPeer.stream);

  @override
  Stream<Uint8List> get messages => _fromPeer.stream;

  @override
  void send(Uint8List body) => _toPeer.add(body);

  @override
  Future<void> close() async {
    if (!_fromPeer.isClosed) await _fromPeer.close();
  }

  /// The next frame the link sent.
  Future<Frame> next() async {
    expect(await sent.moveNext(), isTrue, reason: 'the link sent nothing');
    return Frame.decode(sent.current);
  }

  void reply(Frame frame) => _fromPeer.add(frame.encode());
}

/// Kinds of the suite's own: `maker` hands the peer an `echo` this side
/// serves, or two listeners.
const List<Object?> _ownKinds = [
  {
    'name': 'maker',
    'object': false,
    'slots': [
      {
        'name': 'make',
        'args': [
          {'name': 'e', 'type': 'object', 'kind': 'echo', 'pass': 'taken'},
        ],
      },
      {
        'name': 'pair',
        'args': [
          {'name': 'a', 'type': 'object', 'kind': 'listener', 'pass': 'taken'},
          {'name': 'b', 'type': 'object', 'kind': 'listener', 'pass': 'taken'},
        ],
      },
    ],
  },
  {
    'name': 'echo',
    'object': true,
    'slots': [
      {
        'name': 'big',
        'answer': {'type': 'text'},
      },
      {
        'name': 'fill',
        'args': [
          {'name': 'dst', 'type': 'bytes_mut'},
        ],
        'answer': {'type': 'i64'},
      },
    ],
  },
];

/// A link at the local level over a [_Pipe], its catalog listed: one
/// provider whose greeter table fills only `greet` and `start`, and whose
/// `maker` table fills both its slots. [welcome] is what the peer offers.
Future<(RemoteLink, _Pipe, RemoteProvider)> _scripted({
  Duration? timeout,
  Map<String, Object?> welcome = const {},
}) async {
  final kinds =
      jsonDecode(
            await readRepoText([
              'examples',
              'greeter_kind',
              'include',
              'greeter_kind.kinds.json',
            ]),
          )
          as Map<String, Object?>;
  final pipe = _Pipe();
  final linking = RemoteLink.over(
    pipe,
    trust: RemoteTrust.local,
    timeout: timeout,
  );
  expect(await pipe.next(), isA<Hello>());
  pipe.reply(Welcome(Uint8List(16), Uint8List(0), welcome));
  final link = await linking;
  final listing = link.list();
  final ask = await pipe.next() as ListCatalog;
  final catalog = {
    'kinds': [...kinds['kinds'] as List, ..._ownKinds],
    'libraries': [
      {
        'id': 'scripted',
        'version': '1',
        'stamp': 's1',
        'providers': [
          {
            'id': 'p',
            'name': 'P',
            'tables': [
              {
                'kind': 'greeter',
                'table': 1,
                'slots': ['greet', 'start'],
              },
              {
                'kind': 'maker',
                'table': 2,
                'slots': ['make', 'pair'],
              },
            ],
          },
        ],
      },
    ],
  };
  pipe.reply(Reply(ask.call, 0, (FrameWriter()..value(catalog)).done()));
  return (link, pipe, (await listing).providers.single);
}

void main() {
  setUpAll(() async {
    final (url, stop) = await remoteServer();
    _url = url;
    _stop = stop;
  });

  tearDownAll(() => _stop?.call());

  test('the catalog lists the greeter kinds and their providers', () async {
    final link = await _connect();
    final catalog = await link.list();
    expect(
      catalog.kinds.keys,
      containsAll(['greeter', 'listener', 'conversation', 'counter', 'wide']),
    );
    expect(catalog.kinds['listener']!.object, isTrue);
    final library = catalog.libraries.singleWhere(
      (l) => l.id == 'derived_greeter',
    );
    expect(library.stamp, isNotEmpty);
    expect(
      library.providers.map((p) => p.id),
      contains('derived_greeter_hello'),
    );
    expect(await link.ping(), isA<Duration>());
    await link.close();
  });

  test('a page greets through the greeter kind', () async {
    final (link, hello) = await _hello();
    final greeter = hello.table('greeter')!;
    expect(await greeter.call('greet', ['web']), {'greeting': 'hello, web'});
    await expectLater(
      greeter.call('greet', ['']),
      throwsA(
        isA<BadValue>().having((e) => e.message, 'message', 'nobody to greet'),
      ),
    );
    expect(
      await greeter.call('greet_from', [
        {'name': 'map'},
      ]),
      {'greeting': 'hello, map'},
    );
    final counter = hello.table('counter')!;
    final first = await counter.call('count') as int;
    expect(await counter.call('count'), first + 1);
    await link.close();
  });

  test('a lent map comes back written in place', () async {
    final (link, hello) = await _hello();
    final config = <String, Object?>{};
    expect(await hello.table('greeter')!.call('check', [config]), isTrue);
    expect(config, {'name': 'world', 'greeting': 'hello, world'});
    await link.close();
  });

  test('a lent listener is called during the call, and only then', () async {
    final (link, hello) = await _hello();
    final heard = <String>[];
    final listener = Served('listener', {
      'heard': (args) => heard.add(args[0]! as String),
    });
    await hello.table('greeter')!.call('greet_into', ['lent', listener]);
    expect(heard, ['hello, lent']);
    expect(link.objectsHeld, 0, reason: 'lent for the call alone');
    await link.close();
  });

  test(
    'a handed listener is called back during calls on what it was handed to',
    () async {
      final (link, hello) = await _hello();
      final heard = <String>[];
      final listener = Served('listener', {
        'heard': (args) => heard.add(args[0]! as String),
      });
      final chat =
          await hello.table('greeter')!.call('start', [listener])
              as RemoteObject;
      expect(chat.kind.name, 'conversation');
      await chat.call('say', ['hi']);
      expect(heard, ['hi'], reason: 'heard while `say` was open');
      await chat.call('say', ['there']);
      expect(heard, ['hi', 'there']);
      expect(await chat.call('turns'), 2);
      final buffer = Uint8List(64);
      final n = await chat.call('read', [buffer]) as int;
      expect(utf8.decode(buffer.sublist(0, n)), 'hi\nthere');
      expect(link.objectsHeld, 1, reason: 'the listener, handed over');
      chat.release();
      await expectLater(chat.call('turns'), throwsA(isA<Gone>()));
      // The conversation dropped, it lets the listener go.
      for (var i = 0; i < 100 && link.objectsHeld > 0; i++) {
        await link.ping();
      }
      expect(link.objectsHeld, 0);
      await link.close();
    },
  );

  test('64-bit integers cross at their limits', () async {
    final (link, hello) = await _hello();
    final wide = hello.table('wide')!;
    final min = BigInt.parse('-9223372036854775808');
    final max = BigInt.parse('18446744073709551615');
    final i64 = await wide.call('echo_i64', [min]);
    expect(i64 is BigInt ? i64 : BigInt.from(i64! as int), min);
    expect(await wide.call('echo_u64', [max]), max);
    expect(await wide.call('echo_i64', [-5]), -5);
    await expectLater(
      wide.call('echo_u64', [-1]),
      throwsArgumentError,
      reason: 'below u64',
    );
    await link.close();
  });

  test('a wrong key is refused before the link is up', () async {
    await expectLater(
      _connect(key: 'not the key'),
      throwsA(
        isA<BadValue>().having(
          (e) => e.message,
          'message',
          contains("does not hold this link's key"),
        ),
      ),
    );
  });

  test('a closed link answers Gone', () async {
    final (link, hello) = await _hello();
    await link.close();
    expect(link.alive, isFalse);
    expect(await link.done, contains('closed'));
    await expectLater(
      hello.table('greeter')!.call('greet', ['late']),
      throwsA(isA<Gone>()),
    );
  });

  group('against a scripted peer', () {
    test(
      'a slot the far table leaves out is Unsupported, and nothing is sent',
      () async {
        final (link, pipe, provider) = await _scripted();
        final table = provider.table('greeter')!;
        expect(table.has('shout'), isFalse);
        await expectLater(
          table.call('shout', ['x']),
          throwsA(isA<Unsupported>()),
        );
        final ping = link.ping();
        final frame = await pipe.next();
        expect(frame, isA<Ping>(), reason: 'the refused call sent nothing');
        pipe.reply(Pong((frame as Ping).n));
        await ping;
        await link.close();
      },
    );

    test(
      'a call past its timeout is cancelled, and an object its late answer carries is released',
      () async {
        final (link, pipe, provider) = await _scripted(
          timeout: const Duration(milliseconds: 200),
        );
        final table = provider.table('greeter')!;
        final listener = Served('listener', {'heard': (_) => null});
        final starting = table.call('start', [listener]);
        final call = await pipe.next() as Call;
        await expectLater(starting, throwsA(isA<Gone>()));
        final cancel = await pipe.next() as Cancel;
        expect(cancel.call, call.call);
        expect(
          link.objectsHeld,
          1,
          reason: 'the listener went out: the peer has it',
        );
        // The late answer: a conversation of the peer's, nobody to take it.
        final slot = table.kind.slot('start');
        pipe.reply(
          Reply(call.call, 0, encodeAnswer(slot, const ObjectRef(5), [null])),
        );
        final release = await pipe.next() as Release;
        expect(release.object, 5);

        // Another, whose late answer names this side's own object.
        final again = table.call('start', [listener]);
        final second = await pipe.next() as Call;
        await expectLater(again, throwsA(isA<Gone>()));
        expect(await pipe.next(), isA<Cancel>());
        expect(link.objectsHeld, 2);
        final mine = decodeArgs(slot, second.args).single! as ObjectRef;
        pipe.reply(
          Reply(
            second.call,
            0,
            encodeAnswer(slot, ObjectRef(mine.id, receivers: true), [null]),
          ),
        );
        final ping = link.ping();
        final asked = await pipe.next() as Ping;
        pipe.reply(Pong(asked.n));
        await ping;
        expect(link.objectsHeld, 1, reason: 'the one answered back is dropped');
        await link.close();
      },
    );

    test('a call made after its handler returned names no parent', () async {
      final (link, pipe, provider) = await _scripted();
      final table = provider.table('greeter')!;
      final greet = table.kind.slot('greet');
      final later = Completer<Future<Object?>>();
      final listener = Served('listener', {
        'heard': (args) async {
          await table.call('greet', ['inside']);
          Timer(Duration.zero, () {
            later.complete(table.call('greet', ['after']));
          });
          return null;
        },
      });
      final starting = table.call('start', [listener]);
      final start = await pipe.next() as Call;
      final slot = table.kind.slot('start');
      final mine = decodeArgs(slot, start.args).single! as ObjectRef;
      pipe.reply(
        Reply(start.call, 0, encodeAnswer(slot, const ObjectRef(5), [null])),
      );
      await starting;
      // The peer tells the listener something, during no call of ours.
      pipe.reply(
        Call(
          40,
          0,
          Target.object(mine.id),
          0,
          (FrameWriter()..text('x')).done(),
        ),
      );
      final inside = await pipe.next() as Call;
      expect(inside.parent, 40, reason: 'made while `heard` runs');
      pipe.reply(Reply(inside.call, 0, encodeAnswer(greet, {'g': 1}, [null])));
      final heard = await pipe.next() as Reply;
      expect([heard.call, heard.status], [40, 0]);
      final after = await pipe.next() as Call;
      expect(after.parent, 0, reason: '`heard` has returned');
      pipe.reply(Reply(after.call, 0, encodeAnswer(greet, {'g': 2}, [null])));
      expect(await (await later.future), {'g': 2});
      await link.close();
    });

    test("an answer over the peer's max_frame is refused, not sent", () async {
      final (link, pipe, provider) = await _scripted(
        welcome: {'max_frame': 64},
      );
      final maker = provider.table('maker')!;
      final echo = Served('echo', {
        'big': (_) => 'x' * 200,
        // Breaks the buffer the answer writes back.
        'fill': (args) {
          args[0] = null;
          return 0;
        },
      });
      final making = maker.call('make', [echo]);
      final make = await pipe.next() as Call;
      final ref =
          decodeArgs(maker.kind.slot('make'), make.args).single! as ObjectRef;
      pipe.reply(Reply(make.call, 0, Uint8List(0)));
      await making;
      pipe.reply(Call(60, 0, Target.object(ref.id), 0, Uint8List(0)));
      final refused = await pipe.next() as Reply;
      expect([refused.call, refused.status], [60, 7]);
      expect(refused.words(), contains('over the 64'));
      pipe.reply(
        Call(61, 0, Target.object(ref.id), 1, (FrameWriter()..uint(8)).done()),
      );
      final broken = await pipe.next() as Reply;
      expect([broken.call, broken.status], [61, 100], reason: 'still answered');
      expect(link.alive, isTrue);
      await link.close();
    });

    test('a handed object is taken back when a later argument fails', () async {
      final (link, _, provider) = await _scripted();
      await expectLater(
        provider.table('maker')!.call('pair', [
          Served('listener', {}),
          Served('echo', {}),
        ]),
        throwsA(isA<WrongKind>()),
      );
      expect(link.objectsHeld, 0);
      await link.close();
    });

    test('a slot the kind does not have fails the Future', () async {
      final (link, _, provider) = await _scripted();
      final calling = provider.table('greeter')!.call('nope');
      await expectLater(calling, throwsA(isA<NotFound>()));
      await link.close();
    });

    test('a malformed frame ends the link with GOODBYE', () async {
      final (link, pipe, _) = await _scripted();
      pipe._fromPeer.add(Uint8List.fromList([0x7f]));
      final bye = await pipe.next() as Goodbye;
      expect(bye.status, 7);
      expect(await link.done, contains('malformed'));
      expect(link.alive, isFalse);
    });

    test("a call made during a call that returned is refused Gone", () async {
      final (link, pipe, _) = await _scripted();
      pipe.reply(Call(9, 4, const Target.object(1), 0, Uint8List(0)));
      final refused = await pipe.next() as Reply;
      expect(refused.call, 9);
      expect(refused.status, 12);
      await link.close();
    });

    test(
      'a call on an object this side never served is refused Gone',
      () async {
        final (link, pipe, _) = await _scripted();
        pipe.reply(Call(3, 0, const Target.object(77), 0, Uint8List(0)));
        final refused = await pipe.next() as Reply;
        expect([refused.call, refused.status], [3, 12]);
        expect(refused.words(), contains('let go'));
        await link.close();
      },
    );
  });
}
