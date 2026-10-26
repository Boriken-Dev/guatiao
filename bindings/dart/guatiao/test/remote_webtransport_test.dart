// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A page's link over WebTransport (`https://`) to guatiao-remote-server on
/// loopback: the server's own certificate pinned by its hash
/// (`serverCertificateHashes`), with no certificate authority and no
/// browser flag. On the VM, which has no WebTransport, the URL is refused.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:guatiao/guatiao.dart' show BadValue, Gone, Unsupported;
import 'package:guatiao/remote.dart';
import 'package:test/test.dart';

import 'remote_key.dart';
import 'support.dart';

/// A fingerprint no certificate has.
final String _stranger = 'sha256:${'ab' * 32}';

Future<Object?> _greet(RemoteLink link, String name) async {
  final catalog = await link.list();
  final library = catalog.libraries.singleWhere(
    (l) => l.id == 'derived_greeter',
  );
  await link.open(library);
  return library.providers.first.table('greeter')!.call('greet', [name]);
}

void main() {
  test('the Dart VM refuses a WebTransport URL', () async {
    await expectLater(
      RemoteLink.connect(
        Uri.parse('https://127.0.0.1:9/'),
        security: RemoteSecurity.serverTls,
        key: utf8.encode(remoteKey),
      ),
      throwsA(isA<Unsupported>()),
    );
  }, testOn: 'vm');

  group('in a page', () {
    test(
      'server TLS: the certificate pinned by its hash, the page by the key',
      () async {
        final (url, hashes, stop) = await webTransportServer('server-tls');
        try {
          expect(url.scheme, 'https');
          expect(hashes, hasLength(2), reason: 'the one presented, the next');
          final link = await RemoteLink.connect(
            url,
            security: RemoteSecurity.serverTls,
            key: utf8.encode(remoteKey),
            tls: RemoteTls(pins: hashes),
          );
          expect(link.trust, RemoteTrust.serverTls);
          expect(await _greet(link, 'webtransport'), {
            'greeting': 'hello, webtransport',
          });
          await link.close();
          await expectLater(
            RemoteLink.connect(
              url,
              security: RemoteSecurity.serverTls,
              key: utf8.encode('not the key'),
              tls: RemoteTls(pins: hashes),
            ),
            throwsA(isA<BadValue>()),
          );
          await expectLater(
            RemoteLink.connect(
              url,
              security: RemoteSecurity.serverTls,
              key: utf8.encode(remoteKey),
              tls: RemoteTls(pins: [_stranger]),
              handshakeTimeout: const Duration(seconds: 5),
            ),
            throwsA(isA<Gone>()),
            reason: 'a hash the certificate does not have',
          );
          await expectLater(
            RemoteLink.connect(
              url,
              security: RemoteSecurity.serverTls,
              key: utf8.encode(remoteKey),
              tls: RemoteTls(pins: [hashes[1]]),
              handshakeTimeout: const Duration(seconds: 5),
            ),
            throwsA(isA<Gone>()),
            reason: 'the next certificate is not the one presented yet',
          );
        } finally {
          await stop();
        }
      },
    );

    test('no security: the key proved both ways inside QUIC', () async {
      final (url, hashes, stop) = await webTransportServer('none');
      try {
        final link = await RemoteLink.connect(
          url,
          security: RemoteSecurity.none,
          key: utf8.encode(remoteKey),
          tls: RemoteTls(pins: hashes),
        );
        expect(link.trust, RemoteTrust.trusted);
        expect(await _greet(link, 'none'), {'greeting': 'hello, none'});
        await link.close();
      } finally {
        await stop();
      }
    });

    test(
      'mutual TLS needs a client certificate the browser presents, which this profile holds none of',
      () async {
        final (url, hashes, stop) = await webTransportServer('mutual-tls');
        try {
          await expectLater(
            RemoteLink.connect(
              url,
              tls: RemoteTls(pins: hashes),
              handshakeTimeout: const Duration(seconds: 5),
            ),
            throwsA(isA<Gone>()),
          );
        } finally {
          await stop();
        }
      },
    );

    test(
      'datagrams beside the calls: each one sent comes back from a server that echoes them',
      () async {
        final (url, hashes, stop) = await datagramEchoServer();
        try {
          final link = await RemoteLink.connect(
            url,
            security: RemoteSecurity.none,
            key: utf8.encode(remoteKey),
            tls: RemoteTls(pins: hashes),
          );
          final datagrams = link.datagrams!;
          expect(datagrams.maxSize, greaterThan(1000));
          final received = StreamIterator(datagrams.received);
          // Datagrams may be lost: send until one comes back, each telling
          // which it was.
          final seen = <int>{};
          Future<bool>? next;
          for (var i = 0; i < 50 && seen.length < 3; i++) {
            datagrams.send(Uint8List.fromList([7, i, 0, 9]));
            next ??= received.moveNext();
            final came = await next.timeout(
              const Duration(milliseconds: 200),
              onTimeout: () => false,
            );
            if (came) {
              final back = received.current;
              expect([back[0], back[2], back[3]], [7, 0, 9]);
              seen.add(back[1]);
              next = null;
            }
          }
          expect(seen.length, greaterThanOrEqualTo(3));
          expect(
            () => datagrams.send(Uint8List(datagrams.maxSize + 1)),
            throwsA(isA<BadValue>()),
          );
          // Echoes still on the way arrive before a quiet moment.
          for (;;) {
            next ??= received.moveNext();
            final came = await next.timeout(
              const Duration(milliseconds: 300),
              onTimeout: () => false,
            );
            if (!came) break;
            next = null;
          }
          await link.close();
          expect(await next, isFalse, reason: 'the link ended: done');
          expect(
            () => datagrams.send(Uint8List.fromList([1])),
            throwsA(isA<Gone>()),
          );
        } finally {
          await stop();
        }
      },
    );

    test('a WebSocket link has no datagrams', () async {
      final (url, stop) = await remoteServer();
      try {
        final link = await RemoteLink.connect(
          url,
          security: RemoteSecurity.none,
          key: utf8.encode(remoteKey),
        );
        expect(link.datagrams, isNull);
        await link.close();
      } finally {
        await stop();
      }
    });

    test('a page presents no identity of its own', () async {
      await expectLater(
        RemoteLink.connect(
          Uri.parse('https://127.0.0.1:9/'),
          tls: const RemoteTls(identity: 'PEM'),
        ),
        throwsA(isA<ArgumentError>()),
      );
    });
  }, testOn: 'browser');
}
