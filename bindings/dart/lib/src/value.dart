// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;

/// `guatiao_tag`: which arm of a value's payload is live.
enum Tag {
  absent(0),
  nullValue(1),
  boolean(2),
  number(3),
  string(4),
  bytes(5),
  list(6),
  map(7);

  const Tag(this.code);

  final int code;

  /// The tag `code` names, or `null` for a tag this binding does not know.
  static Tag? fromCode(int code) {
    for (final tag in Tag.values) {
      if (tag.code == code) return tag;
    }
    return null;
  }
}

/// How a number's exact text becomes a Dart value.
enum Numbers {
  /// An `int` when the text has no `.`, `e` or `E` and fits 64 bits, a
  /// `BigInt` when it does not fit, a `double` otherwise.
  auto,

  /// The exact text, always.
  text,

  /// A `BigInt` when the text is integral, a `double` when it is not.
  bigInt,
}

/// The answer to a lookup that found nothing.
///
/// Distinct from `null`, which is the stored null.
class Absent {
  const Absent._();

  @override
  String toString() => 'guatiao.absent';
}

/// The one [Absent].
const Absent absent = Absent._();

class _Unset {
  const _Unset();
}

const _Unset _unset = _Unset();

/// What a [Ref] walks from: a root node and the allocator to build into.
abstract class ValueOwner {
  /// The root `guatiao_value`. Throws when it is no longer usable.
  Pointer<guatiao_value> get rootNode;

  /// The allocator new content is built through. Throws for a root this
  /// binding only borrows.
  Pointer<guatiao_alloc> get allocator;
}

/// A root this binding reads but does not own. See [Ref.borrowed].
class _BorrowedRoot implements ValueOwner {
  _BorrowedRoot(this._root);

  final Pointer<guatiao_value> _root;

  @override
  Pointer<guatiao_value> get rootNode => _root;

  @override
  Pointer<guatiao_alloc> get allocator =>
      throw StateError('a borrowed value cannot be mutated');
}

// ---- reading, with no library needed ---------------------------------
//
// Every reader below reimplements one of `guatiao.h`'s `static inline`
// helpers, which have no symbol in any library.

// **No top-level state on this path**, so a reader is safe inside an
// `isolateGroupBound` callback, which may touch only globals shared across
// the isolate group. A stride comes from a struct pointer's own `+` and an
// offset from `sizeOf` written inline, both resolved at compile time.

/// `guatiao_entry.value`: the entry's key is a `guatiao_string`, and the
/// value follows it. Pinned by the crate's own layout checks.
Pointer<guatiao_value> _entryValue(Pointer<guatiao_entry> entry) =>
    (entry.cast<Uint8>() + sizeOf<guatiao_string>()).cast<guatiao_value>();

Uint8List _stringBytes(guatiao_string s) {
  if (s.len == 0 || s.ptr == nullptr) return Uint8List(0);
  return Uint8List.fromList(s.ptr.asTypedList(s.len));
}

Uint8List _bufferBytes(guatiao_buffer b) {
  if (b.len == 0 || b.ptr == nullptr) return Uint8List(0);
  return Uint8List.fromList(b.ptr.asTypedList(b.len));
}

