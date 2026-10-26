// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * What a package over another guatiao C library builds on.
 *
 * `guatiao-serde` and `guatiao-intake` call their own libraries and pass
 * trees to and from this package through what is here, as may any package
 * binding a C library that takes and answers guatiao values. They never
 * load koffi themselves: one koffi, this package's, declares every library
 * and the structs passed by value (`guatiao_str`, `guatiao_bytes`,
 * `guatiao_object`).
 *
 * - {@link openSurface} finds and opens a surface's library and declares
 *   its exports; a {@link Surface} holds them and its default allocator.
 * - Memory: {@link alloc} and {@link release} for scratch a call writes,
 *   {@link str} and {@link bytesArg} for `guatiao_str` and `guatiao_bytes`
 *   arguments, readers ({@link u32}, {@link ptrAt}, {@link size},
 *   {@link stringText}, ...) and {@link freeNode}, which frees what a node
 *   owns through the core.
 * - A tree crosses as its root node: {@link adopt} makes a `Value` of a
 *   root a call wrote, {@link withNodes} and {@link nodeOf} lend values or
 *   plain data to a call as nodes, and {@link borrow} views a node another
 *   call found. {@link koffi} is the one koffi, for a package's own
 *   callbacks. Built by one library, a tree is read and freed by
 *   any other, since a value records its own allocator; the library that
 *   built it stays loaded while it lives.
 * - A whole document crosses once as wire bytes: {@link wireToJS} reads
 *   them into plain data and {@link jsToWire} writes them from it, here,
 *   with no tree built.
 *
 * Supported API, versioned with the package.
 *
 * @module
 */

export { ALLOC, ENTRY, OWNED, SIZE, Status, Tag, VALUE } from './abi.js';
export { BadValue, GuatiaoError, LibraryNotFound, MissingSymbol, raiseForStatus } from './errors.js';
export {
  type Declared,
  type Koffi,
  type StrArg,
  type SurfaceOptions,
  Surface,
  alloc,
  bytesArg,
  bytesAt,
  freeNode,
  i32,
  i64,
  koffi,
  libraryFilename,
  openSurface,
  ptrAt,
  release,
  setPtr,
  setU8,
  setU32,
  setU64,
  size,
  str,
  stringText,
  textAt,
  u32,
  u64,
  u8,
} from './native.js';
export { MAX_DEPTH, jsToWire, wireToJS } from './tape.js';
export { Ref, Value, adopt, borrow, nodeOf, withNodes } from './value.js';
