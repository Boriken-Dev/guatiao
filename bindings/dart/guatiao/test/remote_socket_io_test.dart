// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The Dart VM's WebSocket channel against a server of the suite's own.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';

import 'package:guatiao/guatiao.dart' show Gone;
import 'package:guatiao/src/remote/socket_io.dart';
import 'package:test/test.dart';

void main() {
  test(
    'a WebSocket that opens after its timeout is closed, not left open',
    () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      final upgraded = Completer<WebSocket>();
      server.listen((request) async {
        // Past the client's timeout, then the upgrade it asked for.
        await Future<void>.delayed(const Duration(milliseconds: 400));
        upgraded.complete(await WebSocketTransformer.upgrade(request));
      });
      final url = Uri.parse('ws://127.0.0.1:${server.port}/remote');
      await expectLater(
        openWebSocket(url, const Duration(milliseconds: 100)),
        throwsA(isA<Gone>()),
      );
      final ws = await upgraded.future;
      // The client closes the socket it gave up on.
      await ws.drain<void>().timeout(const Duration(seconds: 5));
      expect(ws.closeCode, WebSocketStatus.goingAway);
    },
  );
}
