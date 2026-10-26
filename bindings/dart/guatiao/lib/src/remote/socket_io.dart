// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import '../status.dart';
import 'link.dart';

/// Opens a WebSocket to [url] within [timeout].
Future<RemoteChannel> openWebSocket(Uri url, Duration timeout) async {
  try {
    final ws = await WebSocket.connect(url.toString()).timeout(timeout);
    return _IoChannel(ws);
  } on TimeoutException {
    throw Gone(Status.gone.code, 'the WebSocket to $url did not open in time');
  } on Exception catch (e) {
    throw Gone(Status.gone.code, 'the WebSocket to $url did not open: $e');
  }
}

final class _IoChannel implements RemoteChannel {
  _IoChannel(this._ws);

  final WebSocket _ws;

  @override
  late final Stream<Uint8List> messages = _ws.map((m) {
    if (m is List<int>) return m is Uint8List ? m : Uint8List.fromList(m);
    throw BadValue(Status.badValue.code, 'a text message: frames are binary');
  });

  @override
  void send(Uint8List body) => _ws.add(body);

  @override
  Future<void> close() async {
    // The peer answers a close; one that does not is not waited for.
    await _ws
        .close(WebSocketStatus.normalClosure)
        .timeout(const Duration(seconds: 5), onTimeout: () {});
  }
}
