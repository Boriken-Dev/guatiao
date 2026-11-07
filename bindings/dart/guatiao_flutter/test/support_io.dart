// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Points the suite at the library `GUATIAO_LIBRARY` names, else the
/// repository's own `target/debug`.
library;

import 'dart:io';

import 'package:guatiao/guatiao.dart' as guatiao;

/// Registers the library the suite loads; call once from `main`.
Future<void> setUpGuatiao() async {
  final env = Platform.environment['GUATIAO_LIBRARY'];
  // The package itself reads the variable.
  if (env != null && env.isNotEmpty) return;
  var dir = Directory.current.absolute;
  for (var i = 0; i < 6; i++) {
    if (Directory('${dir.path}/crates').existsSync() &&
        File('${dir.path}/Cargo.toml').existsSync()) {
      guatiao.libraryDirectory = '${dir.path}/target/debug';
      return;
    }
    dir = dir.parent;
  }
  throw StateError('no repository above ${Directory.current.path}');
}
