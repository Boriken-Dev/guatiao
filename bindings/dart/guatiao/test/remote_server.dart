// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// guatiao-remote-server on loopback over WebSocket, serving
/// `derived_greeter` for the greeter kinds: started by the VM suite
/// directly, and by a browser suite through a hybrid isolate.
///
/// The server is `GUATIAO_REMOTE_SERVER`, else the workspace's debug
/// build; the library is the one beside it. Missing, the suite fails
/// naming the build that makes them.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:stream_channel/stream_channel.dart';

import 'remote_key.dart';

Directory _repo() {
  var dir = Directory.current.absolute;
  while (!File('${dir.path}/Cargo.toml').existsSync() ||
      !Directory('${dir.path}/crates').existsSync()) {
    final up = dir.parent;
    if (up.path == dir.path) {
      throw StateError('no guatiao checkout above ${Directory.current.path}');
    }
    dir = up;
  }
  return dir;
}

String _library(String dir) {
  if (Platform.isWindows) return '$dir/derived_greeter.dll';
  if (Platform.isMacOS) return '$dir/libderived_greeter.dylib';
  return '$dir/libderived_greeter.so';
}

/// A running server: [url] to connect to, [stop] to end it.
final class RemoteServer {
  RemoteServer._(this.url, this._process, this._dir);

  final Uri url;
  final Process _process;
  final Directory _dir;
  Future<void>? _stopping;

  /// Ends the server and removes its key. Idempotent.
  Future<void> stop() => _stopping ??= () async {
    _process.kill();
    await _process.exitCode;
    _dir.deleteSync(recursive: true);
  }();
}

/// Starts the server and waits for the address it serves.
Future<RemoteServer> startRemoteServer() async {
  final repo = _repo();
  final named = Platform.environment['GUATIAO_REMOTE_SERVER'];
  final exe = Platform.isWindows ? '.exe' : '';
  final server = File(
    named != null && named.isNotEmpty
        ? named
        : '${repo.path}/target/debug/guatiao-remote-server$exe',
  );
  final library = File(_library(server.parent.path));
  if (!server.existsSync() || !library.existsSync()) {
    throw StateError(
      'no ${server.path} beside ${library.path}: build them with '
      '`cargo build -p guatiao-source-remote -p derived_greeter '
      '--features guatiao-source-remote/ws` '
      '(or name the server in GUATIAO_REMOTE_SERVER)',
    );
  }
  final dir = Directory.systemTemp.createTempSync('guatiao-remote-');
  final keyFile = File('${dir.path}/link.key')..writeAsStringSync(remoteKey);
  final process = await Process.start(server.path, [
    '--listen',
    'ws://127.0.0.1:0',
    '--trust',
    'trusted',
    '--range',
    '127.0.0.1',
    '--key-file',
    keyFile.path,
    '--kinds',
    '${repo.path}/examples/greeter_kind/include/greeter_kind.kinds.json',
    '--load',
    library.path,
  ]);
  unawaited(process.stdout.drain<void>());
  final serving = Completer<Uri>();
  final said = <String>[];
  process.stderr.transform(utf8.decoder).transform(const LineSplitter()).listen(
    (line) {
      said.add(line);
      final m = RegExp(r'serving (ws://\S+)').firstMatch(line);
      if (m != null && !serving.isCompleted) {
        serving.complete(Uri.parse('${m[1]}/'));
      }
    },
  );
  unawaited(
    process.exitCode.then((code) {
      if (!serving.isCompleted) {
        serving.completeError(
          StateError('the server exited ($code): ${said.join('\n')}'),
        );
      }
    }),
  );
  final url = await serving.future.timeout(const Duration(seconds: 30));
  return RemoteServer._(url, process, dir);
}

/// For a browser suite: starts the server and sends its URL; `stop` ends
/// it and is answered `stopped`, since the isolate may be killed as soon
/// as the suite is done with it.
Future<void> hybridMain(StreamChannel<Object?> channel) async {
  try {
    final server = await startRemoteServer();
    channel.sink.add(server.url.toString());
    channel.stream.listen((message) async {
      if (message == 'stop') {
        await server.stop();
        channel.sink.add('stopped');
      }
    }, onDone: server.stop);
  } catch (e) {
    channel.sink.add('error: $e');
  }
}
