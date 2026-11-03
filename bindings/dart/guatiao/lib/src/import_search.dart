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

  /// [library]'s directories first; a library the file imports itself
  /// that they leave missing is then looked for in a source file's
  /// `import_dirs`, the Windows directory, the current directory and
  /// `PATH`, and mapped from there, as a Dart host adding a Python bridge
  /// with the Python installation on `PATH` needs. Everything else it
  /// imports is found as under [library].
  path(1);

  const ImportSearch(this.code);

  /// The `GUATIAO_IMPORT_SEARCH_*` value.
  final int code;
}
