// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A WebSocket as a message transport: dart:io's on the VM, the browser's
// on the web.

export 'socket_io.dart'
    if (dart.library.js_interop) 'socket_web.dart'
    show openWebSocket;
