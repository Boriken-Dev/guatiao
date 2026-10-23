// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What a package over another guatiao C surface builds on in a browser:
/// the web's counterpart of `package:guatiao/ffi.dart`, and importable on
/// the web only.
///
/// A page loads one WebAssembly module (`loadModule`), and every surface
/// is called in it: each module has a memory of its own, so a value built
/// in one is unreadable through another. `guatiao_serde` and
/// `guatiao_intake` find their functions in that module with
/// [openSurface] and call them with [WasmModule.call].
///
/// - [Address]: a pointer into the module's memory; [I64] a 64-bit
///   argument.
/// - [WasmModule]: [WasmModule.call], [WasmModule.provides], reading and
///   writing memory, [WasmModule.alloc] and [WasmModule.free] over
///   `guatiao_memory_alloc`, [WasmModule.defaultAlloc].
/// - [withScratch] and [Scratch]: memory for one call's arguments (a
///   `guatiao_str` over a text, an out slot), released together.
/// - [takeValue]: an answer read as plain Dart and freed with its block;
///   [freeNode]: what a node owns, freed; [freeRoot]: that and its block.
/// - [Value.adopt] takes over a root a call wrote into a block of one
///   value, [Value.pointer] and [Ref.requireNode] lend one, and
///   [Ref.borrowedFrom] views a node a call found.
/// - [wireToDart] and [dartToWire]: plain Dart read from and written as
///   wire bytes in Dart, for a call that crosses with a whole document.
///
/// A memory view goes stale when a call grows the memory: read through
/// [WasmModule] after each call, never through a view kept from before.
///
/// Supported API, versioned with the package.
library;

import 'src/status.dart';
import 'src/web/module.dart';

export 'src/status.dart' show GuatiaoException, Status, checkStatus;
export 'src/tape.dart' show dartToWire, wireToDart;
export 'src/value_common.dart' show Numbers, Tag;
export 'src/web/library_web.dart'
    show LibraryNotFound, MissingSymbol, core, openSurface;
export 'src/web/module.dart'
    show
        Address,
        I64,
        Scratch,
        WasmModule,
        allocSize,
        entrySize,
        entryValueAt,
        valueSize,
        viewSize,
        withScratch,
        wordSize;
export 'src/web/value_web.dart' show Ref, Value, freeRoot, strOf, takeValue;

/// Frees what [node] owns and leaves it absent; the block it sits in
/// stays the caller's.
void freeNode(Address node) =>
    checkStatus(WasmModule.current.call('guatiao_value_free', [node]));