bool _bytesEqual(Uint8List a, Uint8List b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

int _tagOf(Pointer<guatiao_value> node) =>
    node == nullptr ? Tag.absent.code : node.ref.tag;

Uint8List _numberText(Pointer<guatiao_value> node) {
  if (node == nullptr || node.ref.tag != Tag.number.code) return Uint8List(0);
  return _stringBytes(node.ref.payload.number);
}

Pointer<guatiao_value> _listAt(Pointer<guatiao_value> node, int index) {
  if (node == nullptr || node.ref.tag != Tag.list.code) return nullptr;
  final list = node.ref.payload.list;
  if (list.ptr == nullptr) return nullptr;
  var i = index;
  if (i < 0) i += list.len;
  if (i < 0 || i >= list.len) return nullptr;
  return list.ptr + i;
}

Pointer<guatiao_entry> _entryAt(guatiao_map map, int index) => map.ptr + index;

/// The value stored under `key`, compared over bytes: a key is UTF-8 with
/// no terminator and may hold a NUL of its own.
Pointer<guatiao_value> _mapFind(Pointer<guatiao_value> node, Uint8List key) {
  if (node == nullptr || node.ref.tag != Tag.map.code) return nullptr;
  final map = node.ref.payload.map;
  if (map.ptr == nullptr) return nullptr;
  for (var i = 0; i < map.len; i++) {
    final entry = _entryAt(map, i);
    if (_bytesEqual(_stringBytes(entry.ref.key), key)) {
      return _entryValue(entry);
    }
  }
  return nullptr;
}

Pointer<guatiao_value> _walk(Pointer<guatiao_value> root, List<Object> path) {
  var node = root;
  for (final part in path) {
    node = part is int
        ? _listAt(node, part)
        : _mapFind(node, _utf8Bytes(part as String));
    if (node == nullptr) return nullptr;
  }
  return node;
}

Uint8List _utf8Bytes(String text) => Uint8List.fromList(utf8.encode(text));

Object _numberToDart(Uint8List text, Numbers numbers) {
  final s = String.fromCharCodes(text);
  switch (numbers) {
    case Numbers.text:
      return s;
    case Numbers.bigInt:
      return BigInt.tryParse(s) ?? double.parse(s);
    case Numbers.auto:
      if (s.contains('.') || s.contains('e') || s.contains('E')) {
        return double.parse(s);
      }
      return int.tryParse(s) ?? BigInt.parse(s);
  }
}

Object? _nodeToDart(Pointer<guatiao_value> node, Numbers numbers) {
  if (node == nullptr) return absent;
  switch (Tag.fromCode(node.ref.tag)) {
    case Tag.absent:
    case null: // a tag this binding does not know: skip, as the header asks
      return absent;
    case Tag.nullValue:
      return null;
    case Tag.boolean:
      return node.ref.payload.b;
    case Tag.number:
      return _numberToDart(_stringBytes(node.ref.payload.number), numbers);
    case Tag.string:
      return utf8.decode(_stringBytes(node.ref.payload.text));
    case Tag.bytes:
      return _bufferBytes(node.ref.payload.bytes);
    case Tag.list:
      final list = node.ref.payload.list;
      return <Object?>[
        for (var i = 0; i < list.len; i++)
          _nodeToDart(_listAt(node, i), numbers),
      ];
    case Tag.map:
      final map = node.ref.payload.map;
      final out = <String, Object?>{};
      for (var i = 0; i < map.len; i++) {
        final entry = _entryAt(map, i);
        final key = utf8.decode(_stringBytes(entry.ref.key));
        out[key] = _nodeToDart(_entryValue(entry), numbers);
      }
      return out;
  }
}

/// Reads `node` as plain Dart, then frees its tree and the box it sits in.
///
/// For an answer a native call filled into a `calloc`-allocated root that
/// nothing else will ever address.
Object? takeValue(
  Pointer<guatiao_value> node, {
  Numbers numbers = Numbers.auto,
}) {
  try {
    return _nodeToDart(node, numbers);
  } finally {
    native.core().bindings.guatiao_value_free(node);
    calloc.free(node);
  }
}

// ---- writing, through the loaded library ------------------------------

/// A `guatiao_str` over a copy of `data`, valid until `arena` is released.
Pointer<guatiao_str> strView(Arena arena, List<int> data) {
  final out = arena<guatiao_str>();
  if (data.isEmpty) {
    out.ref
      ..ptr = nullptr
      ..len = 0;
    return out;
  }
  final buffer = arena<Uint8>(data.length);
  buffer.asTypedList(data.length).setAll(0, data);
  out.ref
    ..ptr = buffer
    ..len = data.length;
  return out;
}

/// A `guatiao_str` over a copy of `text`'s UTF-8 bytes.
Pointer<guatiao_str> strOf(Arena arena, String text) =>
    strView(arena, utf8.encode(text));

Pointer<guatiao_bytes> _bytesView(Arena arena, List<int> data) {
  final out = arena<guatiao_bytes>();
  if (data.isEmpty) {
    out.ref
      ..ptr = nullptr
      ..len = 0;
    return out;
  }
  final buffer = arena<Uint8>(data.length);
  buffer.asTypedList(data.length).setAll(0, data);
  out.ref
    ..ptr = buffer
    ..len = data.length;
  return out;
}

String _textForNumber(Object value) {
  if (value is double) {
    if (!value.isFinite) {
      throw ArgumentError.value(value, 'value', 'is not a finite number');
    }
    return value.toString();
  }
  return value.toString();
}

/// Writes `obj` into `raw`, which must be absent- or null-tagged.
void _writeDart(
  Pointer<guatiao_value> raw,
  Object? obj,
  Pointer<guatiao_alloc> alloc,
) {
  final bindings = native.core().bindings;
  if (obj == null) {
    checkStatus(bindings.guatiao_value_null(raw));
    return;
  }
  if (obj is Absent) {
    checkStatus(bindings.guatiao_value_absent(raw));
    return;
  }
  if (obj is Ref) {
    checkStatus(bindings.guatiao_value_clone(alloc, obj.requireNode(), raw));
    return;
  }
  if (obj is bool) {
    checkStatus(bindings.guatiao_value_bool(obj, raw));
    return;
  }
  if (obj is int || obj is double || obj is BigInt) {
    final text = _textForNumber(obj);
    using((arena) {
      checkStatus(
        bindings.guatiao_value_number(alloc, strOf(arena, text).ref, raw),
      );
    });
    return;
  }
  if (obj is String) {
    using((arena) {
      checkStatus(
        bindings.guatiao_value_string(alloc, strOf(arena, obj).ref, raw),
      );
    });
    return;
  }
  if (obj is TypedData) {
    final data = obj is Uint8List
        ? obj
        : Uint8List.view(
            obj.buffer,
            obj.offsetInBytes,
            obj.lengthInBytes,
          );
    using((arena) {
      checkStatus(
        bindings.guatiao_value_bytes(alloc, _bytesView(arena, data).ref, raw),
      );
    });
    return;
  }
  if (obj is List) {
    checkStatus(bindings.guatiao_value_list(alloc, raw));
    for (final item in obj) {
      _buildInto(
        alloc,
        item,
        (tmp) => checkStatus(bindings.guatiao_list_push(alloc, raw, tmp)),
      );
    }
    return;
  }
  if (obj is Map) {
    checkStatus(bindings.guatiao_value_map(alloc, raw));
    for (final entry in obj.entries) {
      final key = entry.key;
      if (key is! String) {
        throw ArgumentError.value(key, 'key', 'a map key must be a String');
      }
      _buildInto(
        alloc,
        entry.value,
        (tmp) => using(
          (arena) => checkStatus(
            bindings.guatiao_map_set(alloc, raw, strOf(arena, key).ref, tmp),
          ),
        ),
      );
    }
    return;
  }
  throw ArgumentError.value(
    obj,
    'value',
    'cannot be converted to a guatiao value',
  );
}

/// Builds `obj` into a scratch node and hands it to `consume`, which
/// moves it somewhere that owns it.
///
/// `guatiao_map_set` and `guatiao_list_push` leave a moved node
/// null-tagged, so the unconditional free afterwards releases only what a
/// failure left behind.
void _buildInto(
  Pointer<guatiao_alloc> alloc,
  Object? obj,
  void Function(Pointer<guatiao_value>) consume,
) {
  final bindings = native.core().bindings;
  final tmp = calloc<guatiao_value>();
  try {
    _writeDart(tmp, obj, alloc);
    consume(tmp);
  } finally {
    bindings.guatiao_value_free(tmp);
    calloc.free(tmp);
  }
}

// ---- the tree ---------------------------------------------------------

/// A node inside a [Value]'s tree: the owning value plus a path of keys
/// and indices.
///
/// Every access walks the path again. Nothing here caches an address,
/// because a sibling write can reallocate the array a cached one points
/// into.
class Ref {
  Ref(ValueOwner owner, [List<Object> path = const []])
      : this._(owner, List<Object>.unmodifiable(path));

  Ref._(this._ownerOrNull, this.path);

  /// A read-only view of a tree this binding does not own, rooted at
  /// [root]: a value an engine hands a callback, or one a registry lends.
  ///
  /// The caller keeps that memory alive and unchanged for as long as the
  /// view is read. Writing through it throws, and nothing frees it.
  /// Reading one touches no library and no top-level state, so it is safe
  /// inside an `isolateGroupBound` callback.
  factory Ref.borrowed(Pointer<guatiao_value> root) => Ref(_BorrowedRoot(root));

  final ValueOwner? _ownerOrNull;

  /// The keys and indices from the owning value's root to this node.
  final List<Object> path;

  ValueOwner get owner => _ownerOrNull ?? this as ValueOwner;

  /// This node, or `nullptr` when the path no longer leads anywhere.
  Pointer<guatiao_value> node() => _walk(owner.rootNode, path);

  /// This node, or a [StateError] naming the path that found nothing.
  Pointer<guatiao_value> requireNode() {
    final found = node();
    if (found == nullptr) {
      throw StateError('no value at $path');
    }
    return found;
  }

  /// Which arm of this node's payload is live.
  Tag get tag => Tag.fromCode(_tagOf(node())) ?? Tag.absent;

  bool get isAbsent => tag == Tag.absent;

  bool get isNull => tag == Tag.nullValue;

  /// This node as a `bool`, or `fallback` when it is not one.
  bool boolOr(bool fallback) {
    final found = node();
    if (found == nullptr || found.ref.tag != Tag.boolean.code) return fallback;
    return found.ref.payload.b;
  }

  /// This node as an `int`, or `fallback`.
  ///
  /// A wrong kind, a fractional or exponent spelling, and a value outside
  /// a signed 64-bit range all fall back; nothing is ever truncated.
  int intOr(int fallback) {
    final text = _numberText(node());
    if (text.isEmpty || text.length >= 48) return fallback;
    final s = String.fromCharCodes(text);
    if (s.contains('.') || s.contains('e') || s.contains('E')) return fallback;
    return int.tryParse(s) ?? fallback;
  }

  /// This node as a `double`, or `fallback`.
  double doubleOr(double fallback) {
    final text = _numberText(node());
    if (text.isEmpty || text.length >= 344) return fallback;
    return double.tryParse(String.fromCharCodes(text)) ?? fallback;
  }

  /// This node as a `String`, or `fallback` when it is not a string.
  String stringOr(String fallback) {
    final found = node();
    if (found == nullptr || found.ref.tag != Tag.string.code) return fallback;
    return utf8.decode(_stringBytes(found.ref.payload.text));
  }

  /// This whole node as plain Dart: `null`, `bool`, a number, `String`,
  /// `Uint8List`, `List` or `Map<String, Object?>`, with [absent] for a
  /// node that is not there.
  Object? toDart({Numbers numbers = Numbers.auto}) =>
      _nodeToDart(node(), numbers);

  /// This node as a map, or a [StateError] when it is not one.
  MapRef asMap() {
    if (tag != Tag.map) throw StateError('value is not a map');
    return MapRef(owner, path);
  }

  /// This node as a list, or a [StateError] when it is not one.
  ListRef asList() {
    if (tag != Tag.list) throw StateError('value is not a list');
    return ListRef(owner, path);
  }

  /// A deep, independent copy of this node, as its own [Value].
  Value clone() {
    final source = requireNode();
    final alloc = owner.allocator;
    return Value._build(
      alloc,
      (out) => native.core().bindings.guatiao_value_clone(alloc, source, out),
    );
  }

  @override
  String toString() => 'Ref($path)';
}

/// A [Ref] whose node is a map: the map protocol over it.
class MapRef extends Ref {
  MapRef(super.owner, [super.path]);

  int get length {
    final found = node();
    if (found == nullptr || found.ref.tag != Tag.map.code) return 0;
    return found.ref.payload.map.len;
  }

  bool get isEmpty => length == 0;

  bool get isNotEmpty => length != 0;

  bool containsKey(String key) => _mapFind(node(), _utf8Bytes(key)) != nullptr;

  /// The value under `key`, or `null` when there is no such entry.
  Ref? operator [](String key) {
    if (!containsKey(key)) return null;
    return Ref(owner, [...path, key]);
  }

  /// Stores `value` under `key`, replacing any entry in place.
  void operator []=(String key, Object? value) {
    final alloc = owner.allocator;
    final target = requireNode();
    _buildInto(
      alloc,
      value,
      (tmp) => using(
        (arena) => checkStatus(
          native.core().bindings.guatiao_map_set(
                alloc,
                target,
                strOf(arena, key).ref,
                tmp,
              ),
        ),
      ),
    );
  }

  /// Drops the entry under `key`, if there is one.
  void remove(String key) {
    final target = requireNode();
    using(
      (arena) => checkStatus(
        native.core().bindings.guatiao_map_discard(
              target,
              strOf(arena, key).ref,
            ),
      ),
    );
  }

  /// Every key, in insertion order.
  List<String> get keys {
    final found = requireNode();
    if (found.ref.tag != Tag.map.code) throw StateError('value is not a map');
    final map = found.ref.payload.map;
    return <String>[
      for (var i = 0; i < map.len; i++)
        utf8.decode(_stringBytes(_entryAt(map, i).ref.key)),
    ];
  }

  /// Every entry, as a key beside a [Ref] onto its value.
  Iterable<MapEntry<String, Ref>> get entries sync* {
    for (final key in keys) {
      yield MapEntry(key, Ref(owner, [...path, key]));
    }
  }

  void clear() =>
      checkStatus(native.core().bindings.guatiao_map_clear(requireNode()));

  /// Copies every entry of `src`, replacing collisions in place.
  void copyFrom(Ref src) {
    final alloc = owner.allocator;
    checkStatus(
      native.core().bindings.guatiao_map_copy_from(
            alloc,
            requireNode(),
            src.requireNode(),
          ),
    );
  }

  @override
  String toString() => 'MapRef($path)';
}

/// A [Ref] whose node is a list: the list protocol over it.
///
/// `guatiao_list` exports append, discard-by-index and clear, and no
/// native "set" or "insert" at a position. [operator []=] and [insert]
/// are built from what exists -- clone every element, clear, push the new
/// order back -- so they are O(n); [add] and [removeAt] go through their
/// own export.
class ListRef extends Ref {
  ListRef(super.owner, [super.path]);

  int get length {
    final found = node();
    if (found == nullptr || found.ref.tag != Tag.list.code) return 0;
    return found.ref.payload.list.len;
  }

  bool get isEmpty => length == 0;

  bool get isNotEmpty => length != 0;

  int _checkedIndex(int index) {
    final found = requireNode();
    if (found.ref.tag != Tag.list.code) throw StateError('value is not a list');
    final len = found.ref.payload.list.len;
    final i = index < 0 ? index + len : index;
    if (i < 0 || i >= len) {
      throw RangeError.index(index, this, 'index', null, len);
    }
    return i;
  }

  Ref operator [](int index) => Ref(owner, [...path, _checkedIndex(index)]);

  void operator []=(int index, Object? value) {
    final bindings = native.core().bindings;
    final elements = _clonedElements();
    try {
      final i = index < 0 ? index + elements.length : index;
      if (i < 0 || i >= elements.length) {
        throw RangeError.index(index, this, 'index', null, elements.length);
      }
      final replacement = calloc<guatiao_value>();
      try {
        _writeDart(replacement, value, owner.allocator);
      } catch (_) {
        bindings.guatiao_value_free(replacement);
        calloc.free(replacement);
        rethrow;
      }
      final old = elements[i];
      elements[i] = replacement;
      bindings.guatiao_value_free(old);
      calloc.free(old);
      _rebuild(elements);
    } finally {
      _releaseAll(elements);
    }
  }

  /// A [Ref] onto every element, in order.
  List<Ref> get refs => <Ref>[
        for (var i = 0; i < length; i++) Ref(owner, [...path, i])
      ];

  void add(Object? value) {
    final alloc = owner.allocator;
    final target = requireNode();
    _buildInto(
      alloc,
      value,
      (tmp) => checkStatus(
        native.core().bindings.guatiao_list_push(alloc, target, tmp),
      ),
    );
  }

  void insert(int index, Object? value) {
    final elements = _clonedElements();
    try {
      final length = elements.length;
      var i = index < 0 ? length + index : index;
      if (i < 0) i = 0;
      if (i > length) i = length;
      final replacement = calloc<guatiao_value>();
      try {
        _writeDart(replacement, value, owner.allocator);
      } catch (_) {
        native.core().bindings.guatiao_value_free(replacement);
        calloc.free(replacement);
        rethrow;
      }
      elements.insert(i, replacement);
      _rebuild(elements);
    } finally {
      _releaseAll(elements);
    }
  }

  void removeAt(int index) {
    final i = _checkedIndex(index);
    checkStatus(native.core().bindings.guatiao_list_discard(requireNode(), i));
  }

  void clear() =>
      checkStatus(native.core().bindings.guatiao_list_clear(requireNode()));

  /// Every element, deep-copied into its own scratch node. The caller
  /// owns each one until [_rebuild] moves it back in.
  List<Pointer<guatiao_value>> _clonedElements() {
    final bindings = native.core().bindings;
    final alloc = owner.allocator;
    final found = requireNode();
    if (found.ref.tag != Tag.list.code) throw StateError('value is not a list');
    final out = <Pointer<guatiao_value>>[];
    try {
      for (var i = 0; i < found.ref.payload.list.len; i++) {
        final tmp = calloc<guatiao_value>();
        out.add(tmp);
        checkStatus(
            bindings.guatiao_value_clone(alloc, _listAt(found, i), tmp));
      }
    } catch (_) {
      _releaseAll(out);
      out.clear();
      rethrow;
    }
    return out;
  }

  void _rebuild(List<Pointer<guatiao_value>> elements) {
    final bindings = native.core().bindings;
    final alloc = owner.allocator;
    checkStatus(bindings.guatiao_list_clear(requireNode()));
    for (final tmp in elements) {
      checkStatus(bindings.guatiao_list_push(alloc, requireNode(), tmp));
    }
  }

  /// Frees whatever the scratch nodes still own -- nothing, once
  /// [_rebuild] has moved them and left each null-tagged.
  void _releaseAll(List<Pointer<guatiao_value>> elements) {
    final bindings = native.core().bindings;
    for (final tmp in elements) {
      bindings.guatiao_value_free(tmp);
      calloc.free(tmp);
    }
  }

  @override
  String toString() => 'ListRef($path)';
}

NativeFinalizer? _finalizerCache;

NativeFinalizer _valueFinalizer() =>
    _finalizerCache ??= NativeFinalizer(native.core().valueFreeFinalizer);

/// Owns a root `guatiao_value` and the tree under it.
///
/// [close] is the release and the package requires it. A [NativeFinalizer]
/// is attached as a backstop, so a dropped value still releases its tree;
/// it cannot also reclaim the 40-byte root box, because a finalizer runs
/// one native function and those two frees must be ordered.
class Value extends Ref implements ValueOwner, Finalizable {
  Value._(this._raw, this._allocator) : super._(null, const []) {
    _valueFinalizer().attach(this, _raw.cast<Void>(), detach: this);
  }

  /// `Value(obj)` converts as [Value.fromDart] does; `Value()` is absent.
  factory Value([Object? obj = _unset]) {
    if (obj is _Unset) return Value.absent();
    return Value.fromDart(obj);
  }

  /// Converts plain Dart into a new value.
  ///
  /// `null` becomes null, [absent] becomes absent, `bool` a boolean,
  /// `int`/`double`/`BigInt` a number (a non-finite `double` is refused),
  /// `String` a string, a `TypedData` bytes, a `List` a list, a `Map` with
  /// `String` keys a map, and a [Ref] or [Value] a deep copy.
  factory Value.fromDart(Object? obj, {Pointer<guatiao_alloc>? alloc}) {
    final allocator = alloc ?? native.core().alloc;
    final raw = calloc<guatiao_value>();
    try {
      _writeDart(raw, obj, allocator);
    } catch (_) {
      native.core().bindings.guatiao_value_free(raw);
      calloc.free(raw);
      rethrow;
    }
    return Value._(raw, allocator);
  }

  /// Takes over a root the caller allocated with `calloc` and a native
  /// call filled in.
  factory Value.adopt(
    Pointer<guatiao_value> raw, {
    Pointer<guatiao_alloc>? alloc,
  }) =>
      Value._(raw, alloc ?? native.core().alloc);

  factory Value.nullValue() => _build(
        native.core().alloc,
        (out) => native.core().bindings.guatiao_value_null(out),
      );

  factory Value.absent() => _build(
        native.core().alloc,
        (out) => native.core().bindings.guatiao_value_absent(out),
      );

  factory Value.boolean(bool value) => _build(
        native.core().alloc,
        (out) => native.core().bindings.guatiao_value_bool(value, out),
      );

  factory Value.map() {
    final alloc = native.core().alloc;
    return _build(
      alloc,
      (out) => native.core().bindings.guatiao_value_map(alloc, out),
    );
  }

  factory Value.list() {
    final alloc = native.core().alloc;
    return _build(
      alloc,
      (out) => native.core().bindings.guatiao_value_list(alloc, out),
    );
  }

  factory Value.string(String text) {
    final alloc = native.core().alloc;
    return using(
      (arena) {
        final view = strOf(arena, text).ref;
        return _build(
          alloc,
          (out) =>
              native.core().bindings.guatiao_value_string(alloc, view, out),
        );
      },
    );
  }

  /// A number from its exact text, verbatim: no `double` detour.
  factory Value.number(String text) {
    final alloc = native.core().alloc;
    return using(
      (arena) {
        final view = strOf(arena, text).ref;
        return _build(
          alloc,
          (out) =>
              native.core().bindings.guatiao_value_number(alloc, view, out),
        );
      },
    );
  }

  factory Value.bytes(List<int> data) {
    final alloc = native.core().alloc;
    return using(
      (arena) {
        final view = _bytesView(arena, data).ref;
        return _build(
          alloc,
          (out) => native.core().bindings.guatiao_value_bytes(alloc, view, out),
        );
      },
    );
  }

  static Value _build(
    Pointer<guatiao_alloc> allocator,
    int Function(Pointer<guatiao_value>) call,
  ) {
    final raw = calloc<guatiao_value>();
    try {
      checkStatus(call(raw));
    } catch (_) {
      native.core().bindings.guatiao_value_free(raw);
      calloc.free(raw);
      rethrow;
    }
    return Value._(raw, allocator);
  }

  final Pointer<guatiao_value> _raw;
  final Pointer<guatiao_alloc> _allocator;
  bool _closed = false;

  /// The root node, for a native call that takes one. Invalid after
  /// [close].
  Pointer<guatiao_value> get pointer => rootNode;

  @override
  Pointer<guatiao_value> get rootNode {
    if (_closed) throw StateError('this value is closed');
    return _raw;
  }

  @override
  Pointer<guatiao_alloc> get allocator => _allocator;

  bool get isClosed => _closed;

  /// Frees the tree and the root box. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _valueFinalizer().detach(this);
    native.core().bindings.guatiao_value_free(_raw);
    calloc.free(_raw);
  }

  @override
  String toString() => _closed ? 'Value(closed)' : 'Value(${toDart()})';
}
