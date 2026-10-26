// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import '../status.dart';
import 'link.dart';

/// WebTransport is a browser's: the Dart VM has none.
Future<RemoteChannel> openWebTransport(
  Uri url,
  Duration timeout,
  RemoteTls? tls,
  int maxFrame,
) => Future.error(
  Unsupported(
    Status.unsupported.code,
    "WebTransport (https://) is a browser's, and the Dart VM has none: "
    'connect over wss://',
  ),
);
