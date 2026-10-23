// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Reading and writing a `Value` of package:guatiao as JSON, TOML or
/// YAML, over the `guatiao_serde` library, or on the web over the module
/// the page loaded.
///
/// Import it with a prefix: `import
/// 'package:guatiao_serde/guatiao_serde.dart' as serde;`. Resolution is
/// lazy, so importing touches no library.
library;

export 'src/serde.dart'
    if (dart.library.js_interop) 'src/web/serde_web.dart'
    hide finishParse;
