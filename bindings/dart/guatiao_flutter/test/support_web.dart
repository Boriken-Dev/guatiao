// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The suite's setup in a browser: the module the runner copied into
/// `lib/web_module/` is served by `flutter test` under `/packages/`.
library;

import 'package:guatiao/guatiao.dart' as guatiao;

/// Loads the module once for the whole suite; call from `setUpAll`.
Future<void> setUpGuatiao() => guatiao.loadModule(
  Uri.base.resolve('/packages/guatiao_flutter/web_module/module.wasm'),
);
