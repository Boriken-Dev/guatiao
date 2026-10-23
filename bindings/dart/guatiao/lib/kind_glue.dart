// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What the code `dart run guatiao:kind` writes is built on: one call
/// through a table ([CallScope]) and one call into a served object
/// ([ServeScope]). Supported API for that code, versioned with the
/// package; nothing a program writes by hand needs it.
library;

export 'src/kind_runtime.dart' show CallScope, ServeScope, SlotCall;
