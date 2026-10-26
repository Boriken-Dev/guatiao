// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';

import '../status.dart';
import 'link.dart';

/// The client a `wss://` link opens through when [tls] names pins or an
/// identity: pins alone decide which server certificate is trusted (no
/// certificate authority is), and the identity is presented when the
/// server asks for one.
HttpClient? _client(RemoteTls? tls) {
  if (tls == null) return null;
  final pins = {for (final p in tls.pins) p.toLowerCase()};
  final context = SecurityContext(withTrustedRoots: pins.isEmpty);
  final identity = tls.identity;
  if (identity != null) {
    final pem = utf8.encode(identity);
    context
      ..useCertificateChainBytes(pem)
      ..usePrivateKeyBytes(pem);
  }
  final client = HttpClient(context: context);
  if (pins.isNotEmpty) {
    client.badCertificateCallback = (cert, host, port) =>
        pins.contains('sha256:${sha256.convert(cert.der)}');
  }
  return client;
}

/// Opens a WebSocket to [url] within [timeout], its TLS as [tls] says.
Future<RemoteChannel> openWebSocket(
  Uri url,
  Duration timeout, [
  RemoteTls? tls,
]) async {
  try {
    final ws = await WebSocket.connect(
      url.toString(),
      customClient: _client(tls),
    ).timeout(timeout);
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
