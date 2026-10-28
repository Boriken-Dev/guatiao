// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Bindings over the guatiao C ABI: through `dart:ffi` on a native
/// platform, and over the WebAssembly module on the web, with the same
/// API.
///
/// Importing this library touches no native library: every native call is
/// resolved lazily, on first use. On the web, `await loadModule(url)`
/// first; what a browser cannot do (the registry, kind tables) throws
/// `Unsupported` saying so.
///
/// Dates, times and spans are their own entry point, imported with a
/// prefix: `import 'package:guatiao/time.dart' as time;`. JSON, TOML and
/// YAML are the package `guatiao_serde`, forms `guatiao_intake`, each over
/// its own library and built on `package:guatiao/ffi.dart` (on the web,
/// `package:guatiao/wasm.dart`).
library;

export 'src/kinds.dart'
    if (dart.library.js_interop) 'src/web/registry_web.dart'
    show FloorMismatch, kindTable;
export 'src/library.dart'
    if (dart.library.js_interop) 'src/web/library_web.dart'
    show
        LibraryNotFound,
        MissingSymbol,
        dllFilename,
        forgetLibraries,
        libraryDirectory,
        loadModule,
        loadModuleBytes,
        surfaces;
export 'src/import_search.dart' show ImportSearch;
export 'src/platform_native.dart'
    if (dart.library.js_interop) 'src/web/platform_web.dart';
export 'src/registry.dart'
    if (dart.library.js_interop) 'src/web/registry_web.dart'
    show Instance, ProviderTable, Registry;
export 'src/schema.dart'
    if (dart.library.js_interop) 'src/web/schema_web.dart'
    show redact;
export 'src/status.dart'
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
export 'src/time.dart' show TextForm, Unparsed;
export 'src/value.dart'
    if (dart.library.js_interop) 'src/web/value_web.dart'
    show ListRef, MapRef, Ref, Value, ValueOwner;
export 'src/value_common.dart' show Absent, Numbers, Tag, absent;
