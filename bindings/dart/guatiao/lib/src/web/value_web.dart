// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The value model over a WebAssembly module's memory: the classes and
/// behaviour of `value.dart`, with every node an [Address] in the module.
library;

import 'dart:convert';
import 'dart:typed_data';

import '../status.dart';
import '../value_common.dart';
import 'module.dart';

export '../value_common.dart';

/// What a [Ref] walks from: a root node and the allocator to build into.
abstract class ValueOwner {
  /// The root `guatiao_value`. Throws when it is no longer usable.
  Address get rootNode;

  /// The allocator new content is built through. Throws for a root this
  /// binding only borrows.
  Address get allocator;
}

/// A root this binding reads but does not own. See [Ref.borrowed] and
/// [Ref.borrowedFrom].
class _BorrowedRoot implements ValueOwner {
  _BorrowedRoot(this._root, {this.keep, this.find});

  final Address _root;
  final ValueOwner? keep;
  final Address Function()? find;

  @override
  Address get rootNode {
    // Throws when the owner is closed.
    keep?.rootNode;
    final again = find;
    return again == null ? _root : again();
  }

  @override
  Address get allocator => throw StateError('a borrowed value is read-only');
}

void _writable(Ref ref) {
  if (ref.owner is _BorrowedRoot) {
    throw StateError('a borrowed value is read-only');
  }
}

// ---- reading -----------------------------------------------------------
//
// The `static inline` readers of `guatiao.h`, over the module's memory:
// a value is `tag` at 0 and its payload at 8, a container's array pointer
// at 8 and its length at 8 + a word.

WasmModule get _m => WasmModule.current;

int _tagOf(Address node) => node.isNull ? Tag.absent.code : _m.u32(node);

Uint8List _payloadBytes(Address node) =>
    _m.bytes(_m.word(node + 8), _m.u32(node + 8 + wordSize));

int _lengthOf(Address node) => _m.u32(node + 8 + wordSize);

Address _listAt(Address node, int index) {
  if (node.isNull || _tagOf(node) != Tag.list.code) return Address.none;
  final ptr = _m.word(node + 8);
  if (ptr.isNull) return Address.none;
  final len = _lengthOf(node);
  var i = index;
  if (i < 0) i += len;
  if (i < 0 || i >= len) return Address.none;
  return ptr + i * valueSize;
}

Address _entryAt(Address node, int index) =>
    _m.word(node + 8) + index * entrySize;

Uint8List _entryKey(Address entry) =>
    _m.bytes(_m.word(entry), _m.u32(entry + wordSize));

