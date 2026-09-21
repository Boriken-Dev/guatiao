// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';

/// No library of this base name was found by either route.
class LibraryNotFound implements Exception {
  LibraryNotFound(this.basename, this.filename, [this.cause]);

  final String basename;

  /// The platform's filename for [basename].
  final String filename;

  /// What the loader said, when it was reached at all.
  final Object? cause;

  @override
  String toString() {
    final tail = cause == null ? '' : ' ($cause)';
    return 'LibraryNotFound: could not find $filename: checked the '
        'libraries registered with guatiao.useLibrary, the '
        'GUATIAO_LIBRARY environment variable, guatiao.libraryDirectory '
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
  'guatiao_form'
];

/// One symbol per surface, to recognise a library that carries it.
const Map<String, String> _probe = <String, String>{
  'guatiao': 'guatiao_value_free',
  'guatiao_serde': 'guatiao_json_parse',
  'guatiao_form': 'guatiao_form_check',
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

/// Where [basename]'s library is (Q5).
///
/// `GUATIAO_LIBRARY` first, naming either a directory holding the
/// platform's filename or a file; then [libraryDirectory]; otherwise the
/// bare filename, which hands the search to the platform's own loader.
String resolveLibraryPath(String basename) {
  final filename = dllFilename(basename);
  final env = Platform.environment['GUATIAO_LIBRARY'];
  if (env != null && env.isNotEmpty) {
    final found = _inDirectory(env, filename);
    if (found != null) return found;
    if (!Directory(env).existsSync() && File(env).existsSync()) {
      // For the core library a file `GUATIAO_LIBRARY` names is taken as
      // it is, whatever it is called; a sibling is looked for beside it
      // under its own platform filename.
      if (basename == 'guatiao') return env;
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
  NativeLib._(this.path, this._dylib) : bindings = GuatiaoBindings(_dylib);

  /// Opens [basename]'s library, or throws [LibraryNotFound].
  factory NativeLib.open(String basename) {
    final path = resolveLibraryPath(basename);
    try {
      return NativeLib._(path, DynamicLibrary.open(path));
    } on Object catch (error) {
      throw LibraryNotFound(basename, dllFilename(basename), error);
    }
  }

  /// Where the library was loaded from.
  final String path;

  final DynamicLibrary _dylib;

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
  late final Pointer<NativeFinalizerFunction> valueFreeFinalizer =
      _dylib.lookup<NativeFinalizerFunction>('guatiao_value_free');

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
          basename, 'forSurfaces', 'not a guatiao surface');
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

NativeLib _libFor(String basename) =>
    _registered[basename] ?? (_opened[basename] ??= NativeLib.open(basename));

/// The `guatiao` library, loaded on first use.
NativeLib core() => _libFor('guatiao');

/// The `guatiao_serde` library, loaded on first use.
NativeLib serdeLib() => _libFor('guatiao_serde');

/// The `guatiao_form` library, loaded on first use.
NativeLib formLib() => _libFor('guatiao_form');
