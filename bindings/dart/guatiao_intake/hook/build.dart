// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:guatiao/hook.dart';

/// Builds the guatiao_intake library with cargo for the target being built,
/// with a schema's patterns checked. Nothing when the application's own
/// crate carries the surface.
void main(List<String> arguments) => buildHook(
  arguments,
  crate: 'guatiao-intake',
  features: const <String>['guatiao/regex'],
);
