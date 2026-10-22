// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Prints where `guatiao_serde` was found under this process's
/// environment, or the `LibraryNotFound` it raised; `lookup_test.dart`
/// runs it with the environment a case needs, which a process cannot set
/// for itself.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_serde/guatiao_serde.dart' as serde;

void main() {
  try {
    print(serde.library().path);
  } on LibraryNotFound catch (error) {
    print('$error'.replaceAll(RegExp(r'\s+'), ' '));
  }
}
