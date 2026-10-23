// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:io';

import 'package:flutter/material.dart';
// Prefixed: Flutter has a `Listener` of its own.
import 'package:greeter_kind/greeter_kind.dart' as gk;
import 'package:guatiao/ffi.dart' show openSurface;
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kinds.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
import 'package:guatiao_serde/guatiao_serde.dart' as serde;

/// One call through each surface: a value built from Dart, written as JSON
/// and read back, and a field of the copy found by its path.
String roundTrip() {
  final value = Value.fromDart(<String, Object?>{
    'host': '10.0.0.1',
    'port': 5900,
    'agent': <Object?>[
      <String, Object?>{'name': 'first'},
      <String, Object?>{'name': 'second'},
    ],
  });
  try {
    final json = serde.dumps(value);
    final back = serde.loads(json);
    try {
      final name = intake.at(back, 'agent[1].name')?.toDart();
      return 'json $json, agent[1].name $name';
    } finally {
      back.close();
    }
  } finally {
    value.close();
  }
}

/// A greeter written in Dart: a class implementing the greeter kind's
/// interfaces, which `dart run guatiao:kind` wrote from its description.
final class AppGreeter extends Provider
    implements gk.Greeter, gk.GreeterGreetInto {
  @override
  Map<String, Object?> greet(String name) => {'greeting': 'hello, $name'};

  @override
  void greetInto(String name, gk.ListenerRemote listener) =>
      listener.heard('hello, $name');
}

/// A listener the app lends a greeter for one call.
final class Ear implements gk.Listener {
  final List<String> heardSoFar = <String>[];

  @override
  void heard(String what) => heardSoFar.add(what);
}

/// The Dart greeter served as a library and called back through its C
/// table: a map answered, a listener lent.
String kinds() {
  final library = Library(
    'flutter_app',
    '1.0.0',
    offers: [
      Offer.single(AppGreeter(), named: 'greeter', kinds: [gk.Greeter.kind]),
    ],
  );
  final reg = Registry('flutter-app', '1.0');
  try {
    reg.registerLibrary(library);
    final greeter = reg.bind('flutter_app_greeter', gk.Greeter.kind);
    final answer = greeter.greet('ana');
    final ear = Ear();
    try {
      greeter.greetInto('bo', ear);
      return 'greeter ${answer.toDart()}, heard ${ear.heardSoFar}';
    } finally {
      answer.close();
    }
  } finally {
    reg.close();
    library.close();
  }
}

/// Where each surface's library was loaded from: the build hooks' files.
String libraries() => surfaces
    .map((surface) => '$surface ${openSurface(surface).path}')
    .join('; ');

void main() {
  String report;
  try {
    report = '${roundTrip()}\nkinds: ${kinds()}\nlibraries: ${libraries()}';
  } on Object catch (error) {
    report = 'failed: $error';
  }
  // The log a run leaves: logcat's `flutter` tag, the console on desktop.
  for (final line in report.split('\n')) {
    debugPrint('guatiao example: $line');
  }
  // A scripted desktop run reads the console and needs the app to end.
  if (!Platform.isAndroid &&
      !Platform.isIOS &&
      Platform.environment.containsKey('GUATIAO_EXAMPLE_EXIT')) {
    exit(report.startsWith('failed') ? 1 : 0);
  }
  runApp(ExampleApp(report: report));
}

class ExampleApp extends StatelessWidget {
  const ExampleApp({super.key, required this.report});

  final String report;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'guatiao',
    home: Scaffold(
      appBar: AppBar(title: const Text('guatiao')),
      body: Padding(
        padding: const EdgeInsets.all(16),
        child: SelectableText(report),
      ),
    ),
  );
}
