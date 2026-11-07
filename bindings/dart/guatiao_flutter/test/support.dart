// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The suite's setup: a native build on the VM, the module in a browser.
library;

export 'support_io.dart' if (dart.library.js_interop) 'support_web.dart';
