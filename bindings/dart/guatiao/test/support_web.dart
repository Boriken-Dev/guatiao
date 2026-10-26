// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The suite's setup in a browser: a hybrid isolate on the test machine
/// serves the module over loopback and reads the repository's files.
library;

import 'dart:async';

import 'package:guatiao/guatiao.dart' as guatiao;
import 'package:stream_channel/stream_channel.dart';
import 'package:test/test.dart';

/// `true` under dart2js, where an `int` is a JavaScript number and holds
/// an integer exactly only up to 2^53.
const bool jsNumbers = identical(1, 1.0);

/// The suite runs in a browser.
const bool onWeb = true;

/// Serves the module `GUATIAO_WASM` names (default the application
/// library's release build) at `/module.wasm`, with the header a page on
/// another port fetches it with; answers each list of path parts sent on
/// the channel with that repository file's text.
const String _machine = r'''
import 'dart:convert';
import 'dart:io';
import 'package:stream_channel/stream_channel.dart';

Future<void> hybridMain(StreamChannel channel) async {
  var dir = Directory.current.absolute;
  while (!File('${dir.path}/Cargo.toml').existsSync() ||
      !Directory('${dir.path}/crates').existsSync()) {
    dir = dir.parent;
  }
  final named = Platform.environment['GUATIAO_WASM'];
  final module = File(named != null && named.isNotEmpty
      ? named
      : '${dir.path}/target/wasm32-unknown-unknown/release/myapp.wasm');
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  server.listen((request) async {
    request.response.headers.set('Access-Control-Allow-Origin', '*');
    if (request.uri.path == '/module.wasm' && module.existsSync()) {
      request.response.headers.contentType = ContentType('application', 'wasm');
      request.response.add(module.readAsBytesSync());
    } else {
      request.response.statusCode = 404;
    }
    await request.response.close();
  });
  channel.stream.listen((parts) {
    final file = File([dir.path, ...(parts as List).cast<String>()].join('/'));
    channel.sink.add(utf8.decode(file.readAsBytesSync()));
  });
  channel.sink.add(server.port);
}
''';

StreamChannel<Object?>? _channel;
StreamIterator<Object?>? _answers;
int? _port;

Future<StreamIterator<Object?>> _machineAnswers() async {
  final known = _answers;
  if (known != null) return known;
  final channel = _channel = spawnHybridCode(_machine, stayAlive: true);
  final answers = _answers = StreamIterator<Object?>(channel.stream);
  await answers.moveNext();
  _port = (answers.current! as num).toInt();
  return answers;
}

/// Where the test machine serves the module.
Future<Uri> moduleUrl() async {
  await _machineAnswers();
  return Uri.parse('http://127.0.0.1:$_port/module.wasm');
}

/// Loads the module once for the whole suite, through `loadModule`.
Future<void> setUpLibrary() async {
  if (!_loaded) {
    await guatiao.loadModule(await moduleUrl());
    _loaded = true;
  }
}

bool _loaded = false;

/// The repository's file at [parts], as text.
Future<String> readRepoText(List<String> parts) async {
  final answers = await _machineAnswers();
  _channel!.sink.add(parts);
  await answers.moveNext();
  return answers.current! as String;
}

/// guatiao-remote-server on loopback over WebSocket, started on the test
/// machine by `remote_server.dart`: the URL it serves, and what stops it.
Future<(Uri, Future<void> Function())> remoteServer() async {
  final channel = spawnHybridUri('remote_server.dart', stayAlive: true);
  final answers = StreamIterator<Object?>(channel.stream);
  await answers.moveNext();
  final said = answers.current! as String;
  if (said.startsWith('error: ')) throw StateError(said.substring(7));
  return (
    Uri.parse(said),
    () async {
      channel.sink.add('stop');
      await answers.moveNext();
      await channel.sink.close();
    },
  );
}

/// The directory of the native libraries: there are none on the web.
String libraryDir() =>
    throw UnsupportedError('a page loads a module, not a directory');
