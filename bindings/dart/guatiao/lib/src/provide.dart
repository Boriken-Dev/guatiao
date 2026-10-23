// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A library written in Dart: provider classes offered by a [Library],
/// whose descriptor any host in the process reads.
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'kind_runtime.dart';
import 'value.dart';

/// The base of a provider class: a class serving kinds, which it
/// `implements`. [dispose] runs when its host destroys an instance it
/// built, or when the [Library] holding its one instance closes.
abstract class Provider implements Disposable {
  @override
  void dispose() {}
}

typedef _EntryC =
    Pointer<guatiao_library_info> Function(Pointer<guatiao_host_info>);
typedef _UnloadC = Uint32 Function();
typedef _AvailableC = Bool Function(Pointer<Void>, Pointer<guatiao_str>);
typedef _CreateC =
    Uint32 Function(
      Pointer<Void>,
      Pointer<guatiao_value>,
      Pointer<Pointer<Void>>,
      Pointer<guatiao_provider_error>,
    );
typedef _DestroyInstanceC = Void Function(Pointer<Void>, Pointer<Void>);

/// How a [Library] offers one provider class.
final class Offer {
  /// [instance], built now, is the provider: a host calls it through its
  /// tables for [kinds].
  Offer.single(
    Provider instance, {
    required this.named,
    required this.kinds,
    this.id,
    this.displayName = '',
    this.version = '',
    this.meta,
    this.available,
  }) : _instance = instance,
       _create = null,
       _tableForClass = null,
       schema = null;

  Offer._configured(
    Provider Function(Map<String, Object?> config) create,
    Pointer<Void> Function(Kind kind, String what) tableForClass, {
    required this.named,
    required this.kinds,
    required Map<String, Object?> this.schema,
    this.id,
    this.displayName = '',
    this.version = '',
    this.meta,
    this.available,
  }) : _instance = null,
       _create = create,
       _tableForClass = tableForClass;

  /// [create] builds an instance of `T` per configuration the host
  /// checked against [schema] (a JSON Schema), receiving it as plain Dart;
  /// the host destroys each, and its [Provider.dispose] runs. Each table
  /// is `T`'s: a slot every `T` implements.
  ///
  /// A method rather than a constructor so that `T` is what [create]
  /// builds, whatever list the offer is written in.
  static Offer configured<T extends Provider>(
    T Function(Map<String, Object?> config) create, {
    required String named,
    required List<Kind> kinds,
    required Map<String, Object?> schema,
    String? id,
    String displayName = '',
    String version = '',
    Map<String, Object?>? meta,
    String? Function()? available,
  }) => Offer._configured(
    create,
    (kind, what) => kind.tableForType<T>(what: what),
    named: named,
    kinds: kinds,
    schema: schema,
    id: id,
    displayName: displayName,
    version: version,
    meta: meta,
    available: available,
  );

  /// The provider's name in its library: its id is `{library id}_{named}`
  /// unless [id] says otherwise.
  final String named;

  /// The provider kinds it serves, each a kind's `kind` (`Greeter.kind`).
  final List<Kind> kinds;
  final String? id;
  final String displayName;

  /// Empty: the library's.
  final String version;
  final Map<String, Object?>? meta;

  /// `null` when the provider can run here, else why not; asked every
  /// time a host asks.
  final String? Function()? available;

  /// The configuration's schema, for an instance per configuration.
  final Map<String, Object?>? schema;

  final Provider? _instance;
  final Provider Function(Map<String, Object?>)? _create;
  final Pointer<Void> Function(Kind kind, String what)? _tableForClass;

  /// The table for [kind]: by the instance's methods, or by the class's.
  Pointer<Void> _tableFor(Kind kind) {
    final instance = _instance;
    return instance != null
        ? kind.tableFor(instance, what: named)
        : _tableForClass!(kind, named);
  }
}

/// A library's id and version, and what it says about itself: what
/// `dart run guatiao:kind identity` writes from the package's
/// `pubspec.yaml`.
final class LibraryIdentity {
  const LibraryIdentity(this.id, this.version, {this.meta});

  final String id;
  final String version;
  final Map<String, Object?>? meta;
}

/// Every library built here stays reachable for the process: a host may
/// hold its descriptor's function pointers for as long as it runs.
final Set<Library> _kept = <Library>{};

/// Reasons a provider gives for being unavailable, kept for every reader.
final Map<String, Pointer<Uint8>> _reasons = <String, Pointer<Uint8>>{};

