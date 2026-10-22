// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What a package over another guatiao C library builds on.
///
/// `guatiao_serde` and `guatiao_intake` call their own libraries and pass
/// trees to and from this package through what is here, as may any
/// package binding a C library that takes and answers guatiao values:
///
/// - the generated bindings of `guatiao.h` ([GuatiaoBindings], every
///   struct and constant), which a package's own ffigen run reuses through
///   `import: { symbol-files: ['package:guatiao/ffi_symbols.yaml'] }`;
/// - [openSurface], finding and opening a surface's library, and
///   [NativeLib], one opened library with its default allocator;
/// - [strOf], a `guatiao_str` over a Dart string; [takeValue], an answer
///   read and freed; [freeNode], a node freed through the core library;
///   [checkStatus], a status thrown as its [GuatiaoException].
///
/// A tree crosses as its root node: [Value.adopt] takes over a root a call
/// wrote, [Value.pointer] and [Ref.node] lend one to a call, and
/// [Ref.borrowedFrom] views one another call found. Whichever library
/// built a tree, any of them frees it, since a value records its own
/// allocator; the library that built it stays loaded while it lives.
///
/// Supported API, versioned with the package.
library;

import 'dart:ffi';

import 'src/bindings.g.dart';
import 'src/errors.dart';
import 'src/library.dart';
import 'src/value.dart';

export 'src/bindings.g.dart';
export 'src/errors.dart' show GuatiaoException, Status, checkStatus;
export 'src/library.dart'
    show
        LibraryNotFound,
        MissingSymbol,
        NativeLib,
        dllFilename,
        libraryDirectory,
        openSurface,
        resolveLibraryPath,
        useLibrary;
export 'src/value.dart' show Ref, Tag, Value, strOf, takeValue;

/// Frees what [node] owns through the `guatiao` library and leaves it
/// absent; the box it sits in stays the caller's.
void freeNode(Pointer<guatiao_value> node) {
  checkStatus(core().bindings.guatiao_value_free(node));
}
