// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'status.dart';

/// No library of this base name was found by any route.
class LibraryNotFound implements Exception {
  LibraryNotFound(this.basename, this.filename, [this.cause, this.variable]);

  final String basename;

  /// The platform's filename for [basename].
  final String filename;

  /// What the loader said, when it was reached at all.
  final Object? cause;

  /// The surface's own environment variable, consulted before
  /// `GUATIAO_LIBRARY`; `null` for the core.
  final String? variable;

  @override
  String toString() {
    final tail = cause == null ? '' : ' ($cause)';
    final own = variable == null ? '' : 'the $variable environment variable, ';
    return 'LibraryNotFound: could not find $filename: checked the '
        'libraries registered with guatiao.useLibrary, $own'
        'the GUATIAO_LIBRARY environment variable, guatiao.libraryDirectory '
        'and the platform library search path$tail';
  }
}

/// [name] is not exported by the library this build linked.
///
/// The library was found and loaded; it was simply built without the
/// feature that symbol belongs to.
class MissingSymbol implements Exception {
  MissingSymbol(this.path, this.name, [this.feature]);

  final String path;
  final String name;

  /// The build feature the symbol belongs to, when one is known.
  final String? feature;

  @override
  String toString() {
    final where = feature == null ? '' : " (feature '$feature')";
    return 'MissingSymbol: $path was not built with $name$where';
  }
}

/// Throws [UnsupportedError] unless a pointer is four or eight bytes
/// wide: the widths the ABI's layout is written for and tested on.
void requireWordSize(int bytes) {
  if (bytes != 4 && bytes != 8) {
    throw UnsupportedError(
      'package:guatiao reads structs laid out for 32- and 64-bit processes; '
      'this one has $bytes-byte pointers',
    );
  }
}

/// The platform's filename for a library of this base name.
String dllFilename(String basename) {
  if (Platform.isWindows) return '$basename.dll';
  if (Platform.isMacOS) return 'lib$basename.dylib';
  return 'lib$basename.so';
}

/// Every surface this package speaks, by the base name of the library
/// that carries it when each is its own file.
const List<String> surfaces = <String>[
  'guatiao',
  'guatiao_serde',
  'guatiao_intake',
];

/// One symbol per surface, to recognise a library that carries it.
const Map<String, String> _probe = <String, String>{
  'guatiao': 'guatiao_value_free',
  'guatiao_serde': 'guatiao_json_parse',
  'guatiao_intake': 'guatiao_intake_check',
};

/// Each surface's own environment variable, consulted before
/// `GUATIAO_LIBRARY`.
const Map<String, String> _variable = <String, String>{
  'guatiao_serde': 'GUATIAO_SERDE_LIBRARY',
  'guatiao_intake': 'GUATIAO_INTAKE_LIBRARY',
};

/// Where to look when `GUATIAO_LIBRARY` is unset.
///
/// A host that ships its own copies points this at their directory before
/// the first native call. A process cannot set its own environment, so an
/// embedder -- a Flutter app, a test suite -- has no other way to say
/// where its libraries are. `GUATIAO_LIBRARY` still wins.
String? libraryDirectory;

String? _inDirectory(String directory, String filename) {
  if (!Directory(directory).existsSync()) return null;
  final candidate = '$directory${Platform.pathSeparator}$filename';
  return File(candidate).existsSync() ? candidate : null;
}

bool _carries(String path, String symbol) {
  try {
    return DynamicLibrary.open(path).providesSymbol(symbol);
  } on Object {
    return false;
  }
}

/// Where [basename]'s library is.
///
/// [variable] first, when given and not `GUATIAO_LIBRARY`: a file it
/// names is taken as it is, a directory holds the platform's filename.
/// Then `GUATIAO_LIBRARY`: a directory holding the platform's filename,
/// or a file, taken when it exports [probe] (or when [probe] is `null`),
/// else the platform's filename beside it. Then [libraryDirectory];
/// otherwise the bare filename, which hands the search to the platform's
/// own loader.
String resolveLibraryPath(String basename, {String? variable, String? probe}) {
  final filename = dllFilename(basename);
  final own = variable == null || variable == 'GUATIAO_LIBRARY'
      ? null
      : Platform.environment[variable];
  if (own != null && own.isNotEmpty) {
    if (Directory(own).existsSync()) {
      final found = _inDirectory(own, filename);
      if (found != null) return found;
    } else if (File(own).existsSync()) {
      return own;
    }
  }
  final env = Platform.environment['GUATIAO_LIBRARY'];
  if (env != null && env.isNotEmpty) {
    final found = _inDirectory(env, filename);
    if (found != null) return found;
    if (!Directory(env).existsSync() && File(env).existsSync()) {
      // A file `GUATIAO_LIBRARY` names serves every surface it exports,
      // whatever it is called; for any other, the surface's own library
      // is looked for beside it.
      if (probe == null || _carries(env, probe)) return env;
      final beside = _inDirectory(File(env).parent.path, filename);
      if (beside != null) return beside;
    }
  }
  final configured = libraryDirectory;
  if (configured != null && configured.isNotEmpty) {
    final found = _inDirectory(configured, filename);
    if (found != null) return found;
  }
  return filename;
}

/// One loaded shared library: its bindings, its default allocator, and
/// the symbol lookups the rest of the package goes through.
///
/// Symbols are resolved lazily by [GuatiaoBindings]' own `late final`
/// lookups, so opening a library costs nothing beyond the load itself.
class NativeLib {
  NativeLib._(this.path, this._dylib) : bindings = GuatiaoBindings(_dylib) {
    requireWordSize(sizeOf<IntPtr>());
  }

