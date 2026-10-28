// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Where Windows finds the libraries a file a `Registry` opens imports
/// (`guatiao_registry_set_import_search`). Unix ignores it: a library's
/// own `RUNPATH` (`$ORIGIN` for its directory) and `LD_LIBRARY_PATH`
/// decide there.
enum ImportSearch {
  /// Beside the file, beside the host's executable, in the system
  /// directory and in directories the process added; not the current
  /// directory or `PATH`. The default.
  library(0),

  /// [library]'s directories first; a file they leave an import missing
  /// for is opened again with Windows' standard search from its own
  /// directory: the system directories, the current directory and `PATH`,
  /// as a Dart host adding a Python bridge needs.
  path(1);

  const ImportSearch(this.code);

  /// The `GUATIAO_IMPORT_SEARCH_*` value.
  final int code;
}
