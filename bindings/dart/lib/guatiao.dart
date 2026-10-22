// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Dart FFI bindings over the guatiao C ABI.
///
/// Importing this library touches no native library: every native call is
/// resolved lazily, on first use.
///
/// Dates, times and spans are their own entry point, imported with a
/// prefix: `import 'package:guatiao/time.dart' as time;`. JSON, TOML and
/// YAML are the package `guatiao_serde`, forms `guatiao_intake`, each over
/// its own library and built on `package:guatiao/ffi.dart`.
library;

export 'src/bindings.g.dart' show guatiao_alloc, guatiao_value;
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
export 'src/schema.dart' show redact;
export 'src/time.dart' show TextForm, Unparsed;
export 'src/value.dart'
    show Absent, ListRef, MapRef, Numbers, Ref, Tag, Value, ValueOwner, absent;
