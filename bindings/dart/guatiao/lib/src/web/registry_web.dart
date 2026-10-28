// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The registry's names on the web, where it does not exist: a browser
/// maps no libraries, so opening one throws `Unsupported` saying so.
library;

import '../import_search.dart';
import 'library_web.dart';
import 'module.dart';
import 'value_web.dart';

/// What a provider speaks a kind through. Never answered on the web.
class ProviderTable {
  const ProviderTable(this.table, this.size, this.ctx);

  final Address table;
  final int size;
  final Address ctx;

  bool get isEmpty => table.isNull;
}

/// An instance a registry built. Never answered on the web.
class Instance {
  Instance._();

  final Address ctx = Address.none;

  bool get isClosed => true;

  void close() {}
}

Never _no() => unsupportedOnTheWeb('the registry');

/// A host's table of loaded libraries. On the web its constructor throws
/// `Unsupported`: a browser maps no libraries.
class Registry {
  factory Registry(String id, String version, {Address? alloc}) => _no();

  bool get isClosed => true;

  void close() {}

  Map<String, Object?> loadFile(String path) => _no();
  Map<String, Object?> scanDir(
    String directory, {
    bool descending = false,
    String? rules,
  }) => _no();
  Map<String, Object?> scanPath(
    String spec, {
    bool descending = false,
    String? rules,
  }) => _no();
  List<Map<String, Object?>> libraries() => _no();
  List<Map<String, Object?>> providers([String kind = '']) => _no();
  List<Map<String, Object?>> available([String kind = '']) => _no();
  Map<String, Object?> whyNot(String kind) => _no();
  Map<String, Object?> provider(String key) => _no();
  Map<String, Object?> best(String kind) => _no();
  Registry keyedBy(String template) => _no();
  Registry librariesKeyedBy(String template) => _no();
  void setPriority(String id, int priority) => _no();
  int priority(String id) => _no();
  ImportSearch get importSearch => _no();
  set importSearch(ImportSearch search) => _no();
  ProviderTable providerTable(String key, {String? kind}) => _no();
  Ref? providerConfig(String key) => _no();
  Value? providerMeta(String key) => _no();
  Instance create(String key, Object? config) => _no();
  void retire(String key) => _no();
  List<String> collect() => _no();
  String addSourceFile(String path, [Object? config]) => _no();
  void watchDir(String directory, {String? rules}) => _no();
  Map<String, Object?> refresh() => _no();
  void unload(String key, {bool unchecked = false}) => _no();
}

/// The table is too small, or its `floor_hash` does not match.
class FloorMismatch implements Exception {
  FloorMismatch(this.reason);

  final String reason;

  @override
  String toString() => 'FloorMismatch: $reason';
}

/// A kind's table as its C header declares it. On the web there is no
/// table to take: throws `Unsupported`.
Never kindTable<T>(
  ProviderTable found, {
  required int floorHash,
  required int floorSize,
}) => unsupportedOnTheWeb('a kind table');
