// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:guatiao/hook.dart';

/// Builds the guatiao_serde library with cargo for the target being built:
/// JSON with numbers kept as their text, TOML and YAML, every format this
/// package reads and writes. Nothing when the application's own crate
/// carries the surface.
void main(List<String> arguments) => buildHook(
  arguments,
  crate: 'guatiao-serde',
  features: const <String>['exact-numbers', 'toml', 'yaml'],
);
