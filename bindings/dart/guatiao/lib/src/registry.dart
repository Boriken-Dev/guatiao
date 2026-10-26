// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'kind_runtime.dart';
import 'library.dart' as native;
import 'provide.dart';
import 'value.dart';

/// What a provider speaks a kind through: its table, the table's size as
/// the library built it, and the `ctx` every slot takes first.
///
/// A null [table] with a zero [size] means there is no such table.
class ProviderTable {
  const ProviderTable(this.table, this.size, this.ctx);

  final Pointer<Void> table;
  final int size;
  final Pointer<Void> ctx;

  bool get isEmpty => table == nullptr;
}

/// A root this binding only borrows: read it, never free it, never write
/// through it.

/// An instance [Registry.create] built.
///
/// [close] runs the provider's own `destroy`; [ctx] is what every call
/// through that provider's kind tables takes first.
class Instance {
  Instance._(this._registry, this._key, this.ctx);

  final Registry _registry;
  final String _key;

  /// The provider's per-instance context.
  final Pointer<Void> ctx;

  bool _closed = false;

  bool get isClosed => _closed;

  /// This instance as [kind]: a remote whose calls go to it, valid until
  /// it is closed. [NotFound] when its provider serves no such table.
  R bind<R extends KindRemote>(Kind<R> kind) {
    final found = _registry.providerTable(_key, kind: kind.name);
    if (found.isEmpty) {
      throw NotFound(
        Status.notFound.code,
        '$_key serves no ${kind.name} table',
      );
    }
    return kind.remote(found.table, found.size, ctx);
  }

  /// Destroys the instance. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    // A freed registry has already let go of everything it held; calling
    // through its handle afterwards would address freed memory.
    if (_registry.isClosed) return;
    using((arena) {
      native.core().bindings.guatiao_registry_provider_destroy(
        _registry._handle,
        strOf(arena, _key).ref,
        ctx,
      );
    });
  }
}

/// A host's table of loaded libraries and the providers they offer.
///
/// Used from the isolate that made it, like the C handle it wraps.
class Registry implements Finalizable {
  /// Opens a registry this host answers to as `id` at `version`.
  factory Registry(String id, String version, {Pointer<guatiao_alloc>? alloc}) {
    final lib = native.core();
    lib.require('guatiao_registry_new', feature: 'load');
    final allocator = alloc ?? lib.alloc;
    final handle = using(
      (arena) => lib.bindings.guatiao_registry_new(
        strOf(arena, id).ref,
        strOf(arena, version).ref,
        allocator,
      ),
    );
    if (handle == nullptr) {
      throw StateError(
        'guatiao_registry_new failed: id or version is not UTF-8, or the '
        'allocator is incomplete',
      );
    }
    return Registry._(handle, allocator);
  }

  Registry._(this._handle, this._allocator);

  final Pointer<guatiao_registry> _handle;
  final Pointer<guatiao_alloc> _allocator;
  bool _closed = false;

  bool get isClosed => _closed;

  void _checkOpen() {
    if (_closed) throw StateError('this registry is closed');
  }

  /// Frees the registry. Idempotent.
  ///
  /// Every library it holds stays mapped; [unload] and [collect] are what
  /// unmap one.
  void close() {
    if (_closed) return;
    _closed = true;
    native.core().bindings.guatiao_registry_free(_handle);
  }

  /// One `(reg, ..., alloc, out) -> status` answer, as plain Dart, freed.
  Object? _answer(
    int Function(
      Pointer<guatiao_registry>,
      Pointer<guatiao_alloc>,
      Pointer<guatiao_value>,
    )
    call,
  ) {
    _checkOpen();
    final out = calloc<guatiao_value>();
    var filled = false;
    try {
      checkStatus(call(_handle, _allocator, out));
      filled = true;
      return takeValue(out);
    } finally {
      if (!filled) {
        native.core().bindings.guatiao_value_free(out);
        calloc.free(out);
      }
    }
  }

  Map<String, Object?> _answerMap(
    int Function(
      Pointer<guatiao_registry>,
      Pointer<guatiao_alloc>,
      Pointer<guatiao_value>,
    )
    call,
  ) => (_answer(call)! as Map).cast<String, Object?>();

