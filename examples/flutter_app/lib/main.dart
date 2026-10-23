// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:guatiao/ffi.dart' show openSurface;
import 'package:guatiao/guatiao.dart';
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

/// Where each surface's library was loaded from: the build hooks' files.
String libraries() => surfaces
    .map((surface) => '$surface ${openSurface(surface).path}')
    .join('; ');

void main() {
  String report;
  try {
    report = '${roundTrip()}\nlibraries: ${libraries()}';
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
