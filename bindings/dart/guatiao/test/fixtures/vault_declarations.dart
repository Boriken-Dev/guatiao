// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind whose answers are secrets, declared in Dart: sensitive_test.dart
// describes it into vault.kinds.json and checks vault_kind.dart is what
// that document writes.

import 'package:guatiao/describe.dart';
import 'package:guatiao/guatiao.dart' show MapRef;

/// A store of secrets.
@KindName('dart_vault')
abstract interface class Vault {
  /// A secret, as a tree.
  @fallible
  @sensitive
  ValueAnswer read(String key);

  /// Secrets by name.
  @fallible
  @sensitive
  MapAnswer every();

  /// A secret, as text.
  @sensitive
  String pin();

  /// Not a secret.
  String label();

  /// Lent a map, and changes nothing.
  void touch(Lent<MapRef> map);
}
