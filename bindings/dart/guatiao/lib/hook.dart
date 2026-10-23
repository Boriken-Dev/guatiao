// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Building a guatiao library from source in a package's build hook.
///
/// `hook/build.dart` of `guatiao`, `guatiao_serde` and `guatiao_intake`
/// is one call to [buildHook]: cargo builds the package's crate for the
/// target the build names, and the library is bundled as the package's
/// code asset, which `openSurface` finds at run time. Nothing is
/// downloaded but the crates cargo fetches; no binary is.
///
/// What an application configures, in its own `pubspec.yaml`:
///
/// ```yaml
/// hooks:
///   user_defines:
///     guatiao:
///       crate: native/myapp   # build this crate instead, one file for every surface
///       build: false          # build nothing: GUATIAO_LIBRARY, useLibrary or the platform search
/// ```
///
/// Supported API, versioned with the package; for the hooks of packages
/// over another guatiao library.
library;

export 'src/hook/cargo_build.dart'
    show HookFailure, buildHook, buildLibrary, missingTarget, rustTarget;