  List<Map<String, Object?>> _answerRows(
    int Function(
      Pointer<guatiao_registry>,
      Pointer<guatiao_alloc>,
      Pointer<guatiao_value>,
    )
    call,
  ) => <Map<String, Object?>>[
    for (final row in _answer(call)! as List)
      (row! as Map).cast<String, Object?>(),
  ];

  // ---- loading ----------------------------------------------------

  /// Loads one library by path. The report says what was loaded or
  /// skipped.
  Map<String, Object?> loadFile(String path) => using((arena) {
    final view = strOf(arena, path).ref;
    return _answerMap(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_load_file(
        reg,
        view,
        alloc,
        out,
      ),
    );
  });

  /// Loads every library in `directory`.
  ///
  /// `rules` is the newline-separated scan filter -- `kind=greeter` and
  /// friends -- matched against what each library declares about itself.
  Map<String, Object?> scanDir(
    String directory, {
    bool descending = false,
    String? rules,
  }) => using((arena) {
    final dir = strOf(arena, directory).ref;
    final bindings = native.core().bindings;
    if (rules == null) {
      return _answerMap(
        (reg, alloc, out) => bindings.guatiao_registry_scan_dir(
          reg,
          dir,
          descending,
          alloc,
          out,
        ),
      );
    }
    final filter = strOf(arena, rules).ref;
    return _answerMap(
      (reg, alloc, out) => bindings.guatiao_registry_scan_dir_rules(
        reg,
        dir,
        descending,
        filter,
        alloc,
        out,
      ),
    );
  });

  /// Loads every library along a search path.
  Map<String, Object?> scanPath(
    String spec, {
    bool descending = false,
    String? rules,
  }) => using((arena) {
    final path = strOf(arena, spec).ref;
    final filter = strOf(arena, rules ?? '').ref;
    return _answerMap(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_scan_path(
        reg,
        path,
        descending,
        filter,
        alloc,
        out,
      ),
    );
  });

  // ---- what is loaded ----------------------------------------------

  /// Every loaded library.
  List<Map<String, Object?>> libraries() => _answerRows(
    (reg, alloc, out) =>
        native.core().bindings.guatiao_registry_libraries(reg, alloc, out),
  );

  /// Every provider that CLAIMS `kind`, best first. Every provider when
  /// `kind` is empty.
  List<Map<String, Object?>> providers([String kind = '']) => using((arena) {
    final view = strOf(arena, kind).ref;
    return _answerRows(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_providers(
        reg,
        view,
        alloc,
        out,
      ),
    );
  });

  /// Every provider that can actually serve `kind` right now.
  List<Map<String, Object?>> available([String kind = '']) => using((arena) {
    final view = strOf(arena, kind).ref;
    return _answerRows(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_available(
        reg,
        view,
        alloc,
        out,
      ),
    );
  });

  /// Why nothing can serve `kind`: `available`, and `why` with the
  /// providers that were passed over when there were any.
  Map<String, Object?> whyNot(String kind) => using((arena) {
    final view = strOf(arena, kind).ref;
    return _answerMap(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_why_not(
        reg,
        view,
        alloc,
        out,
      ),
    );
  });

  /// The provider filed under `key`.
  Map<String, Object?> provider(String key) => using((arena) {
    final view = strOf(arena, key).ref;
    return _answerMap(
      (reg, alloc, out) => native.core().bindings.guatiao_registry_provider(
        reg,
        view,
        alloc,
        out,
      ),
    );
  });

  /// The best provider that can serve `kind`.
  Map<String, Object?> best(String kind) => using((arena) {
    final view = strOf(arena, kind).ref;
    return _answerMap(
      (reg, alloc, out) =>
          native.core().bindings.guatiao_registry_best(reg, view, alloc, out),
    );
  });

  // ---- keys and priority ---------------------------------------------

  /// Sets the template providers are filed under (`%id`, `%name`,
  /// `%version`, `%library`, `%source`), and refiles what is loaded.
  /// `%source` is the source that served the library (empty for one the
  /// host loaded itself): `%source/%id` keeps the same library from
  /// several sources apart.
  Registry keyedBy(String template) {
    _checkOpen();
    using(
      (arena) => checkStatus(
        native.core().bindings.guatiao_registry_keyed_by(
          _handle,
          strOf(arena, template).ref,
        ),
      ),
    );
    return this;
  }

