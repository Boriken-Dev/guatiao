// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:guatiao/hook.dart';

/// Builds the guatiao library with cargo for the target being built: the
/// loader and pattern checks on, as the package's registry and schema
/// calls need.
void main(List<String> arguments) => buildHook(
  arguments,
  crate: 'guatiao',
  features: const <String>['load', 'regex'],
);
