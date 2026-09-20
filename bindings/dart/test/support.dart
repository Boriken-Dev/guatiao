// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Points the suite at the repository's own build output when nothing
/// else does, the way `bindings/python/tests/conftest.py` does.
library;

import 'dart:io';

import 'package:guatiao/guatiao.dart' as guatiao;

String? _libraryDir;

/// The directory the suite loads from: `GUATIAO_LIBRARY` when it is set,
/// otherwise the workspace's `target/debug`.
String libraryDir() {
  final cached = _libraryDir;
  if (cached != null) return cached;
  final env = Platform.environment['GUATIAO_LIBRARY'];
  if (env != null && env.isNotEmpty) return _libraryDir = env;
  final found = _workspaceTargetDebug();
  guatiao.libraryDirectory = found;
  return _libraryDir = found;
}

/// The repository root, found by walking up from this package.
String repoRoot() => _walkUp((dir) =>
    Directory('${dir.path}${Platform.pathSeparator}crates').existsSync() &&
    File('${dir.path}${Platform.pathSeparator}Cargo.toml').existsSync());

String _workspaceTargetDebug() {
  final root = repoRoot();
  return [root, 'target', 'debug'].join(Platform.pathSeparator);
}

String _walkUp(bool Function(Directory) matches) {
  var dir = Directory.current.absolute;
  for (var i = 0; i < 6; i++) {
    if (matches(dir)) return dir.path;
    final parent = dir.parent;
    if (parent.path == dir.path) break;
    dir = parent;
  }
  throw StateError('no repository root above ${Directory.current.path}');
}

/// A path under the repository root, joined for this platform.
String repoPath(List<String> parts) =>
    [repoRoot(), ...parts].join(Platform.pathSeparator);