/// A library of providers written in Dart: its descriptor, built now and
/// kept for the process, which `Registry.registerLibrary` files.
///
/// Its callbacks run on the thread of the isolate that built it: a host
/// calling one from another thread ends the process, as Dart does for any
/// isolate-local callback.
final class Library {
  /// A library [id] at [version] offering [offers]; [meta] is what it says
  /// about itself (a map).
  Library(
    this.id,
    this.version, {
    required List<Offer> offers,
    Map<String, Object?>? meta,
  }) : offers = List.unmodifiable(offers),
       meta = meta == null ? null : Map.unmodifiable(meta) {
    _build();
  }

  /// A library named and versioned by [identity], its meta merged under
  /// [meta].
  Library.of(
    LibraryIdentity identity, {
    required List<Offer> offers,
    Map<String, Object?>? meta,
  }) : this(
         identity.id,
         identity.version,
         offers: offers,
         meta: identity.meta == null && meta == null
             ? null
             : {...?identity.meta, ...?meta},
       );

  final String id;
  final String version;
  final List<Offer> offers;
  final Map<String, Object?>? meta;

  final Set<int> _instances = <int>{};
  final List<int> _defaults = <int>[];
  final List<Value> _values = <Value>[];
  final List<NativeCallable<Function>> _callables =
      <NativeCallable<Function>>[];
  late final Pointer<guatiao_library_info> _info;
  late final NativeCallable<_EntryC> _entry;
  bool _closed = false;

  /// The `guatiao_library_entry` a host registers.
  Pointer<
    NativeFunction<
      Pointer<guatiao_library_info> Function(Pointer<guatiao_host_info>)
    >
  >
  get entry => _entry.nativeFunction;

  /// The descriptor the entry answers.
  Pointer<guatiao_library_info> get info => _info;

  /// How many instances a host built and has not destroyed.
  int liveInstances() => _instances.length;

  /// The kinds its providers serve, each once, in order.
  List<String> get kinds => [
    ...{
      for (final offer in offers)
        for (final kind in offer.kinds) kind.name,
    },
  ];

  /// [callable]'s function, kept for the process, which it does not keep
  /// running.
  Pointer<NativeFunction<N>> _callable<N extends Function>(
    NativeCallable<N> callable,
  ) {
    callable.keepIsolateAlive = false;
    _callables.add(callable);
    return callable.nativeFunction;
  }

  void _build() {
    if (id.isEmpty) throw ArgumentError('a library has an id');
    final ids = [for (final o in offers) o.id ?? '${id}_${o.named}'];
    if (ids.toSet().length != ids.length) {
      throw ArgumentError(
        'two providers of $id share an id: ${ids.join(', ')}',
      );
    }
    for (final offer in offers) {
      if (offer.kinds.isEmpty) {
        throw ArgumentError('${offer.named} serves no kind');
      }
      for (final kind in offer.kinds) {
        if (kind.isObject) {
          throw ArgumentError(
            '${offer.named}: ${kind.name} is an object kind; a provider '
            'serves provider kinds',
          );
        }
      }
    }
    final stride = sizeOf<guatiao_provider_info>();
    final infos = calloc<Uint8>(stride * (offers.isEmpty ? 1 : offers.length));
    for (var i = 0; i < offers.length; i++) {
      _describe((infos + i * stride).cast(), offers[i], ids[i]);
    }
    final info = calloc<guatiao_library_info>();
    info.ref
      ..struct_size = sizeOf<guatiao_library_info>()
      ..abi_version = GUATIAO_ABI_VERSION;
    _fill(info.ref.id, id);
    _fill(info.ref.version, version);
    info.ref.providers
      ..ptr = infos.cast()
      ..len = offers.length
      ..stride = stride;
    info.ref.meta = _map(meta);
    info.ref.unload = _callable(
      NativeCallable<_UnloadC>.isolateLocal(
        () => _instances.isEmpty ? Status.ok.code : Status.busy.code,
        exceptionalReturn: 100,
      ),
    );
    _info = info;
    _entry = NativeCallable<_EntryC>.isolateLocal(
      (Pointer<guatiao_host_info> _) => _closed ? nullptr : info,
    )..keepIsolateAlive = false;
    _kept.add(this);
  }

  Pointer<guatiao_map> _map(Map<String, Object?>? data) {
    if (data == null) return nullptr;
    final value = Value.fromDart(data);
    _values.add(value);
    return (value.pointer.cast<Uint8>() +
            (sizeOf<guatiao_value>() - sizeOf<guatiao_payload>()))
        .cast();
  }

