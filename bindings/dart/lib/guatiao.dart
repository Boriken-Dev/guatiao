// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Dart FFI bindings over the guatiao C ABI.
///
/// Importing this library touches no native library: every native call is
/// resolved lazily, on first use.
///
/// The two optional halves are their own entry points, imported with a
/// prefix so their verbs keep a namespace:
/// `import 'package:guatiao/serde.dart' as serde;` and
/// `import 'package:guatiao/form.dart' as form;`.
library;

export 'src/errors.dart'
    show
        AllocFailed,
        BadValue,
        Busy,
        Gone,
        GuatiaoException,
        InternalError,
        NotFound,
        NullArgument,
        Status,
        Unsupported,
        WrongKind;
export 'src/kinds.dart' show FloorMismatch, kindTable;
export 'src/library.dart'
    show
        LibraryNotFound,
        MissingSymbol,
        dllFilename,
        forgetLibraries,
        libraryDirectory,
        surfaces,
        useLibrary;
export 'src/registry.dart' show Instance, ProviderTable, Registry;
export 'src/serde.dart' show Format;
export 'src/value.dart'
    show Absent, ListRef, MapRef, Numbers, Ref, Tag, Value, ValueOwner, absent;
