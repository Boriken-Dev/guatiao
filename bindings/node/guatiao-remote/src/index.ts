// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Providers a server serves, called over guatiao's remote protocol from a
 * browser page or any JavaScript program: no native library, no
 * WebAssembly module.
 *
 * ```ts
 * import { RemoteLink } from 'guatiao-remote';
 *
 * const link = await RemoteLink.connect('ws://192.0.2.21:7000/', { security: 'none', key });
 * const catalog = await link.list();
 * const library = catalog.libraries[0]!;
 * await link.open(library);
 * const greeter = library.providers[0]!.table('greeter')!;
 * console.log(await greeter.call('greet', ['web']));   // { greeting: 'hello, web' }
 * link.close();
 * ```
 *
 * The server listens with `guatiao-remote-server --listen ws://...
 * --security none`: a pre-shared key proved both ways, peers admitted by an
 * access control list, no encryption. Over `wss://` the link runs server
 * TLS or mutual TLS (the default), the TLS being the browser's. Calls are
 * typed by the kinds' descriptions the catalog carries.
 *
 * @module
 */

export { MalformedFrame } from './codec.js';
export type { FrameFault, Int } from './codec.js';
export { RemoteAnswer, RemoteArg, RemoteKind, RemoteSlot } from './described.js';
export type { Pass } from './described.js';
export {
  AllocFailed,
  BadValue,
  Busy,
  Gone,
  GuatiaoError,
  Internal,
  NotFound,
  NullArgument,
  Status,
  Unsupported,
  WrongKind,
  errorFor,
} from './errors.js';
export {
  InboundCall,
  RemoteCallee,
  RemoteCatalog,
  RemoteLibrary,
  RemoteLink,
  RemoteObject,
  RemoteProvider,
  RemoteTable,
  RemoteTrust,
  Served,
} from './link.js';
export type { CallOptions, Handler, LinkOptions, RemoteChannel, RemoteSecurity } from './link.js';