  /// Sets the template libraries are filed under (`%id`, `%version`,
  /// `%source`).
  Registry librariesKeyedBy(String template) {
    _checkOpen();
    using(
      (arena) => checkStatus(
        native.core().bindings.guatiao_registry_libraries_keyed_by(
          _handle,
          strOf(arena, template).ref,
        ),
      ),
    );
    return this;
  }

  /// Ranks the provider id, deciding what [best] picks.
  void setPriority(String id, int priority) {
    _checkOpen();
    using(
      (arena) => checkStatus(
        native.core().bindings.guatiao_registry_set_priority(
          _handle,
          strOf(arena, id).ref,
          priority,
        ),
      ),
    );
  }

  /// What this host ranked `id`. Zero unless it said otherwise.
  int priority(String id) {
    _checkOpen();
    return using(
      (arena) => native.core().bindings.guatiao_registry_priority(
        _handle,
        strOf(arena, id).ref,
      ),
    );
  }

  // ---- a provider's table and configuration ---------------------------

  /// The provider filed under [key] as [kind], over its own instance: a
  /// remote with a method per slot. [NotFound] when it serves no table for
  /// the kind, [FloorMismatch] when its table is not the one [kind]
  /// describes. A provider that only builds instances is bound through
  /// `create(...).bind(kind)`. Valid while its library is.
  R bind<R extends KindRemote>(String key, Kind<R> kind) {
    final found = providerTable(key, kind: kind.name);
    if (found.isEmpty) {
      throw NotFound(Status.notFound.code, '$key serves no ${kind.name} table');
    }
    return kind.bind(found);
  }

  /// Registers a library the host links: [entry] is its
  /// `guatiao_library_entry`, asked for its descriptor now. Answers the
  /// load report, as [loadFile] does.
  Map<String, Object?> registerEntry(
    String name,
    Pointer<
      NativeFunction<
        Pointer<guatiao_library_info> Function(Pointer<guatiao_host_info>)
      >
    >
    entry,
  ) {
    native.core().require('guatiao_registry_register_entry', feature: 'load');
    return using(
      (arena) => _answerMap(
        (reg, alloc, out) =>
            native.core().bindings.guatiao_registry_register_entry(
              reg,
              strOf(arena, name).ref,
              entry,
              alloc,
              out,
            ),
      ),
    );
  }

  /// Files a library written in Dart under its id. It is linked, not
  /// mapped: [unload] answers [WrongKind], and [retire] takes it out.
  Map<String, Object?> registerLibrary(Library library) =>
      registerEntry(library.id, library.entry);

  /// This registry's host block (`guatiao_host_info *`), which a library
  /// reaching other libraries through the host's services is handed.
  Pointer<guatiao_host_info> host() {
    _checkOpen();
    return native.core().bindings.guatiao_registry_host(_handle);
  }

  /// The table `key` speaks `kind` through, or its single table when no
  /// `kind` is named.
  ProviderTable providerTable(String key, {String? kind}) {
    _checkOpen();
    final bindings = native.core().bindings;
    final size = calloc<Size>();
    try {
      return using((arena) {
        final keyView = strOf(arena, key).ref;
        final table = kind == null
            ? bindings.guatiao_registry_provider_vtable(_handle, keyView, size)
            : bindings.guatiao_registry_provider_table(
                _handle,
                keyView,
                strOf(arena, kind).ref,
                size,
              );
        final ctx = bindings.guatiao_registry_provider_ctx(_handle, keyView);
        return ProviderTable(table, size.value, ctx);
      });
    } finally {
      calloc.free(size);
    }
  }

  /// A read-only, borrowed view of the provider's configuration schema,
  /// or `null` when it declares none.
  ///
  /// The registry owns it: never write through it, and do not use it
  /// after [retire] or [unload] takes its library out.
  Ref? providerConfig(String key) {
    _checkOpen();
    final root = using(
      (arena) => native.core().bindings.guatiao_registry_provider_config(
        _handle,
        strOf(arena, key).ref,
      ),
    );
    if (root == nullptr) return null;
    return Ref.borrowed(root);
  }

