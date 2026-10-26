// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:async';
import 'dart:js_interop';
import 'dart:typed_data';

import '../status.dart';
import 'link.dart';

@JS('WebSocket')
extension type _WebSocket._(JSObject _) implements JSObject {
  external factory _WebSocket(String url);
  external set binaryType(String value);
  external int get readyState;
  external void send(JSAny data);
  external void close([int code]);
  external set onopen(JSFunction? handler);
  external set onmessage(JSFunction? handler);
  external set onclose(JSFunction? handler);
  external set onerror(JSFunction? handler);
}

extension type _MessageEvent._(JSObject _) implements JSObject {
  external JSAny? get data;
}

/// Opens the browser's WebSocket to [url] within [timeout].
Future<RemoteChannel> openWebSocket(Uri url, Duration timeout) {
  final opened = Completer<RemoteChannel>();
  final _WebSocket ws;
  try {
    ws = _WebSocket(url.toString());
  } catch (e) {
    return Future.error(
      Gone(Status.gone.code, 'the WebSocket to $url did not open: $e'),
    );
  }
  ws.binaryType = 'arraybuffer';
  final channel = _WebChannel(ws);
  final timer = Timer(timeout, () {
    if (opened.isCompleted) return;
    ws.close();
    opened.completeError(
      Gone(Status.gone.code, 'the WebSocket to $url did not open in time'),
    );
  });
  ws.onopen = ((JSAny _) {
    timer.cancel();
    if (!opened.isCompleted) opened.complete(channel);
  }).toJS;
  ws.onerror = ((JSAny _) {
    timer.cancel();
    if (!opened.isCompleted) {
      opened.completeError(
        Gone(Status.gone.code, 'the WebSocket to $url did not open'),
      );
    }
  }).toJS;
  ws.onmessage = ((_MessageEvent event) {
    final data = event.data;
    if (data.isA<JSArrayBuffer>()) {
      channel._incoming.add((data as JSArrayBuffer).toDart.asUint8List());
    } else {
      channel._incoming.addError(
        BadValue(Status.badValue.code, 'a text message: frames are binary'),
      );
    }
  }).toJS;
  ws.onclose = ((JSAny _) {
    timer.cancel();
    if (!opened.isCompleted) {
      opened.completeError(
        Gone(Status.gone.code, 'the WebSocket to $url closed as it opened'),
      );
    }
    channel._incoming.close();
  }).toJS;
  return opened.future;
}

final class _WebChannel implements RemoteChannel {
  _WebChannel(this._ws);

  final _WebSocket _ws;
  final _incoming = StreamController<Uint8List>();

  @override
  Stream<Uint8List> get messages => _incoming.stream;

  @override
  void send(Uint8List body) => _ws.send(body.toJS);

  @override
  Future<void> close() async {
    _ws.close(1000);
  }
}
