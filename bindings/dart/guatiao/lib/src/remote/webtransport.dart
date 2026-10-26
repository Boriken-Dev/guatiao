// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// WebTransport as a message transport: the browser's on the web; the Dart
// VM has none.

export 'webtransport_io.dart'
    if (dart.library.js_interop) 'webtransport_web.dart'
    show openWebTransport;