  /// The descriptions of the kinds the provider filed under `key` serves,
  /// and of every object kind their slots take or answer, as the document
  /// a kind crate commits beside its header (`{"format": 1, "kinds":
  /// [...]}`, plain data): what `dart run guatiao:kind` writes Dart from,
  /// for a kind this side has no `.kinds.json` for. `null` when the
  /// provider carries none (one written by hand, or a library built
  /// before descriptors carried them).
  ///
  /// Throws `NotFound` when nothing answers to `key`, `BadValue` when what
  /// it carries is not a description.
  Map<String, Object?>? kindDescriptions(String key) {
    _checkOpen();
    final out = calloc<guatiao_value>();
    var adopted = false;
    try {
      using((arena) {
        checkStatus(
          native.core().bindings.guatiao_registry_kind_descriptions(
            _handle,
            strOf(arena, key).ref,
            _allocator,
            out,
          ),
        );
      });
      if (out.ref.tag == Tag.absent.code) return null;
      final document = Value.adopt(out, alloc: _allocator);
      adopted = true;
      try {
        return (document.toDart()! as Map).cast<String, Object?>();
      } finally {
        document.close();
      }
    } finally {
      if (!adopted) {
        native.core().bindings.guatiao_value_free(out);
        calloc.free(out);
      }
    }
  }

  /// What the provider filed under `key` declared beyond its schema -- its
  /// legal notices, or its form under `'form'` (`providerFormKey` of
  /// package:guatiao_intake) -- as a
  /// [Value] the caller owns and closes, or `null` when it declares
  /// nothing.
  ///
  /// A copy, unlike [providerConfig]: it outlives the library. Throws
  /// `NotFound` when nothing answers to `key`.
  Value? providerMeta(String key) {
    _checkOpen();
    final out = calloc<guatiao_value>();
    var filled = false;
    try {
      using((arena) {
        checkStatus(
          native.core().bindings.guatiao_registry_provider_meta(
            _handle,
            strOf(arena, key).ref,
            _allocator,
            out,
          ),
        );
      });
      if (out.ref.tag == Tag.absent.code) return null;
      filled = true;
      return Value.adopt(out, alloc: _allocator);
    } finally {
      if (!filled) {
        native.core().bindings.guatiao_value_free(out);
        calloc.free(out);
      }
    }
  }

  /// Builds an instance of the provider filed under `key` from `config`
  /// -- a [Value], or anything [Value.fromDart] accepts.
  Instance create(String key, Object? config) {
    _checkOpen();
    final bindings = native.core().bindings;
    final Value? owned;
    final Value configValue;
    if (config is Value) {
      owned = null;
      configValue = config;
    } else {
      owned = Value.fromDart(config, alloc: _allocator);
      configValue = owned;
    }
    final out = calloc<Pointer<Void>>();
    final err = calloc<guatiao_provider_error>();
    try {
      final status = using(
        (arena) => bindings.guatiao_registry_provider_create(
          _handle,
          strOf(arena, key).ref,
          configValue.pointer,
          out,
          err,
        ),
      );
      checkProviderStatus(bindings, status, err);
      return Instance._(this, key, out.value);
    } finally {
      owned?.close();
      calloc.free(err);
      calloc.free(out);
    }
  }

  // ---- taking a library back out ---------------------------------------

  void _keyCall(String name, String key) {
    _checkOpen();
    final lib = native.core();
    lib.require(name, feature: 'load');
    final bindings = lib.bindings;
    using((arena) {
      final view = strOf(arena, key).ref;
      final status = switch (name) {
        'guatiao_registry_retire' => bindings.guatiao_registry_retire(
          _handle,
          view,
        ),
        'guatiao_registry_unload_unchecked' =>
          bindings.guatiao_registry_unload_unchecked(_handle, view),
        _ => bindings.guatiao_registry_unload(_handle, view),
      };
      checkStatus(status);
    });
  }

  /// Takes the library `key` names out of this registry and leaves it
  /// mapped.
  ///
  /// `key` is the LIBRARY key -- [librariesKeyedBy]'s template, `%id` by
  /// default -- not a provider key. Its providers leave and the key is
  /// free to load again. A [providerConfig] of one of its providers goes
  /// with it, because that copy belonged to the registry.
  ///
  /// The library is closed once nothing uses it: by [collect], or by this
  /// registry's next load, retire or unload, once no live [Instance] or
  /// object of its own remains and its own `unload` function agrees. A
  /// [ProviderTable] taken from it is valid until then and not after.
  void retire(String key) => _keyCall('guatiao_registry_retire', key);

