// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Mutual TLS over `wss://` from the Dart VM, against
/// guatiao-remote-server: the server pinned by its fingerprint, this side
/// presenting an identity the server pins. A page cannot do this (its TLS
/// is the browser's), so the suite runs on the VM alone.
@TestOn('vm')
library;

import 'dart:io';

import 'package:guatiao/guatiao.dart' show GuatiaoException;
import 'package:guatiao/remote.dart';
import 'package:test/test.dart';

import 'remote_server.dart';

late final Directory _dir;
late final RemoteServer _server;
late final String _client;
late final String _clientPem;

/// A server over `wss://` presenting the identity at [identity] (made when
/// absent), pinning [pin].
Future<RemoteServer> _wss(String identity, String pin) => startRemoteServer(
  listen: [
    '--listen',
    'wss://127.0.0.1:0',
    '--identity',
    identity,
    '--pin',
    pin,
  ],
);

void main() {
  setUpAll(() async {
    _dir = Directory.systemTemp.createTempSync('guatiao-remote-tls-');
    // The client's identity: what a server presenting it is known by.
    final maker = await _wss('${_dir.path}/client.pem', 'sha256:${'0' * 64}');
    _client = maker.fingerprint!;
    await maker.stop();
    _clientPem = File('${_dir.path}/client.pem').readAsStringSync();
    _server = await _wss('${_dir.path}/server.pem', _client);
  });

  tearDownAll(() async {
    await _server.stop();
    _dir.deleteSync(recursive: true);
  });

  test('a pinned client with its identity links over wss', () async {
    final link = await RemoteLink.connect(
      _server.url,
      tls: RemoteTls(pins: [_server.fingerprint!], identity: _clientPem),
    );
    expect(link.trust, RemoteTrust.secure);
    final catalog = await link.list();
    final library = catalog.libraries.singleWhere(
      (l) => l.id == 'derived_greeter',
    );
    await link.open(library);
    final greeter = library.providers.first.table('greeter')!;
    expect(await greeter.call('greet', ['tls']), {'greeting': 'hello, tls'});
    await link.close();
  });

  test('a server matching no pin is refused', () async {
    await expectLater(
      RemoteLink.connect(
        _server.url,
        tls: RemoteTls(pins: [_client], identity: _clientPem),
      ),
      throwsA(isA<GuatiaoException>()),
    );
  });

  test('a client the server does not pin is refused', () async {
    final stranger = File('${_dir.path}/server.pem').readAsStringSync();
    await expectLater(() async {
      final link = await RemoteLink.connect(
        _server.url,
        tls: RemoteTls(pins: [_server.fingerprint!], identity: stranger),
      );
      // TLS 1.3 lets a client finish before the server checks it: the
      // refusal arrives with the first answer.
      await link.list();
    }(), throwsA(isA<GuatiaoException>()));
  });

  test('wss:// runs TLS, never no security', () async {
    await expectLater(
      RemoteLink.connect(_server.url, security: RemoteSecurity.none, key: [1]),
      throwsA(isA<ArgumentError>()),
    );
  });
}