  void _describe(
    Pointer<guatiao_provider_info> at,
    Offer offer,
    String providerId,
  ) {
    final kinds = offer.kinds;
    final names = calloc<guatiao_str>(kinds.isEmpty ? 1 : kinds.length);
    for (var i = 0; i < kinds.length; i++) {
      _fill(names[i], kinds[i].name);
    }
    final tableStride = sizeOf<guatiao_kind_table>();
    final tables = calloc<Uint8>(
      tableStride * (kinds.isEmpty ? 1 : kinds.length),
    );
    for (var i = 0; i < kinds.length; i++) {
      final kind = kinds[i];
      final table = (tables + i * tableStride).cast<guatiao_kind_table>().ref
        ..struct_size = tableStride
        ..vtable_size = kind.tableSize
        ..vtable = offer._tableFor(kind);
      _fill(table.kind, kind.name);
    }
    final p = at.ref
      ..struct_size = sizeOf<guatiao_provider_info>()
      ..meta = _map(offer.meta);
    _fill(p.id, providerId);
    _fill(p.display_name, offer.displayName);
    _fill(p.version, offer.version);
    p.kinds
      ..ptr = names
      ..len = kinds.length;
    p.tables
      ..ptr = tables.cast()
      ..len = kinds.length
      ..stride = tableStride;
    final ask = offer.available;
    if (ask != null) {
      p.available = _callable(
        NativeCallable<_AvailableC>.isolateLocal(
          (Pointer<Void> _, Pointer<guatiao_str> reason) =>
              _available(ask, reason),
          exceptionalReturn: false,
        ),
      );
    }
    final instance = offer._instance;
    if (instance != null) {
      final ctx = hold(instance);
      _defaults.add(ctx);
      p.ctx = Pointer.fromAddress(ctx);
      return;
    }
    final schema = Value.fromDart(offer.schema);
    _values.add(schema);
    p.config = schema.pointer;
    final create = offer._create!;
    p.create = _callable(
      NativeCallable<_CreateC>.isolateLocal(
        (
          Pointer<Void> _,
          Pointer<guatiao_value> config,
          Pointer<Pointer<Void>> out,
          Pointer<guatiao_provider_error> err,
        ) => _createInstance(offer, create, config, out, err),
        exceptionalReturn: 100,
      ),
    );
    p.destroy = _callable(
      NativeCallable<_DestroyInstanceC>.isolateLocal((
        Pointer<Void> _,
        Pointer<Void> instance,
      ) {
        _instances.remove(instance.address);
        try {
          release(instance.address, dispose: true);
        } on Object catch (e) {
          stderr.writeln(
            'guatiao: $providerId: disposing an instance threw $e',
          );
        }
      }),
    );
  }

  int _createInstance(
    Offer offer,
    Provider Function(Map<String, Object?>) create,
    Pointer<guatiao_value> config,
    Pointer<Pointer<Void>> out,
    Pointer<guatiao_provider_error> err,
  ) {
    try {
      // Refused before anything is built: an instance with no slot to land
      // in would be held for the process.
      if (out == nullptr) {
        throw NullArgument(
          Status.nullArgument.code,
          'no slot for the instance',
        );
      }
      out.value = nullptr;
      if (config == nullptr) {
        throw NullArgument(Status.nullArgument.code, 'no configuration');
      }
      final data = Ref.borrowed(config).toDart();
      final made = create(
        data is Map ? data.cast<String, Object?>() : <String, Object?>{},
      );
      final handle = hold(made);
      _instances.add(handle);
      out.value = Pointer.fromAddress(handle);
      return Status.ok.code;
    } on Object catch (e) {
      final (status, message) = statusOf(e);
      if (err != nullptr) writeProviderError(err, status, message);
      return status;
    }
  }

  bool _available(String? Function() ask, Pointer<guatiao_str> reason) {
    String? why;
    try {
      why = ask();
    } on Object catch (e) {
      why = statusOf(e).$2;
    }
    if (why == null) return true;
    final bytes = utf8.encode(why);
    final kept = _reasons.putIfAbsent(why, () {
      final ptr = calloc<Uint8>(bytes.isEmpty ? 1 : bytes.length);
      ptr.asTypedList(bytes.length).setAll(0, bytes);
      return ptr;
    });
    if (reason != nullptr) {
      reason.ref
        ..ptr = kept
        ..len = bytes.length;
    }
    return false;
  }

  /// Lets go of each provider's own instance and answers null to any host
  /// asking for the descriptor from now on. Call it once no host will call
  /// this library again; the descriptor stays readable.
  void close() {
    if (_closed) return;
    _closed = true;
    for (final ctx in _defaults) {
      release(ctx, dispose: true);
    }
    _defaults.clear();
  }

  @override
  String toString() => 'Library $id $version';
}

/// Points [at] at a copy of [text], kept for the process.
void _fill(guatiao_str at, String text) {
  final bytes = utf8.encode(text);
  if (bytes.isNotEmpty) {
    final ptr = calloc<Uint8>(bytes.length);
    ptr.asTypedList(bytes.length).setAll(0, bytes);
    at.ptr = ptr;
  }
  at.len = bytes.length;
}
