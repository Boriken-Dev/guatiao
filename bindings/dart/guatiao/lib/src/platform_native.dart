// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What `guatiao.dart` exports on a native platform only: the struct
/// types its pointers name, and registering an open library.
library;

export 'bindings.g.dart'
    show guatiao_alloc, guatiao_host_info, guatiao_library_info, guatiao_value;
export 'library.dart' show useLibrary;
