// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Providers a server serves, called over guatiao's remote protocol from
/// plain Dart: a browser page, or any Dart program, with no native
/// library and no WebAssembly module.
///
/// ```dart
/// import 'package:guatiao/remote.dart';
///
/// final link = await RemoteLink.connect(
///   Uri.parse('ws://127.0.0.1:7000/'),
///   security: RemoteSecurity.none,
///   key: utf8.encode(key),
/// );
/// final catalog = await link.list();
/// final library = catalog.libraries.first;
/// await link.open(library);
/// final greeter = library.providers.first.table('greeter')!;
/// print(await greeter.call('greet', ['web']));   // {greeting: hello, web}
/// await link.close();
/// ```
///
/// The server listens with `guatiao-remote-server --listen ws://...
/// --security none`: a pre-shared key proved both ways, peers admitted by
/// address range, no encryption. Over `wss://` the link runs server TLS
/// (the key or a token proved inside TLS) or mutual TLS, the default.
/// Calls are typed by the kinds' descriptions
/// the catalog carries: a tree crosses as plain Dart, an object as a
/// [RemoteObject] (the peer's) or a [Served] (this side's), which the peer
/// may call back while a call is open.
library;

export 'src/remote/described.dart'
    show RemoteAnswer, RemoteArg, RemoteKind, RemoteSlot;
export 'src/remote/link.dart'
    show
        RemoteCallee,
        RemoteCatalog,
        RemoteChannel,
        RemoteDatagrams,
        RemoteLibrary,
        RemoteLink,
        RemoteObject,
        RemoteProvider,
        RemoteSecurity,
        RemoteTable,
        RemoteTls,
        RemoteTrust,
        Served;
export 'src/remote/codec.dart' show FrameFault, MalformedFrame;