bool _bytesEqual(Uint8List a, Uint8List b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

Address _mapFind(Address node, Uint8List key) {
  if (node.isNull || _tagOf(node) != Tag.map.code) return Address.none;
  if (_m.word(node + 8).isNull) return Address.none;
  final len = _lengthOf(node);
  for (var i = 0; i < len; i++) {
    final entry = _entryAt(node, i);
    if (_bytesEqual(_entryKey(entry), key)) return entry + entryValueAt;
  }
  return Address.none;
}

Address _walk(Address root, List<Object> path) {
  var node = root;
  for (final part in path) {
    node = part is int
        ? _listAt(node, part)
        : _mapFind(node, Uint8List.fromList(utf8.encode(part as String)));
    if (node.isNull) return Address.none;
  }
  return node;
}

Uint8List _numberText(Address node) {
  if (node.isNull || _tagOf(node) != Tag.number.code) return Uint8List(0);
  return _payloadBytes(node);
}

Object? _nodeToDart(Address node, Numbers numbers) {
  if (node.isNull) return absent;
  switch (Tag.fromCode(_tagOf(node))) {
    case Tag.absent:
    case null: // a tag this binding does not know: skip, as the header asks
      return absent;
    case Tag.nullValue:
      return null;
    case Tag.boolean:
      return _m.u8(node + 8) != 0;
    case Tag.number:
      return numberFromText(String.fromCharCodes(_payloadBytes(node)), numbers);
    case Tag.string:
      return utf8.decode(_payloadBytes(node));
    case Tag.bytes:
      return _payloadBytes(node);
    case Tag.list:
      final len = _lengthOf(node);
      return <Object?>[
        for (var i = 0; i < len; i++) _nodeToDart(_listAt(node, i), numbers),
      ];
    case Tag.map:
      final len = _lengthOf(node);
      final out = <String, Object?>{};
      for (var i = 0; i < len; i++) {
        final entry = _entryAt(node, i);
        out[utf8.decode(_entryKey(entry))] = _nodeToDart(
          entry + entryValueAt,
          numbers,
        );
      }
      return out;
  }
}

/// Reads [node] as plain Dart, then frees its tree and the box it sits
/// in, a `guatiao_memory_alloc` block of one value.
Object? takeValue(Address node, {Numbers numbers = Numbers.auto}) {
  try {
    return _nodeToDart(node, numbers);
  } finally {
    freeRoot(node);
  }
}

/// Frees what [node] owns, then the one-value block it sits in.
void freeRoot(Address node) {
  final m = _m;
  m.call('guatiao_value_free', [node]);
  m.free(node, valueSize);
}

// ---- writing -------------------------------------------------------------

String _textForNumber(Object value) {
  if (value is double && !value.isFinite) {
    throw ArgumentError.value(value, 'value', 'is not a finite number');
  }
  return value.toString();
}

/// Writes [obj] into [raw], which must be absent- or null-tagged.
void _writeDart(Address raw, Object? obj, Address alloc) {
  final m = _m;
  if (obj == null) {
    checkStatus(m.call('guatiao_value_null', [raw]));
    return;
  }
  if (obj is Absent) {
    checkStatus(m.call('guatiao_value_absent', [raw]));
    return;
  }
  if (obj is Ref) {
    checkStatus(m.call('guatiao_value_clone', [alloc, obj.requireNode(), raw]));
    return;
  }
  if (obj is bool) {
    checkStatus(m.call('guatiao_value_bool', [obj, raw]));
    return;
  }
  if (obj is num || obj is BigInt) {
    final text = _textForNumber(obj);
    withScratch(
      (s) => checkStatus(
        m.call('guatiao_value_number', [alloc, s.str(text), raw]),
      ),
    );
    return;
  }
  if (obj is String) {
    withScratch(
      (s) =>
          checkStatus(m.call('guatiao_value_string', [alloc, s.str(obj), raw])),
    );
    return;
  }
  if (obj is TypedData) {
    final data = obj is Uint8List
        ? obj
        : Uint8List.view(obj.buffer, obj.offsetInBytes, obj.lengthInBytes);
    withScratch(
      (s) => checkStatus(
        m.call('guatiao_value_bytes', [alloc, s.view(data), raw]),
      ),
    );
    return;
  }
  if (obj is List) {
    checkStatus(m.call('guatiao_value_list', [alloc, raw]));
    for (final item in obj) {
      _buildInto(
        alloc,
        item,
        (tmp) => checkStatus(m.call('guatiao_list_push', [alloc, raw, tmp])),
      );
    }
    return;
  }
  if (obj is Map) {
    checkStatus(m.call('guatiao_value_map', [alloc, raw]));
    for (final entry in obj.entries) {
      final key = entry.key;
      if (key is! String) {
        throw ArgumentError.value(key, 'key', 'a map key must be a String');
      }
      _buildInto(
        alloc,
        entry.value,
        (tmp) => withScratch(
          (s) => checkStatus(
            m.call('guatiao_map_set', [alloc, raw, s.str(key), tmp]),
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

/// Builds [obj] into a scratch node and hands it to [consume], which moves
/// it somewhere that owns it; what a failure left behind is freed after.
void _buildInto(Address alloc, Object? obj, void Function(Address) consume) {
  final tmp = _m.alloc(valueSize);
  try {
    _writeDart(tmp, obj, alloc);
    consume(tmp);
  } finally {
    freeRoot(tmp);
  }
}

// ---- the tree ------------------------------------------------------------

/// A node inside a [Value]'s tree: the owning value plus a path of keys
/// and indices, walked again on every access.
class Ref {
  Ref(ValueOwner owner, [List<Object> path = const []])
    : this._(owner, List<Object>.unmodifiable(path));

  Ref._(this._ownerOrNull, this.path);

  /// A read-only view of a tree this binding does not own, rooted at
  /// [root] in the module's memory. The caller keeps that memory alive
  /// and unchanged while the view is read; nothing frees it.
  factory Ref.borrowed(Address root) => Ref(_BorrowedRoot(root));

  /// A read-only view of a node inside [source]'s tree, which [find]
  /// locates again on every access. It throws [StateError] once [source]
  /// is closed; [find] answers [Address.none] when the node is gone.
  factory Ref.borrowedFrom(Ref source, Address Function() find) =>
      Ref(_BorrowedRoot(Address.none, keep: source.owner, find: find));

  final ValueOwner? _ownerOrNull;

  /// The keys and indices from the owning value's root to this node.
  final List<Object> path;

  ValueOwner get owner => _ownerOrNull ?? this as ValueOwner;

  /// This node, or [Address.none] when the path no longer leads anywhere.
  Address node() => _walk(owner.rootNode, path);

  /// This node, or a [StateError] naming the path that found nothing.
  Address requireNode() {
    final found = node();
    if (found.isNull) throw StateError('no value at $path');
    return found;
  }

  Tag get tag => Tag.fromCode(_tagOf(node())) ?? Tag.absent;

  bool get isAbsent => tag == Tag.absent;

  bool get isNull => tag == Tag.nullValue;

  bool boolOr(bool fallback) {
    final found = node();
    if (found.isNull || _tagOf(found) != Tag.boolean.code) return fallback;
    return _m.u8(found + 8) != 0;
  }

  /// This node as an `int`, or [fallback]: a wrong kind, a fractional or
  /// exponent spelling, and a value past what an `int` holds exactly all
  /// fall back.
  int intOr(int fallback) {
    final text = _numberText(node());
    if (text.isEmpty || text.length >= 48) return fallback;
    return intFromText(String.fromCharCodes(text)) ?? fallback;
  }

  double doubleOr(double fallback) {
    final text = _numberText(node());
    if (text.isEmpty || text.length >= 344) return fallback;
    return double.tryParse(String.fromCharCodes(text)) ?? fallback;
  }

  String stringOr(String fallback) {
    final found = node();
    if (found.isNull || _tagOf(found) != Tag.string.code) return fallback;
    return utf8.decode(_payloadBytes(found));
  }

  Object? toDart({Numbers numbers = Numbers.auto}) =>
      _nodeToDart(node(), numbers);

  MapRef asMap() {
    if (tag != Tag.map) throw StateError('value is not a map');
    return MapRef(owner, path);
  }

  ListRef asList() {
    if (tag != Tag.list) throw StateError('value is not a list');
    return ListRef(owner, path);
  }

  /// This node in the wire encoding.
  Uint8List toWire() {
    final source = requireNode();
    final m = _m;
    final out = m.alloc(valueSize);
    try {
      checkStatus(m.call('guatiao_wire_encode', [m.defaultAlloc, source, out]));
      return _payloadBytes(out);
    } finally {
      freeRoot(out);
    }
  }

  /// A deep, independent copy of this node, as its own [Value].
  Value clone() {
    final source = requireNode();
    final alloc = owner is _BorrowedRoot ? _m.defaultAlloc : owner.allocator;
    return Value._build(
      alloc,
      (out) => _m.call('guatiao_value_clone', [alloc, source, out]),
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
    if (found.isNull || _tagOf(found) != Tag.map.code) return 0;
    return _lengthOf(found);
  }

  bool get isEmpty => length == 0;

  bool get isNotEmpty => length != 0;

  bool containsKey(String key) =>
      !_mapFind(node(), Uint8List.fromList(utf8.encode(key))).isNull;

  Ref? operator [](String key) {
    if (!containsKey(key)) return null;
    return Ref(owner, [...path, key]);
  }

  void operator []=(String key, Object? value) {
    _writable(this);
    final alloc = owner.allocator;
    _buildInto(
      alloc,
      value,
      (tmp) => withScratch(
        (s) => checkStatus(
          _m.call('guatiao_map_set', [alloc, requireNode(), s.str(key), tmp]),
        ),
      ),
    );
  }

  void remove(String key) {
    _writable(this);
    final target = requireNode();
    withScratch(
      (s) => checkStatus(_m.call('guatiao_map_discard', [target, s.str(key)])),
    );
  }

  List<String> get keys {
    final found = requireNode();
    if (_tagOf(found) != Tag.map.code) throw StateError('value is not a map');
    final len = _lengthOf(found);
    return <String>[
      for (var i = 0; i < len; i++) utf8.decode(_entryKey(_entryAt(found, i))),
    ];
  }

  Iterable<MapEntry<String, Ref>> get entries sync* {
    for (final key in keys) {
      yield MapEntry(key, Ref(owner, [...path, key]));
    }
  }

  void clear() {
    _writable(this);
    checkStatus(_m.call('guatiao_map_clear', [requireNode()]));
  }

  void copyFrom(Ref src) {
    _writable(this);
    checkStatus(
      _m.call('guatiao_map_copy_from', [
        owner.allocator,
        requireNode(),
        src.requireNode(),
      ]),
    );
  }

  @override
  String toString() => 'MapRef($path)';
}

/// A [Ref] whose node is a list: the list protocol over it. [operator []=]
/// and [insert] are built from append, discard and clear, so they are
/// O(n).
class ListRef extends Ref {
  ListRef(super.owner, [super.path]);

  int get length {
    final found = node();
    if (found.isNull || _tagOf(found) != Tag.list.code) return 0;
    return _lengthOf(found);
  }

  bool get isEmpty => length == 0;

  bool get isNotEmpty => length != 0;

  int _checkedIndex(int index) {
    final found = requireNode();
    if (_tagOf(found) != Tag.list.code) throw StateError('value is not a list');
    final len = _lengthOf(found);
    final i = index < 0 ? index + len : index;
    if (i < 0 || i >= len) {
      throw RangeError.index(index, this, 'index', null, len);
    }
    return i;
  }

  Ref operator [](int index) => Ref(owner, [...path, _checkedIndex(index)]);

  void operator []=(int index, Object? value) {
    _writable(this);
    final elements = _clonedElements();
    try {
      final i = index < 0 ? index + elements.length : index;
      if (i < 0 || i >= elements.length) {
        throw RangeError.index(index, this, 'index', null, elements.length);
      }
      final replacement = _m.alloc(valueSize);
      try {
        _writeDart(replacement, value, owner.allocator);
      } catch (_) {
        freeRoot(replacement);
        rethrow;
      }
      final old = elements[i];
      elements[i] = replacement;
      freeRoot(old);
      _rebuild(elements);
    } finally {
      _releaseAll(elements);
    }
  }

  List<Ref> get refs => <Ref>[
    for (var i = 0; i < length; i++) Ref(owner, [...path, i]),
  ];

  void add(Object? value) {
    _writable(this);
    final alloc = owner.allocator;
    _buildInto(
      alloc,
      value,
      (tmp) => checkStatus(
        _m.call('guatiao_list_push', [alloc, requireNode(), tmp]),
      ),
    );
  }

  void insert(int index, Object? value) {
    _writable(this);
    final elements = _clonedElements();
    try {
      final length = elements.length;
      var i = index < 0 ? length + index : index;
      if (i < 0) i = 0;
      if (i > length) i = length;
      final replacement = _m.alloc(valueSize);
      try {
        _writeDart(replacement, value, owner.allocator);
      } catch (_) {
        freeRoot(replacement);
        rethrow;
      }
      elements.insert(i, replacement);
      _rebuild(elements);
    } finally {
      _releaseAll(elements);
    }
  }

  void removeAt(int index) {
    _writable(this);
    final i = _checkedIndex(index);
    checkStatus(_m.call('guatiao_list_discard', [requireNode(), i]));
  }

  void clear() {
    _writable(this);
    checkStatus(_m.call('guatiao_list_clear', [requireNode()]));
  }

  /// Every element deep-copied into a scratch node of its own.
  List<Address> _clonedElements() {
    final alloc = owner.allocator;
    final found = requireNode();
    if (_tagOf(found) != Tag.list.code) throw StateError('value is not a list');
    final out = <Address>[];
    try {
      final len = _lengthOf(found);
      for (var i = 0; i < len; i++) {
        final tmp = _m.alloc(valueSize);
        out.add(tmp);
        // Found again each time: the clone may have grown the memory, and
        // an address stays valid where a view would not.
        checkStatus(
          _m.call('guatiao_value_clone', [
            alloc,
            _listAt(requireNode(), i),
            tmp,
          ]),
        );
      }
    } catch (_) {
      _releaseAll(out);
      out.clear();
      rethrow;
    }
    return out;
  }

  void _rebuild(List<Address> elements) {
    final alloc = owner.allocator;
    checkStatus(_m.call('guatiao_list_clear', [requireNode()]));
    for (final tmp in elements) {
      checkStatus(_m.call('guatiao_list_push', [alloc, requireNode(), tmp]));
    }
  }

  void _releaseAll(List<Address> elements) {
    for (final tmp in elements) {
      freeRoot(tmp);
    }
  }

  @override
  String toString() => 'ListRef($path)';
}

/// Frees a dropped value's tree and its root block, in that order.
final Finalizer<(WasmModule, Address)> _backstop =
    Finalizer<(WasmModule, Address)>((held) {
      final (module, root) = held;
      if (!identical(WasmModule.loaded, module)) return;
      module.call('guatiao_value_free', [root]);
      module.free(root, valueSize);
    });

/// Owns a root `guatiao_value` in the module's memory and the tree under
/// it. [close] releases both and is required; a [Finalizer] is attached as
/// a backstop for a dropped value.
class Value extends Ref implements ValueOwner {
  Value._(this._raw, this._allocator)
    : _module = WasmModule.current,
      super._(null, const []) {
    _backstop.attach(this, (_module, _raw), detach: this);
  }

  /// `Value(obj)` converts as [Value.fromDart] does; `Value()` is absent.
  factory Value([Object? obj = _unset]) {
    if (obj is _Unset) return Value.absent();
    return Value.fromDart(obj);
  }

  /// Converts plain Dart into a new value. Under dart2js a `double` with
  /// no fraction is an `int` and is written as one.
  factory Value.fromDart(Object? obj, {Address? alloc}) {
    final m = _m;
    final allocator = alloc ?? m.defaultAlloc;
    final raw = m.alloc(valueSize);
    try {
      _writeDart(raw, obj, allocator);
    } catch (_) {
      freeRoot(raw);
      rethrow;
    }
    return Value._(raw, allocator);
  }

  /// Takes over a root a call filled in, in a block of one value from
  /// `guatiao_memory_alloc`.
  factory Value.adopt(Address raw, {Address? alloc}) =>
      Value._(raw, alloc ?? _m.defaultAlloc);

  factory Value.fromWire(List<int> data, {Address? alloc}) {
    final allocator = alloc ?? _m.defaultAlloc;
    return withScratch((s) {
      final view = s.view(data);
      return _build(
        allocator,
        (out) => _m.call('guatiao_wire_decode', [allocator, view, out]),
      );
    });
  }

  factory Value.nullValue() =>
      _build(_m.defaultAlloc, (out) => _m.call('guatiao_value_null', [out]));

  factory Value.absent() =>
      _build(_m.defaultAlloc, (out) => _m.call('guatiao_value_absent', [out]));

  factory Value.boolean(bool value) => _build(
    _m.defaultAlloc,
    (out) => _m.call('guatiao_value_bool', [value, out]),
  );

  factory Value.map() {
    final alloc = _m.defaultAlloc;
    return _build(alloc, (out) => _m.call('guatiao_value_map', [alloc, out]));
  }

  factory Value.list() {
    final alloc = _m.defaultAlloc;
    return _build(alloc, (out) => _m.call('guatiao_value_list', [alloc, out]));
  }

  factory Value.string(String text) {
    final alloc = _m.defaultAlloc;
    return withScratch((s) {
      final view = s.str(text);
      return _build(
        alloc,
        (out) => _m.call('guatiao_value_string', [alloc, view, out]),
      );
    });
  }

  /// A number from its exact text, verbatim.
  factory Value.number(String text) {
    final alloc = _m.defaultAlloc;
    return withScratch((s) {
      final view = s.str(text);
      return _build(
        alloc,
        (out) => _m.call('guatiao_value_number', [alloc, view, out]),
      );
    });
  }

  factory Value.bytes(List<int> data) {
    final alloc = _m.defaultAlloc;
    return withScratch((s) {
      final view = s.view(data);
      return _build(
        alloc,
        (out) => _m.call('guatiao_value_bytes', [alloc, view, out]),
      );
    });
  }

  static Value _build(Address allocator, int Function(Address) call) {
    final raw = _m.alloc(valueSize);
    try {
      checkStatus(call(raw));
    } catch (_) {
      freeRoot(raw);
      rethrow;
    }
    return Value._(raw, allocator);
  }

  final Address _raw;
  final Address _allocator;
  final WasmModule _module;
  bool _closed = false;

  /// The root node, for a call that takes one. Invalid after [close].
  Address get pointer => rootNode;

  @override
  Address get rootNode {
    if (_closed) throw StateError('this value is closed');
    if (!identical(_module, WasmModule.loaded)) {
      throw StateError('this value lives in a module that is no longer loaded');
    }
    return _raw;
  }

  @override
  Address get allocator => _allocator;

  bool get isClosed => _closed;

  /// Frees the tree and the root block. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _backstop.detach(this);
    if (!identical(_module, WasmModule.loaded)) return;
    freeRoot(_raw);
  }

  @override
  String toString() => _closed ? 'Value(closed)' : 'Value(${toDart()})';
}

class _Unset {
  const _Unset();
}

const _Unset _unset = _Unset();

/// A `guatiao_str` over a copy of [text], valid until [scratch] is
/// released.
Address strOf(Scratch scratch, String text) => scratch.str(text);