  /// Closes every retired library nothing uses any more, and returns their
  /// library keys, oldest first.
  ///
  /// One whose `unload` function refuses stays retired and mapped, and is
  /// asked again next time; one with no such function stays mapped. No
  /// [ProviderTable] taken from a retired library is used again.
  List<String> collect() {
    native.core().require('guatiao_registry_collect', feature: 'load');
    return <String>[
      for (final key
          in _answer(
                (reg, alloc, out) => native
                    .core()
                    .bindings
                    .guatiao_registry_collect(reg, alloc, out),
              )!
              as List)
        key! as String,
    ];
  }

  // ---- sources: libraries that come and go -----------------------------

  /// Maps a source file -- one exporting `guatiao_source`, such as a
  /// bridge to another language -- and adds the source it answers for
  /// `config` (a map, or a [Value] holding one: the source's own settings).
  /// Answers the source's name, its path.
  ///
  /// The file stays mapped for the life of the process and is not a
  /// library: nothing here retires it. Nothing it serves is loaded until
  /// [refresh]. A refusal throws with the source's words: [NotFound] when
  /// the file cannot be mapped, [WrongKind] when it is no source,
  /// [BadValue] for a table the checks refuse or a path already added, and
  /// the source's own status when it refuses `config`.
  String addSourceFile(String path, [Object? config]) {
    _checkOpen();
    final lib = native.core();
    lib.require('guatiao_registry_add_source_file', feature: 'load');
    final bindings = lib.bindings;
    final Value? owned;
    final Value configValue;
    if (config is Value) {
      owned = null;
      configValue = config;
    } else {
      owned = Value.fromDart(
        config ?? const <String, Object?>{},
        alloc: _allocator,
      );
      configValue = owned;
    }
    final err = calloc<guatiao_provider_error>();
    try {
      final status = using(
        (arena) => bindings.guatiao_registry_add_source_file(
          _handle,
          strOf(arena, path).ref,
          configValue.pointer,
          err,
        ),
      );
      checkProviderStatus(bindings, status, err);
      return path;
    } finally {
      owned?.close();
      calloc.free(err);
    }
  }

  /// Watches a directory: each [refresh] loads a library file that
  /// appeared there (filtered by `rules`, one `[!]KEY=VALUE` per line, as
  /// [scanDir] takes them), retires one that vanished, and retires and
  /// loads again one whose length or modification time changed.
  /// [BadValue] for a rule that is not one or a directory already watched.
  void watchDir(String directory, {String? rules}) {
    _checkOpen();
    final lib = native.core();
    lib.require('guatiao_registry_watch_dir', feature: 'load');
    using((arena) {
      checkStatus(
        lib.bindings.guatiao_registry_watch_dir(
          _handle,
          strOf(arena, directory).ref,
          strOf(arena, rules ?? '').ref,
        ),
      );
    });
  }

  /// Asks every source and watched directory what it serves now and makes
  /// this registry agree. Answers what changed: `loaded` and `retired`
  /// (each `{source, id, stamp, key}`), `skipped` (`{source, id,
  /// skipped, ...}`) and `refused` (`{source, id?, why}`).
  ///
  /// A new candidate loads; a changed one loads and, once it has, its old
  /// build is retired while an [Instance] of the old keeps working; a
  /// vanished one is retired; a library retired by hand stays out until
  /// its stamp changes. A retired library closes once nothing uses it, and
  /// its source is told. Refusals are part of the answer.
  Map<String, Object?> refresh() {
    native.core().require('guatiao_registry_refresh', feature: 'load');
    return _answerMap(
      (reg, alloc, out) =>
          native.core().bindings.guatiao_registry_refresh(reg, alloc, out),
    );
  }

  /// Unmaps a library, when the library agrees.
  ///
  /// [Busy] while a handle into it is held -- an answer another library
  /// pinned through the host's services -- or, from its own `unload`
  /// function, asked next and its promise that what it can account for is
  /// released, while an instance or object it handed out is alive.
  /// [Unsupported] when it has no such function, [WrongKind] when it is
  /// linked into the host rather than mapped, [NotFound] for an unknown
  /// key. A refusal leaves it loaded.
  ///
  /// `unchecked: true` is the caller insisting on a library with no
  /// `unload` function. Either way, no table, `ctx` or borrowed schema
  /// taken from it is used again.
  void unload(String key, {bool unchecked = false}) => _keyCall(
    unchecked ? 'guatiao_registry_unload_unchecked' : 'guatiao_registry_unload',
    key,
  );
}
