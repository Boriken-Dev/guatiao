// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:async';
import 'dart:js_interop';
import 'dart:typed_data';

import '../status.dart';
import 'link.dart';

@JS('WebTransport')
extension type _WebTransport._(JSObject _) implements JSObject {
  external factory _WebTransport(String url, JSObject options);
  external JSPromise<JSAny?> get ready;
  external JSPromise<JSAny?> get closed;
  external JSPromise<_BidirectionalStream> createBidirectionalStream();
  external void close([JSObject info]);
}

extension type _BidirectionalStream._(JSObject _) implements JSObject {
  external _ReadableStream get readable;
  external _WritableStream get writable;
}

extension type _ReadableStream._(JSObject _) implements JSObject {
  external _Reader getReader();
}

extension type _Reader._(JSObject _) implements JSObject {
  external JSPromise<_ReadResult> read();
}

extension type _ReadResult._(JSObject _) implements JSObject {
  external bool get done;
  external JSUint8Array? get value;
}

extension type _WritableStream._(JSObject _) implements JSObject {
  external _Writer getWriter();
}

extension type _Writer._(JSObject _) implements JSObject {
  external JSPromise<JSAny?> write(JSUint8Array chunk);
  external JSPromise<JSAny?> close();
}

extension type _Options._(JSObject _) implements JSObject {
  external factory _Options({JSArray<_Hash> serverCertificateHashes});
}

extension type _Hash._(JSObject _) implements JSObject {
  external factory _Hash({String algorithm, JSUint8Array value});
}

extension type _CloseInfo._(JSObject _) implements JSObject {
  external factory _CloseInfo({int closeCode, String reason});
}

@JS('globalThis.WebTransport')
external JSAny? get _webTransportClass;

/// How long closing waits for what was written to leave.
const Duration _closeWait = Duration(seconds: 2);

/// A fingerprint's 32 bytes: what `serverCertificateHashes` names.
Uint8List _hashOf(String pin) {
  final m = RegExp(r'^sha256:([0-9a-fA-F]{64})$').firstMatch(pin);
  if (m == null) {
    throw ArgumentError.value(
      pin,
      'pins',
      'not a certificate fingerprint, sha256:<64 hex digits>',
    );
  }
  final hex = m[1]!;
  return Uint8List.fromList([
    for (var i = 0; i < 32; i++)
      int.parse(hex.substring(2 * i, 2 * i + 2), radix: 16),
  ]);
}

/// Opens a WebTransport session to [url] (`https://host:port/path`) within
/// [timeout]: the server's certificate trusted by its fingerprint when
/// [tls] pins any (`serverCertificateHashes`), else as the browser trusts
/// any. One bidirectional stream carries each frame's body behind its
/// `u32` little-endian length; a frame over [maxFrame] bytes ends it.
Future<RemoteChannel> openWebTransport(
  Uri url,
  Duration timeout,
  RemoteTls? tls,
  int maxFrame,
) async {
  if (tls?.identity != null) {
    throw ArgumentError.value(
      tls,
      'tls',
      "a page presents the certificate the user installed in its browser, "
          'not an identity of its own: name pins alone',
    );
  }
  if (_webTransportClass == null) {
    throw Unsupported(
      Status.unsupported.code,
      'this browser has no WebTransport: connect over ws:// or wss://',
    );
  }
  final hashes = [
    for (final pin in tls?.pins ?? const <String>[])
      _Hash(algorithm: 'sha-256', value: _hashOf(pin).toJS),
  ];
  final _WebTransport transport;
  try {
    transport = _WebTransport(
      url.toString(),
      hashes.isEmpty
          ? JSObject()
          : _Options(serverCertificateHashes: hashes.toJS),
    );
  } catch (e) {
    throw Gone(
      Status.gone.code,
      'the WebTransport session to $url did not open: $e',
    );
  }
  // A session that fails rejects both; `closed` is read below once open.
  transport.closed.toDart.ignore();
  final _Reader reader;
  final _Writer writer;
  try {
    final stream = await transport.ready.toDart
        .then((_) => transport.createBidirectionalStream().toDart)
        .timeout(timeout);
    reader = stream.readable.getReader();
    writer = stream.writable.getWriter();
  } on TimeoutException {
    _quietly(() => transport.close());
    throw Gone(
      Status.gone.code,
      'the WebTransport session to $url did not open in time',
    );
  } catch (e) {
    _quietly(() => transport.close());
    throw Gone(
      Status.gone.code,
      'the WebTransport session to $url did not open: $e',
    );
  }
  final channel = _WtChannel(transport, writer);
  channel._pump(reader, maxFrame);
  transport.closed.toDart.then(
    (_) => channel._end(),
    onError: (Object _) => channel._end(),
  );
  return channel;
}

void _quietly(void Function() f) {
  try {
    f();
  } catch (_) {
    // Already closed.
  }
}

final class _WtChannel implements RemoteChannel {
  _WtChannel(this._transport, this._writer);

  final _WebTransport _transport;
  final _Writer _writer;
  final _incoming = StreamController<Uint8List>();
  bool _closing = false;

  @override
  Stream<Uint8List> get messages => _incoming.stream;

  void _end([Object? error]) {
    if (_incoming.isClosed) return;
    if (error != null) _incoming.addError(error);
    _incoming.close();
  }

  /// Reads the stream, handing on each whole frame's body.
  Future<void> _pump(_Reader reader, int maxFrame) async {
    var buf = Uint8List(0);
    var len = 0;
    try {
      for (;;) {
        final read = await reader.read().toDart;
        if (read.done) break;
        final chunk = read.value!.toDart;
        if (len + chunk.length > buf.length) {
          final grown = Uint8List(
            [
              len + chunk.length,
              buf.length * 2,
              1024,
            ].reduce((a, b) => a > b ? a : b),
          )..setRange(0, len, buf);
          buf = grown;
        }
        buf.setRange(len, len + chunk.length, chunk);
        len += chunk.length;
        while (len >= 4) {
          final size = ByteData.sublistView(
            buf,
            0,
            4,
          ).getUint32(0, Endian.little);
          if (size > maxFrame) {
            _end(
              BadValue(
                Status.badValue.code,
                'a frame of $size bytes is larger than this side reads ($maxFrame)',
              ),
            );
            _quietly(() => _transport.close());
            return;
          }
          if (len < 4 + size) break;
          if (_incoming.isClosed) return;
          _incoming.add(Uint8List.fromList(buf.sublist(4, 4 + size)));
          buf.setRange(0, len - 4 - size, buf, 4 + size);
          len -= 4 + size;
        }
      }
    } catch (_) {
      // The session ended; `closed` says how.
    }
    _end();
  }

  @override
  void send(Uint8List body) {
    if (_closing || _incoming.isClosed) return;
    final frame = Uint8List(4 + body.length);
    ByteData.sublistView(frame, 0, 4).setUint32(0, body.length, Endian.little);
    frame.setRange(4, frame.length, body);
    _writer.write(frame.toJS).toDart.ignore();
  }

  @override
  Future<void> close() async {
    if (_closing) return;
    _closing = true;
    // What was written leaves before the session ends, unless the peer
    // stops reading.
    try {
      await _writer.close().toDart.timeout(_closeWait);
    } catch (_) {
      // Ended already, or the peer reads nothing.
    }
    _quietly(() => _transport.close(_CloseInfo(closeCode: 0, reason: '')));
    _end();
  }
}
