// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Where `guatiao_serde` is found: its own variable, then `GUATIAO_LIBRARY`
/// as a directory, a file carrying the surface, or a file beside it. Each
/// case runs `lookup_probe.dart` in a process with its own environment.
@TestOn('vm')
@Timeout(Duration(minutes: 3))
library;

import 'dart:io';

import 'package:guatiao/guatiao.dart' show dllFilename;
import 'package:test/test.dart';

import 'support.dart';

/// What `lookup_probe.dart` prints before its answer.
const _marker = 'lookup: ';

String _found(Map<String, String> environment) {
  final run = Process.runSync(
    Platform.resolvedExecutable,
    ['run', 'test${Platform.pathSeparator}lookup_probe.dart'],
    environment: {
      'GUATIAO_SERDE_LIBRARY': '',
      'GUATIAO_LIBRARY': '',
      ...environment,
    },
  );
  expect(run.exitCode, 0, reason: '${run.stderr}');
  // `dart run` may print its own progress first ("Running build hooks..."),
  // on the same line.
  final out = run.stdout as String;
  return out.substring(out.lastIndexOf(_marker) + _marker.length).trim();
}

String _file(String basename) =>
    [libraryDir(), dllFilename(basename)].join(Platform.pathSeparator);

void main() {
  test('its own variable names a file, whatever it is called', () {
    final intake = _file('guatiao_intake');
    expect(_found({'GUATIAO_SERDE_LIBRARY': intake}), intake);
  });

  test('its own variable names a directory', () {
    expect(
      _found({'GUATIAO_SERDE_LIBRARY': libraryDir()}),
      _file('guatiao_serde'),
    );
  });

  test('a GUATIAO_LIBRARY file carrying the surface serves it', () {
    final serde = _file('guatiao_serde');
    expect(_found({'GUATIAO_LIBRARY': serde}), serde);
  });

  test('a GUATIAO_LIBRARY file without the surface points beside it', () {
    expect(
      _found({'GUATIAO_LIBRARY': _file('guatiao')}),
      _file('guatiao_serde'),
    );
  });

  test('not found names its own variable', () {
    final missing = [libraryDir(), 'nowhere'].join(Platform.pathSeparator);
    final answer = _found({
      'GUATIAO_SERDE_LIBRARY': missing,
      'GUATIAO_LIBRARY': missing,
      'PATH': '',
      'LD_LIBRARY_PATH': '',
      'DYLD_LIBRARY_PATH': '',
    });
    expect(answer, contains('GUATIAO_SERDE_LIBRARY'));
    expect(answer, contains('GUATIAO_LIBRARY'));
  });
}
