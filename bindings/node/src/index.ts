// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Node.js and TypeScript bindings over the guatiao C ABI.
 *
 * Importing this module touches nothing native: koffi and the guatiao
 * library are loaded on the first call that needs them.
 *
 * @module
 */

export { Status, Tag } from './abi.js';
export {
  AllocFailed,
  BadValue,
  Busy,
  FloorMismatch,
  Gone,
  GuatiaoError,
  Internal,
  LibraryNotFound,
  MissingSymbol,
  NotFound,
  NullArgument,
  ParseError,
  Unparsed,
  Unsupported,
  WrongKind,
} from './errors.js';
export { useLibrary } from './native.js';
export { Instance, Registry, type ProviderTable, type RegistrableLibrary, type ScanOptions } from './registry.js';
export { redact } from './schema.js';
export { ListRef, MapRef, Ref, Value, type ReadOptions, type Step, type ValueOptions } from './value.js';
