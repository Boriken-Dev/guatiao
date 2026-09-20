// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Dart FFI bindings over the guatiao C ABI.
///
/// Importing this library touches no native library: every native call is
/// resolved lazily, on first use.
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
export 'src/library.dart' show LibraryNotFound, MissingSymbol, dllFilename;
export 'src/value.dart'
    show Absent, ListRef, MapRef, Numbers, Ref, Tag, Value, ValueOwner, absent;
