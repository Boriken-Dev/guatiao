// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Loading the one WebAssembly module a page uses.
library;

import '../status.dart';
import 'module.dart';

export 'module.dart' show LibraryNotFound, MissingSymbol;

/// The filename of a surface's module when it is built on its own,
/// `<basename>.wasm`.
String dllFilename(String basename) => '$basename.wasm';

/// Every surface this package speaks.
const List<String> surfaces = <String>[
  'guatiao',
  'guatiao_serde',
  'guatiao_intake',
];

/// A directory to look in, on the VM. A page has none: set or not, it
/// changes nothing here.
String? libraryDirectory;

/// One symbol per surface, to recognise a module that carries it.
const Map<String, String> _probe = <String, String>{
  'guatiao': 'guatiao_value_free',
  'guatiao_serde': 'guatiao_json_parse',
  'guatiao_intake': 'guatiao_intake_check',
};

/// Fetches the module at [url] (relative to the page) and makes it the
/// one every surface uses. Throws [StateError] when one is loaded
/// already, [ArgumentError] when [url] answers no guatiao module.
Future<void> loadModule(Uri url) async {
  _refuseSecond();
  await WasmModule.fromUrl(url);
}

/// [bytes], a module's whole file, made the one every surface uses.
Future<void> loadModuleBytes(
  List<int> bytes, {
  String source = '<bytes>',
}) async {
  _refuseSecond();
  await WasmModule.fromBytes(bytes, source);
}

void _refuseSecond() {
  final loaded = WasmModule.loaded;
  if (loaded != null) {
    throw StateError(
      'a guatiao module is loaded already (${loaded.source}): a page uses '
      'one, and every value lives in its memory',
    );
  }
}

/// Forgets the loaded module, so another may be loaded. A value built in
/// it throws [StateError] from then on.
void forgetLibraries() => WasmModule.forget();

/// The loaded module, carrying the surface [basename], or
/// [LibraryNotFound]. [probe] names the symbol that says it does, for a
/// surface this package does not know.
WasmModule openSurface(String basename, {String? variable, String? probe}) {
  final module = WasmModule.loaded;
  if (module == null) {
    throw LibraryNotFound(basename, dllFilename(basename));
  }
  final symbol = probe ?? _probe[basename];
  if (symbol != null && !module.provides(symbol)) {
    throw LibraryNotFound(
      basename,
      dllFilename(basename),
      '${module.source} does not export $symbol',
    );
  }
  return module;
}

/// The loaded module, for the core surface.
WasmModule core() => openSurface('guatiao');

/// Throws [Unsupported]: what [feature] names needs files and a loader,
/// which a page does not have.
Never unsupportedOnTheWeb(String feature) => throw Unsupported(
  Status.unsupported.code,
  '$feature is not available on the web: a browser maps no libraries',
);
