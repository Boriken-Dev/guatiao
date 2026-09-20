// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
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
class _BorrowedRoot implements ValueOwner {
  _BorrowedRoot(this._root);

  final Pointer<guatiao_value> _root;

  @override
  Pointer<guatiao_value> get rootNode => _root;

  @override
  Pointer<guatiao_alloc> get allocator =>
      throw StateError('a borrowed value cannot be mutated');
}

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
  factory Registry(
    String id,
    String version, {
    Pointer<guatiao_alloc>? alloc,
  }) {
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
  /// Every library it holds stays mapped; [unload] is what unmaps one.
  void close() {
    if (_closed) return;
    _closed = true;
    native.core().bindings.guatiao_registry_free(_handle);
  }

  /// One `(reg, ..., alloc, out) -> status` answer, as plain Dart, freed.
  Object? _answer(
    int Function(Pointer<guatiao_registry>, Pointer<guatiao_alloc>,
            Pointer<guatiao_value>)
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
    int Function(Pointer<guatiao_registry>, Pointer<guatiao_alloc>,
            Pointer<guatiao_value>)
        call,
  ) =>
      (_answer(call)! as Map).cast<String, Object?>();

  List<Map<String, Object?>> _answerRows(
    int Function(Pointer<guatiao_registry>, Pointer<guatiao_alloc>,
            Pointer<guatiao_value>)
        call,
  ) =>
      <Map<String, Object?>>[
        for (final row in _answer(call)! as List)
          (row! as Map).cast<String, Object?>(),
      ];

  // ---- loading ----------------------------------------------------

  /// Loads one library by path. The report says what was loaded or
  /// skipped.
  Map<String, Object?> loadFile(String path) => using(
        (arena) {
          final view = strOf(arena, path).ref;
          return _answerMap(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_load_file(reg, view, alloc, out),
          );
        },
      );

  /// Loads every library in `directory`.
  ///
  /// `rules` is the newline-separated scan filter -- `kind=greeter` and
  /// friends -- matched against what each library declares about itself.
  Map<String, Object?> scanDir(
    String directory, {
    bool descending = false,
    String? rules,
  }) =>
      using(
        (arena) {
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
        },
      );

  /// Loads every library along a search path.
  Map<String, Object?> scanPath(
    String spec, {
    bool descending = false,
    String? rules,
  }) =>
      using(
        (arena) {
          final path = strOf(arena, spec).ref;
          final filter = strOf(arena, rules ?? '').ref;
          return _answerMap(
            (reg, alloc, out) =>
                native.core().bindings.guatiao_registry_scan_path(
                      reg,
                      path,
                      descending,
                      filter,
                      alloc,
                      out,
                    ),
          );
        },
      );

  // ---- what is loaded ----------------------------------------------

  /// Every loaded library.
  List<Map<String, Object?>> libraries() => _answerRows(
        (reg, alloc, out) =>
            native.core().bindings.guatiao_registry_libraries(reg, alloc, out),
      );

  /// Every provider that CLAIMS `kind`, best first. Every provider when
  /// `kind` is empty.
  List<Map<String, Object?>> providers([String kind = '']) => using(
        (arena) {
          final view = strOf(arena, kind).ref;
          return _answerRows(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_providers(reg, view, alloc, out),
          );
        },
      );

  /// Every provider that can actually serve `kind` right now.
  List<Map<String, Object?>> available([String kind = '']) => using(
        (arena) {
          final view = strOf(arena, kind).ref;
          return _answerRows(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_available(reg, view, alloc, out),
          );
        },
      );

  /// Why nothing can serve `kind`: `available`, and `why` with the
  /// providers that were passed over when there were any.
  Map<String, Object?> whyNot(String kind) => using(
        (arena) {
          final view = strOf(arena, kind).ref;
          return _answerMap(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_why_not(reg, view, alloc, out),
          );
        },
      );

  /// The provider filed under `key`.
  Map<String, Object?> provider(String key) => using(
        (arena) {
          final view = strOf(arena, key).ref;
          return _answerMap(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_provider(reg, view, alloc, out),
          );
        },
      );

  /// The best provider that can serve `kind`.
  Map<String, Object?> best(String kind) => using(
        (arena) {
          final view = strOf(arena, kind).ref;
          return _answerMap(
            (reg, alloc, out) => native
                .core()
                .bindings
                .guatiao_registry_best(reg, view, alloc, out),
          );
        },
      );

  // ---- keys and priority ---------------------------------------------

  /// Sets the template providers are filed under (`%id`, `%name`,
  /// `%version`, `%library`), and refiles what is loaded.
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

  /// Sets the template libraries are filed under (`%id`, `%version`).
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
    return Ref(_BorrowedRoot(root));
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
        'guatiao_registry_retire' =>
          bindings.guatiao_registry_retire(_handle, view),
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
  /// free to load again. A table or an [Instance] already taken from it
  /// keeps working; a [providerConfig] of one of its providers does not,
  /// because that copy belonged to the registry.
  void retire(String key) => _keyCall('guatiao_registry_retire', key);

  /// Unmaps a library, when the library agrees.
  ///
  /// Its own `unload` function is asked first, and is its promise that
  /// what it can account for is released. [Busy] while an instance or
  /// object it handed out is alive, [Unsupported] when it has no such
  /// function, [WrongKind] when it is linked into the host rather than
  /// mapped, [NotFound] for an unknown key. A refusal leaves it loaded.
  ///
  /// `unchecked: true` is the caller insisting on a library with no
  /// `unload` function. Either way, no table, `ctx` or borrowed schema
  /// taken from it is used again.
  void unload(String key, {bool unchecked = false}) => _keyCall(
        unchecked
            ? 'guatiao_registry_unload_unchecked'
            : 'guatiao_registry_unload',
        key,
      );
}