  /// Opens [basename]'s library, found by [resolveLibraryPath] with
  /// [variable] and [probe], or throws [LibraryNotFound].
  factory NativeLib.open(String basename, {String? variable, String? probe}) {
    final path = resolveLibraryPath(basename, variable: variable, probe: probe);
    try {
      return NativeLib._(path, DynamicLibrary.open(path));
    } on Object catch (error) {
      throw LibraryNotFound(basename, dllFilename(basename), error, variable);
    }
  }

  /// Where the library was loaded from.
  final String path;

  final DynamicLibrary _dylib;

  /// The loaded library, for a package's own generated bindings over the
  /// surface it carries.
  DynamicLibrary get library => _dylib;

  /// The generated raw surface over this library.
  final GuatiaoBindings bindings;

  Pointer<guatiao_alloc>? _alloc;

  /// Whether this build exports [name].
  bool provides(String name) => _dylib.providesSymbol(name);

  /// Throws [MissingSymbol] unless this build exports [name].
  void require(String name, {String? feature}) {
    if (!_dylib.providesSymbol(name)) {
      throw MissingSymbol(path, name, feature);
    }
  }

  /// `guatiao_value_free` as a finalizer callback, for the backstop a
  /// `Value` attaches to itself.
  ///
  /// The C function returns a status the finalizer never reads.
  late final Pointer<NativeFinalizerFunction> valueFreeFinalizer = _dylib
      .lookup<NativeFinalizerFunction>('guatiao_value_free');

  /// `guatiao_wire_receiver_free` as a finalizer callback, for the
  /// backstop a channel `Receiver` attaches to itself.
  late final Pointer<NativeFinalizerFunction> wireReceiverFreeFinalizer = _dylib
      .lookup<NativeFinalizerFunction>('guatiao_wire_receiver_free');

  /// The default allocator, filled by `guatiao_alloc_default` once.
  ///
  /// Allocated for the life of the library and never released: every
  /// container built through it keeps a pointer to these 40 bytes.
  Pointer<guatiao_alloc> get alloc {
    final cached = _alloc;
    if (cached != null) return cached;
    require('guatiao_alloc_default');
    final out = calloc<guatiao_alloc>();
    final status = bindings.guatiao_alloc_default(out);
    if (status != 0) {
      calloc.free(out);
      throw StateError('guatiao_alloc_default failed with status $status');
    }
    return _alloc = out;
  }
}

final Map<String, NativeLib> _registered = <String, NativeLib>{};
final Map<String, NativeLib> _opened = <String, NativeLib>{};

/// Answers for a surface with a library that is already open, ahead of
/// every other route.
///
/// Two cases need it, and neither can be expressed by a path:
///
/// - **One library carrying several surfaces.** An application that links
///   the ABI into its own shared library exports `guatiao_*` from that
///   one file, under its own name. Registering it says so.
/// - **Symbols already in the process.** A statically linked build has no
///   file to open, and `DynamicLibrary.process()` is the library.
///
/// With no [forSurfaces], the library is registered for each surface it
/// actually exports, so a host registers what it has and nothing else.
/// Naming them explicitly skips that check and takes the caller's word.
/// [path] is only what an error message will quote.
void useLibrary(
  DynamicLibrary library, {
  Iterable<String>? forSurfaces,
  String path = '<registered>',
}) {
  final wanted = forSurfaces ?? surfaces;
  var any = false;
  for (final basename in wanted) {
    final probe = _probe[basename];
    if (probe == null) {
      throw ArgumentError.value(
        basename,
        'forSurfaces',
        'not a guatiao surface',
      );
    }
    if (forSurfaces == null && !library.providesSymbol(probe)) continue;
    _registered[basename] = NativeLib._(path, library);
    any = true;
  }
  if (!any) {
    throw ArgumentError.value(
      path,
      'library',
      'exports none of the guatiao surfaces (looked for ${_probe.values.join(', ')})',
    );
  }
}

/// Forgets every library registered with [useLibrary]; the next call
/// resolves again. For a test that swaps one out.
void forgetLibraries() {
  _registered.clear();
  _opened.clear();
}

/// The library carrying the surface [basename] (`'guatiao'`,
/// `'guatiao_serde'`, `'guatiao_intake'`), opened once on first use: the
/// one registered with [useLibrary] for it, else the one
/// [resolveLibraryPath] finds through the surface's own variable
/// (`GUATIAO_SERDE_LIBRARY`, `GUATIAO_INTAKE_LIBRARY`) and
/// `GUATIAO_LIBRARY`. Throws [LibraryNotFound].
///
/// A surface this package does not know names its own [variable] and
/// [probe].
NativeLib openSurface(String basename, {String? variable, String? probe}) =>
    _registered[basename] ??
    (_opened[basename] ??= NativeLib.open(
      basename,
      variable: variable ?? _variable[basename],
      probe: probe ?? _probe[basename],
    ));

/// The `guatiao` library, loaded on first use.
NativeLib core() => openSurface('guatiao');

/// Loads a WebAssembly module, on the web. A native process loads a
/// shared library instead, found as above: throws `Unsupported`.
Future<void> loadModule(Uri url) async => throw Unsupported(
  Status.unsupported.code,
  'loadModule is for the web: a native process loads the library '
  'GUATIAO_LIBRARY, useLibrary or the platform search finds',
);

/// Loads a WebAssembly module from its bytes, on the web. A native
/// process loads a shared library instead: throws `Unsupported`.
Future<void> loadModuleBytes(
  List<int> bytes, {
  String source = '<bytes>',
}) async => throw Unsupported(
  Status.unsupported.code,
  'loadModuleBytes is for the web: a native process loads the library '
  'GUATIAO_LIBRARY, useLibrary or the platform search finds',
);
