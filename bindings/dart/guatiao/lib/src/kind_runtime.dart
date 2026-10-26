// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A kind's table from Dart: calling one through a class, and serving one
/// from a Dart object. The code `dart run guatiao:kind` writes for a kind
/// is built on what is here: a [Kind] subclass, a [KindRemote] subclass,
/// and per slot a call through [CallScope] and a shim through
/// [ServeScope].
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'kinds.dart' show FloorMismatch;
import 'library.dart' as native;
import 'registry.dart' show ProviderTable;
import 'value.dart';

/// Size of a pointer: one slot of a table.
int get _word => sizeOf<IntPtr>();

/// `guatiao_kind_header`: two `uint32_t`.
const int _header = 8;

/// One slot of a kind: its name in the table, and whether every table
/// carries it and whether it can refuse.
final class KindSlot {
  const KindSlot(this.name, {this.required = false, this.fallible = false});

  /// The slot's field in the C table (`greet_into`).
  final String name;

  /// A slot every table carries: the kind's original ones.
  final bool required;

  /// It ends in `guatiao_provider_error *err`.
  final bool fallible;
}

/// What an object whose holder destroys it, or a provider instance its
/// host destroys, is told: [dispose] runs once, when that happens.
abstract interface class Disposable {
  void dispose();
}

/// A kind: its name, floor hash and slots, how a caller binds a table of
/// it, and how a Dart object is served as one.
///
/// Each kind is a subclass the code `dart run guatiao:kind` writes,
/// reached as the static `kind` of the kind's interface (`Greeter.kind`).
/// `R` is what a caller holds.
abstract class Kind<R extends KindRemote> {
  Kind({
    required this.name,
    required this.floorHash,
    required this.isObject,
    required this.slots,
  });

  /// What a descriptor and a registry call it.
  final String name;

  /// The `<table>_FLOOR_HASH` its header defines.
  final int floorHash;

  /// An object kind: `destroy` comes first, and one holder owns an object.
  final bool isObject;

  /// The slots in table order, `destroy` aside.
  final List<KindSlot> slots;

  /// The table's size in bytes: the header, `destroy`, a word per slot.
  int get tableSize => _first + _word * slots.length;

  int get _first => _header + (isObject ? _word : 0);

  /// The offset of slot [index] in the table.
  int offsetOf(int index) => _first + _word * index;

  /// A table of this kind whose every slot is null, made once: what a
  /// stand-in remote is over, for a call refused before its method runs.
  late final Pointer<Void> _standIn = () {
    final table = calloc<Uint8>(tableSize).cast<guatiao_kind_header>();
    table.ref.struct_size = tableSize;
    table.ref.floor_hash = floorHash;
    return table.cast<Void>();
  }();

  /// The smallest table this kind accepts: up to its last required slot.
  int get floor {
    var end = isObject ? _first : _header;
    for (var i = 0; i < slots.length; i++) {
      if (slots[i].required) end = offsetOf(i) + _word;
    }
    return end;
  }

  /// A caller's remote over a table of this kind; with [scope], valid for
  /// that call only; [owned], an object the caller destroys. Generated.
  R remote(
    Pointer<Void> table,
    int size,
    Pointer<Void> ctx, {
    SlotCall? scope,
    bool owned = false,
  });

  /// Whether [target] implements slot [index]: the kind's interface for a
  /// required slot, the slot's own interface for an appended one.
  /// Generated.
  bool slotServedBy(Object target, int index);

  /// The same for every object of type [T]. Generated.
  bool slotServedByType<T>(int index);

  /// The native function serving slot [index] over the object `ctx`
  /// names. Generated.
  Pointer<Void> shim(int index);

  /// A caller's remote over [found], checked: [FloorMismatch] for no
  /// table, one shorter than the required slots, or another kind's hash.
  R bind(ProviderTable found) => remote(found.table, found.size, found.ctx);

  /// An object of this kind the caller owns: its `close()` destroys it.
  R handle(Pointer<Void> table, int size, Pointer<Void> ctx) =>
      remote(table, size, ctx, owned: true);

  String get _requiredNames => [
    for (final s in slots)
      if (s.required) s.name,
  ].join(', ');

  /// The table serving this kind through [target]'s methods, built once
  /// per set of slots implemented and kept for the process.
  /// [ArgumentError] when [target] lacks a required slot.
  Pointer<Void> tableFor(Object target, {String? what}) => _tableFor([
    for (var i = 0; i < slots.length; i++) slotServedBy(target, i),
  ], what ?? 'the object');

  /// The same for every object of type [T].
  Pointer<Void> tableForType<T>({String? what}) => _tableFor([
    for (var i = 0; i < slots.length; i++) slotServedByType<T>(i),
  ], what ?? '$T');

  final Map<String, Pointer<Void>> _tables = <String, Pointer<Void>>{};

  Pointer<Void> _tableFor(List<bool> has, String what) {
    final missing = [
      for (var i = 0; i < slots.length; i++)
        if (slots[i].required && !has[i]) slots[i].name,
    ];
    if (missing.isNotEmpty) {
      throw ArgumentError(
        '$what does not implement ${missing.join(', ')}, which the $name '
        'kind requires (its required slots: $_requiredNames)',
      );
    }
    final key = has.map((h) => h ? '1' : '0').join();
    final found = _tables[key];
    if (found != null) return found;
    final table = calloc<Uint8>(tableSize);
    table.cast<guatiao_kind_header>().ref
      ..struct_size = tableSize
      ..floor_hash = floorHash;
    if (isObject) {
      (table + _header).cast<Pointer<Void>>().value = _destroyShim.cast();
    }
    for (var i = 0; i < slots.length; i++) {
      if (has[i]) (table + offsetOf(i)).cast<Pointer<Void>>().value = shim(i);
    }
    return _tables[key] = table.cast();
  }

  /// [target] as a `guatiao_object` of this kind, written into [out]: held
  /// until its holder destroys it, or, with [scope], until the call ends.
  void _serve(Object target, Pointer<guatiao_object> out, {SlotCall? scope}) {
    final table = tableFor(target);
    final ctx = hold(target);
    if (scope != null) scope.after(() => release(ctx));
    out.ref
      ..table = table
      ..size = tableSize
      ..ctx = Pointer.fromAddress(ctx);
  }

  @override
  String toString() => 'Kind($name)';
}

// ---- a caller's remote ----------------------------------------------------

final Finalizer<_Owned> _ownedObjects = Finalizer<_Owned>((o) => o.destroy());

/// What an object handle owns: its table and `ctx`, destroyed once.
final class _Owned {
  _Owned(this.table, this.size, this.ctx);

  final Pointer<Void> table;
  final int size;
  final Pointer<Void> ctx;
  bool done = false;

  void destroy() {
    if (done) return;
    done = true;
    _destroy(table, size, ctx);
  }
}

typedef _DestroyC = Void Function(Pointer<Void>);
typedef _DestroyD = void Function(Pointer<Void>);

/// Calls an object's `destroy`, the slot right after its header.
void _destroy(Pointer<Void> table, int size, Pointer<Void> ctx) {
  if (table == nullptr || size < _header + _word) return;
  final slot = (table.cast<Uint8>() + _header)
      .cast<Pointer<NativeFunction<_DestroyC>>>()
      .value;
  if (slot == nullptr) return;
  slot.asFunction<_DestroyD>()(ctx);
}

/// A caller's handle on a table of a kind: a method per slot (in the
/// generated subclass), [has], and what it is.
///
/// A remote of a provider's table is valid while its library is; one lent
/// to a method is valid for that call. An object handle, an object of an
/// object kind the caller owns, is destroyed by [close], once, or when it
/// is collected unclosed.
abstract class KindRemote {
  KindRemote(
    this.kind,
    this.table,
    this.size,
    this.ctx, {
    SlotCall? scope,
    bool owned = false,
  }) : _scope = scope {
    if (table == nullptr) throw FloorMismatch('no ${kind.name} table');
    if (size < kind.floor) {
      throw FloorMismatch(
        'the ${kind.name} table is $size bytes, its required slots need '
        '${kind.floor}',
      );
    }
    final hash = table.cast<guatiao_kind_header>().ref.floor_hash;
    if (hash != kind.floorHash) {
      throw FloorMismatch(
        "the table's floor hash $hash is not the ${kind.name} kind's "
        '${kind.floorHash}',
      );
    }
    if (owned) {
      final held = _Owned(table, size, ctx);
      _held = held;
      _ownedObjects.attach(this, held, detach: this);
    }
  }

  final Kind kind;
  final Pointer<Void> table;
  final int size;

  /// What every slot is passed first.
  final Pointer<Void> ctx;

  final SlotCall? _scope;
  _Owned? _held;
  bool _closed = false;

  /// Whether the table carries the slot named [slotName] (its C name).
  bool has(String slotName) {
    _checkOpen();
    final index = kind.slots.indexWhere((s) => s.name == slotName);
    return index >= 0 && _slotAt(index) != nullptr;
  }

  void _checkOpen() {
    _scope?.checkOpen();
    if (_closed) throw StateError('this object is closed, or was handed over');
  }

  Pointer<Void> _slotAt(int index) {
    final at = kind.offsetOf(index);
    if (at + _word > size) return nullptr;
    return (table.cast<Uint8>() + at).cast<Pointer<Void>>().value;
  }

  /// Slot [index]'s function, for the generated method: [Unsupported]
  /// when the table is too short to carry it or carries null.
  Pointer<Void> slot(int index) {
    _checkOpen();
    final fn = _slotAt(index);
    if (fn == nullptr) {
      throw Unsupported(
        Status.unsupported.code,
        'this ${kind.name} table has no ${kind.slots[index].name}',
      );
    }
    return fn;
  }

  /// Whether this handle was closed or handed over.
  bool get isClosed => _closed;

  /// Destroys an object this side owns; does nothing for any other remote.
  /// A second call does nothing.
  void close() {
    final held = _held;
    if (held == null || _closed) return;
    _closed = true;
    _ownedObjects.detach(this);
    held.destroy();
  }

  /// The object as a `guatiao_object` for one call, still this handle's.
  void _lendInto(Pointer<guatiao_object> out) {
    _checkOpen();
    out.ref
      ..table = table
      ..size = size
      ..ctx = ctx;
  }

  /// Hands the object over: from now on someone else destroys it.
  void _giveInto(Pointer<guatiao_object> out) {
    final held = _held;
    if (held == null) {
      throw StateError(
        'only an object this side owns can be handed over; lend this one',
      );
    }
    _lendInto(out);
    _closed = true;
    held.done = true;
    _ownedObjects.detach(this);
  }

  @override
  String toString() => '${_held == null ? 'Remote' : 'Object'} ${kind.name}';
}

// ---- objects a C caller can reach ---------------------------------------

/// Each served object under a `ctx` that is a number shaped like an
/// address, never reused, so a call naming one that was released answers
/// `GUATIAO_ERR_GONE` rather than reaching another.
final Map<int, Object> _live = <int, Object>{};
int _nextCtx = 0x10000;

/// A `ctx` naming [target] until [release].
int hold(Object target) {
  _nextCtx += 16;
  _live[_nextCtx] = target;
  return _nextCtx;
}

/// Lets go of what [ctx] names; with [dispose], a [Disposable] is told.
void release(int ctx, {bool dispose = false}) {
  final target = _live.remove(ctx);
  if (dispose && target is Disposable) target.dispose();
}

/// How many objects a C caller can still reach: served and not yet
/// destroyed or released.
int liveObjects() => _live.length;

void _destroyServed(Pointer<Void> ctx) {
  try {
    release(ctx.address, dispose: true);
  } on Object catch (e) {
    stderr.writeln('guatiao: disposing a destroyed object threw $e');
  }
}

final Pointer<NativeFunction<_DestroyC>> _destroyShim =
    Pointer.fromFunction<_DestroyC>(_destroyServed);

// ---- trees -------------------------------------------------------------

Pointer<guatiao_alloc> get _binding => native.core().alloc;

/// Whether [alloc] can build: its size reaches past `free`, and both
/// functions are there. `struct_size`, `ctx`, `alloc`, `free` are four
/// words on 32- and 64-bit alike.
bool _complete(Pointer<guatiao_alloc> alloc) {
  if (alloc == nullptr) return false;
  final a = alloc.ref;
  return a.struct_size >= _word * 4 && a.alloc != nullptr && a.free != nullptr;
}

/// `guatiao_value.payload`'s offset: after the tag and its padding.
int get _payload => sizeOf<guatiao_value>() - sizeOf<guatiao_payload>();

Pointer<NativeType> _armOf(Pointer<guatiao_value> node) =>
    (node.cast<Uint8>() + _payload).cast();

/// The allocator a container node records, else this binding's.
Pointer<guatiao_alloc> _recorded(Pointer<guatiao_value> node) {
  final tag = node.ref.tag;
  if (tag >= Tag.number.code && tag <= Tag.map.code) {
    final named = _armOf(node).cast<guatiao_map>().ref.alloc;
    if (_complete(named)) return named;
  }
  return _binding;
}

void _copy(Pointer<NativeType> to, Pointer<NativeType> from, int size) {
  to
      .cast<Uint8>()
      .asTypedList(size)
      .setAll(0, from.cast<Uint8>().asTypedList(size));
}

void _free(Pointer<guatiao_value> node) {
  native.core().bindings.guatiao_value_free(node);
  calloc.free(node);
}

/// Whether the core moves a tree home: a library older than
/// `guatiao_value_make_local` leaves an answer as the slot built it.
final bool _makesLocal = native.core().provides('guatiao_value_make_local');

/// [node] moved into this binding's allocator so it outlives the library
/// that built it: a tree a slot answered, which a slot that ignored the
/// carrier built in its own, or one a caller handed over ([what] says
/// which). Freed, and the refusal thrown, when the move fails.
Pointer<guatiao_value> _homed(
  Pointer<guatiao_value> node, [
  String what = 'the answer',
]) {
  if (!_makesLocal) return node;
  final status = native.core().bindings.guatiao_value_make_local(
    node,
    _binding,
  );
  if (status != Status.ok.code) {
    _free(node);
    throw statusException(status, "moving $what into this binding's allocator");
  }
  return node;
}

/// [node], a tree lent for a call, moved where it stands into [home],
/// the allocator it recorded when it was lent, so what the callee grew it
/// with, or wrote over it, outlives the callee's library. A refusal leaves
/// it whole, where it was, and is the status answered.
int _settle(Pointer<guatiao_value> node, Pointer<guatiao_alloc> home) =>
    _makesLocal
    ? native.core().bindings.guatiao_value_make_local(node, home)
    : Status.ok.code;

/// A new `calloc`ed root holding a copy of the struct at [arm]: a whole
/// node when [tag] is null, else a map or list.
Pointer<guatiao_value> _nodeOver(Pointer<NativeType> arm, Tag? tag) {
  final node = calloc<guatiao_value>();
  if (tag == null) {
    _copy(node, arm, sizeOf<guatiao_value>());
  } else {
    node.ref.tag = tag.code;
    _copy(_armOf(node), arm, sizeOf<guatiao_map>());
  }
  return node;
}

String _tagName(int code) => Tag.fromCode(code)?.name ?? 'value';

/// A fresh root, `calloc`ed, holding [value]'s tree given up by whoever
/// held it: a [Value] is moved (left absent), another [Ref] copied into
/// [alloc], plain data built there. [tag], when given, is what is asked.
Pointer<guatiao_value> _ownedTree(
  Object? value,
  Tag? tag,
  Pointer<guatiao_alloc> alloc,
) {
  if (value is Value) {
    if (tag != null && value.tag != tag) {
      throw ArgumentError('expected a ${tag.name}, got a ${value.tag.name}');
    }
    return moveOutOf(value);
  }
  final root = calloc<guatiao_value>();
  try {
    if (value is Ref) {
      checkStatus(
        native.core().bindings.guatiao_value_clone(
          alloc,
          value.requireNode(),
          root,
        ),
      );
    } else {
      writeDartInto(root, value, alloc);
    }
    if (tag != null && root.ref.tag != tag.code) {
      throw ArgumentError(
        'expected a ${tag.name}, got a ${_tagName(root.ref.tag)}',
      );
    }
  } catch (_) {
    _free(root);
    rethrow;
  }
  return root;
}

/// [node] in [alloc]: a tree built elsewhere is copied there, so nothing
/// the caller keeps needs another allocator.
Pointer<guatiao_value> _inAlloc(
  Pointer<guatiao_value> node,
  Pointer<guatiao_alloc> alloc,
) {
  final tag = node.ref.tag;
  if (tag < Tag.number.code || tag > Tag.map.code || _recorded(node) == alloc) {
    return node;
  }
  final copy = calloc<guatiao_value>();
  try {
    checkStatus(native.core().bindings.guatiao_value_clone(alloc, node, copy));
  } catch (_) {
    calloc.free(copy);
    _free(node);
    rethrow;
  }
  _free(node);
  return copy;
}

String _text(Pointer<Uint8> ptr, int len, String what) {
  if (len == 0 || ptr == nullptr) return '';
  try {
    return utf8.decode(ptr.asTypedList(len));
  } on FormatException {
    throw BadValue(Status.badValue.code, '$what is not UTF-8');
  }
}

/// `guatiao_provider_error.message`'s offset: after the status, aligned.
int get _messageOffset =>
    sizeOf<guatiao_provider_error>() - sizeOf<guatiao_string>();

/// Owned text, built through this binding's allocator, written over
/// [out], which owns nothing.
void _writeString(Pointer<guatiao_string> out, String text) {
  final size = sizeOf<guatiao_string>();
  out.cast<Uint8>().asTypedList(size).fillRange(0, size, 0);
  if (text.isEmpty) return;
  final node = calloc<guatiao_value>();
  try {
    using((arena) {
      checkStatus(
        native.core().bindings.guatiao_value_string(
          _binding,
          strOf(arena, text).ref,
          node,
        ),
      );
    });
    _copy(out, _armOf(node), size);
  } finally {
    calloc.free(node);
  }
}

/// Fills [err] with [status] and owned [message] for the caller to read
/// and free.
void writeProviderError(
  Pointer<guatiao_provider_error> err,
  int status,
  String message,
) {
  err.ref.status = status;
  _writeString(
    (err.cast<Uint8>() + _messageOffset).cast<guatiao_string>(),
    message,
  );
}

/// The status and words a refusal crosses with: a [GuatiaoException]'s
/// own, anything else `GUATIAO_ERR_INTERNAL` with its type and text.
(int, String) statusOf(Object error) {
  if (error is GuatiaoException) {
    final code = error.code == Status.ok.code
        ? Status.internalError.code
        : error.code;
    return (code, error.message);
  }
  return (Status.internalError.code, '${error.runtimeType}: $error');
}

// ---- one call's lifetime ------------------------------------------------

/// What one call keeps alive and what runs once it returns: the base of
/// [CallScope] and [ServeScope].
abstract class SlotCall {
  /// Memory for the call, released when it ends.
  final Arena arena = Arena();
  final List<void Function()> _after = <void Function()>[];
  bool _open = true;

  /// Throws once the call is over: a view lent for it is no longer valid.
  void checkOpen() {
    if (!_open) {
      throw StateError('this view was lent for one call, which has returned');
    }
  }

  /// Runs once the call is over, in reverse order.
  void after(void Function() fn) => _after.add(fn);

  void _end() {
    Object? failure;
    for (final fn in _after.reversed) {
      try {
        fn();
      } on Object catch (e) {
        failure ??= e;
      }
    }
    _after.clear();
    _open = false;
    arena.releaseAll();
    if (failure != null) throw failure;
  }
}

/// A `Ref` owner over a node lent to a call: valid while the call is,
/// writable, through [_alloc], when the tree is lent rather than borrowed.
final class _CallOwner implements ValueOwner {
  _CallOwner(this._node, this._call, this._alloc);

  final Pointer<guatiao_value> _node;
  final SlotCall _call;
  final Pointer<guatiao_alloc>? _alloc;

  @override
  Pointer<guatiao_value> get rootNode {
    _call.checkOpen();
    return _node;
  }

  @override
  Pointer<guatiao_alloc> get allocator {
    _call.checkOpen();
    final alloc = _alloc;
    if (alloc == null) throw StateError('a borrowed value is read-only');
    return alloc;
  }
}

/// A read-only view of [node], valid while [call] is.
Ref _borrowedView(Pointer<guatiao_value> node, SlotCall call) =>
    Ref.borrowedFrom(Ref(_CallOwner(node, call, null)), () => node);

// ---- the caller's side of one call ---------------------------------------

/// One call a remote makes: arguments built for it, out slots, the check,
/// and the answer. For generated code; [close] it in a `finally`.
final class CallScope extends SlotCall {
  final List<void Function()> _given = <void Function()>[];
  bool _called = false;

  /// Trees lent for the call, each with the allocator it recorded when it
  /// was lent: settled once the call returns.
  final List<(Pointer<guatiao_value>, Pointer<guatiao_alloc>)> _lentTrees =
      <(Pointer<guatiao_value>, Pointer<guatiao_alloc>)>[];

  /// The answer's out slot and what it holds once the call returns: a
  /// [Tag] for a tree (null for a value), `'text'` or `'object'`.
  Pointer<NativeType>? _out;
  Object? _outShape;

  /// Settles every lent tree; the first refusal's status, else OK.
  int _settleLent() {
    var status = Status.ok.code;
    for (final (node, home) in _lentTrees) {
      final moved = _settle(node, home);
      if (moved != Status.ok.code && status == Status.ok.code) status = moved;
    }
    _lentTrees.clear();
    return status;
  }

  /// Frees what the slot answered, which nobody will read.
  void _dropAnswer() {
    final out = _out;
    if (out == null) return;
    switch (_outShape) {
      case 'text':
        final node = calloc<guatiao_value>();
        node.ref.tag = Tag.string.code;
        _copy(_armOf(node), out, sizeOf<guatiao_string>());
        _free(node);
      case 'object':
        final o = out.cast<guatiao_object>().ref;
        _destroy(o.table, o.size, o.ctx);
      case final Tag? tag:
        _free(_nodeOver(out, tag));
    }
    _out = null;
  }

  guatiao_str str(String text) => strOf(arena, text).ref;

  guatiao_bytes bytes(List<int> data) {
    final out = arena<guatiao_bytes>();
    if (data.isNotEmpty) {
      final buffer = arena<Uint8>(data.length);
      buffer.asTypedList(data.length).setAll(0, data);
      out.ref.ptr = buffer;
    }
    out.ref.len = data.length;
    return out.ref;
  }

  /// [data] as the callee's out-buffer: copied in, and back once it
  /// returns.
  guatiao_bytes_mut bytesMut(Uint8List data) {
    final out = arena<guatiao_bytes_mut>();
    if (data.isNotEmpty) {
      final buffer = arena<Uint8>(data.length);
      buffer.asTypedList(data.length).setAll(0, data);
      out.ref.ptr = buffer;
      after(() => data.setAll(0, buffer.asTypedList(data.length)));
    }
    out.ref.len = data.length;
    return out.ref;
  }

  /// A value the callee borrows: a [Ref] as it is, plain data built for
  /// the call, `null` as a null pointer where the slot allows one.
  Pointer<guatiao_value> borrowValue(Object? value) =>
      value == null ? nullptr : _borrowed(value, null);

  /// A map the callee borrows: a [Ref] onto one, or a `Map` built for the
  /// call.
  Pointer<guatiao_map> borrowMap(Object value) =>
      _armOf(_borrowed(value, Tag.map)).cast();

  Pointer<guatiao_value> _borrowed(Object value, Tag? tag) {
    final Ref ref;
    if (value is Ref) {
      ref = value;
    } else {
      final built = Value.fromDart(value);
      after(built.close);
      ref = built;
    }
    final node = ref.requireNode();
    if (tag != null && node.ref.tag != tag.code) {
      throw ArgumentError('expected a ${tag.name}, got a ${ref.tag.name}');
    }
    return node;
  }

  /// A tree lent for the call and changed where it stands: a [Ref] onto a
  /// tree this side owns.
  Pointer<guatiao_value> lendValue(Ref ref) => _lent(ref, null);

  Pointer<guatiao_map> lendMap(Ref ref) => _armOf(_lent(ref, Tag.map)).cast();

  Pointer<guatiao_list> lendList(Ref ref) =>
      _armOf(_lent(ref, Tag.list)).cast();

  Pointer<guatiao_value> _lent(Ref ref, Tag? tag) {
    // Throws for a view this binding only borrows: what the callee
    // changes would have nowhere to go.
    ref.owner.allocator;
    final node = ref.requireNode();
    if (tag != null && node.ref.tag != tag.code) {
      throw ArgumentError('expected a ${tag.name}, got a ${ref.tag.name}');
    }
    // What the callee grew it with may be in its library's allocator:
    // moved into the one the tree records now, which a callee writing a
    // fresh tree over it would change.
    _lentTrees.add((node, _recorded(node)));
    return node;
  }

  /// A tree handed over with the call: a [Value] moved (left absent), a
  /// [Ref] copied, plain data built. Freed here if the call never happens.
  Pointer<guatiao_value> takeValue(Object? value) => _taken(value, null);

  Pointer<guatiao_map> takeMap(Object? value) =>
      _armOf(_taken(value, Tag.map)).cast();

  Pointer<guatiao_list> takeList(Object? value) =>
      _armOf(_taken(value, Tag.list)).cast();

  Pointer<guatiao_value> _taken(Object? value, Tag? tag) {
    final node = _ownedTree(value, tag, _binding);
    _given.add(() => native.core().bindings.guatiao_value_free(node));
    after(() => calloc.free(node));
    return node;
  }

  /// An object of [kind] lent for the call: a Dart object implementing
  /// it, served until the call returns, or a remote of it.
  Pointer<guatiao_object> lendObject(Kind kind, Object value) {
    final out = arena<guatiao_object>();
    if (value is KindRemote) {
      _sameKind(kind, value);
      value._lendInto(out);
    } else {
      kind._serve(value, out, scope: this);
    }
    return out;
  }

  /// An object of [kind] handed over: a Dart object implementing it,
  /// served until its new holder destroys it, or a handle given up.
  guatiao_object takeObject(Kind kind, Object value) {
    final out = arena<guatiao_object>();
    if (value is KindRemote) {
      _sameKind(kind, value);
      value._giveInto(out);
    } else {
      kind._serve(value, out);
      final ctx = out.ref.ctx.address;
      _given.add(() => release(ctx));
    }
    return out.ref;
  }

  void _sameKind(Kind kind, KindRemote value) {
    if (value.kind.name != kind.name ||
        value.kind.floorHash != kind.floorHash) {
      throw ArgumentError(
        'an object of ${value.kind.name} where ${kind.name} was asked for',
      );
    }
  }

  /// An answer's out slot holding a carrier: it owns nothing and names
  /// the allocator to build in, this binding's.
  Pointer<guatiao_map> mapOut() {
    final out = arena<guatiao_map>();
    out.ref.alloc = _binding;
    _out = out;
    _outShape = Tag.map;
    return out;
  }

  Pointer<guatiao_list> listOut() {
    final out = arena<guatiao_list>();
    out.ref.alloc = _binding;
    _out = out;
    _outShape = Tag.list;
    return out;
  }

  Pointer<guatiao_value> valueOut() {
    final out = arena<guatiao_value>();
    _armOf(out).cast<guatiao_map>().ref.alloc = _binding;
    _out = out;
    _outShape = null;
    return out;
  }

  Pointer<guatiao_string> textOut() {
    final out = arena<guatiao_string>();
    _out = out;
    _outShape = 'text';
    return out;
  }

  Pointer<guatiao_object> objectOut() {
    final out = arena<guatiao_object>();
    _out = out;
    _outShape = 'object';
    return out;
  }

  Pointer<guatiao_provider_error> errorOut() => arena<guatiao_provider_error>();

  /// The call happened: what was handed over is the callee's, and every
  /// lent tree is settled. Throws the refusal [status] names, with the
  /// provider's words from [err], or, for a [slot] that cannot fail,
  /// saying so; else a refused settle, the answer freed unread.
  void check(int status, Pointer<guatiao_provider_error>? err, String slot) {
    _called = true;
    _given.clear();
    final settled = _settleLent();
    if (status == Status.ok.code) {
      if (settled == Status.ok.code) return;
      _dropAnswer();
      throw statusException(
        settled,
        "moving what $slot put into a lent tree into that tree's allocator",
      );
    }
    if (err != null) checkProviderStatus(native.core().bindings, status, err);
    throw statusException(
      status,
      '$slot failed, which its kind says it cannot',
    );
  }

  Value mapAnswer(Pointer<guatiao_map> out) =>
      Value.adopt(_homed(_nodeOver(out, Tag.map)));

  Value listAnswer(Pointer<guatiao_list> out) =>
      Value.adopt(_homed(_nodeOver(out, Tag.list)));

  Value valueAnswer(Pointer<guatiao_value> out) =>
      Value.adopt(_homed(_nodeOver(out, null)));

  /// The text answered; the owned string is freed whatever it holds.
  String textAnswer(Pointer<guatiao_string> out) {
    final node = calloc<guatiao_value>();
    node.ref.tag = Tag.string.code;
    _copy(_armOf(node), out, sizeOf<guatiao_string>());
    try {
      return _text(out.ref.ptr, out.ref.len, 'the answer');
    } finally {
      _free(node);
    }
  }

  R objectAnswer<R extends KindRemote>(
    Kind<R> kind,
    Pointer<guatiao_object> out,
  ) => kind.handle(out.ref.table, out.ref.size, out.ref.ctx);

  /// Ends the call: copies out-buffers back, frees what it built, and
  /// frees what was to be handed over if the call never happened.
  void close() {
    if (!_called) {
      for (final undo in _given.reversed) {
        undo();
      }
      _given.clear();
    }
    _settleLent();
    _end();
  }
}

// ---- the callee's side of one call ---------------------------------------

/// One call into a served object: arguments read for the method, the
/// answer written, a refusal reported. For generated code.
final class ServeScope extends SlotCall {
  ServeScope(this.where, this._err);

  /// `kind.slot`, for a message.
  final String where;
  final Pointer<guatiao_provider_error>? _err;
  final List<Object> _received = <Object>[];
  bool _ran = false;
  int _status = Status.ok.code;

  /// The object [ctx] names: [Gone] once it was released. Called once
  /// every handed-over argument is taken, it first raises a refusal met
  /// while taking one.
  T held<T>(Pointer<Void> ctx) {
    final refused = _refused;
    if (refused != null) throw refused;
    if (ctx == nullptr) {
      throw NullArgument(Status.nullArgument.code, 'the call names no object');
    }
    final found = _live[ctx.address];
    if (found == null) {
      throw Gone(Status.gone.code, 'the object this call names is gone');
    }
    return found as T;
  }

  /// The method is about to run: what was handed over is its own.
  void running() => _ran = true;

  String str(guatiao_str s) => _text(s.ptr, s.len, 'the text argument');

  Uint8List bytes(guatiao_bytes b) => b.len == 0 || b.ptr == nullptr
      ? Uint8List(0)
      : Uint8List.fromList(b.ptr.asTypedList(b.len));

  /// A copy the method writes into, copied back when it returns.
  Uint8List bytesMut(guatiao_bytes_mut b) {
    if (b.len == 0 || b.ptr == nullptr) return Uint8List(0);
    final copy = Uint8List.fromList(b.ptr.asTypedList(b.len));
    final ptr = b.ptr;
    final len = b.len;
    after(() => ptr.asTypedList(len).setAll(0, copy));
    return copy;
  }

  Pointer<T> _nonNull<T extends NativeType>(Pointer<T> p, String what) {
    if (p == nullptr) {
      throw NullArgument(
        Status.nullArgument.code,
        'the $what argument is null',
      );
    }
    return p;
  }

  /// A value the caller lends read-only, `null` where the slot allows it.
  Ref? borrowedValue(Pointer<guatiao_value> p, {bool nullable = false}) {
    if (p == nullptr && nullable) return null;
    return _borrowedView(_nonNull(p, 'value'), this);
  }

  /// A map the caller lends read-only, through a node of this call's own.
  MapRef borrowedMap(Pointer<guatiao_map> p) =>
      _borrowedView(_nodeIn(_nonNull(p, 'map'), Tag.map), this).asMap();

  /// A tree handed over, which the method owns and may keep: moved out of
  /// the caller's allocators, which may go with the caller's library.
  Value takenValue(Pointer<guatiao_value> p) => _taking(
    () =>
        Value.adopt(_homed(_nodeOver(_nonNull(p, 'value'), null), _handedOver)),
  );

  Value takenMap(Pointer<guatiao_map> p) => _taking(
    () => Value.adopt(
      _homed(_nodeOver(_nonNull(p, 'map'), Tag.map), _handedOver),
    ),
  );

  Value takenList(Pointer<guatiao_list> p) => _taking(
    () => Value.adopt(
      _homed(_nodeOver(_nonNull(p, 'list'), Tag.list), _handedOver),
    ),
  );

  static const _handedOver = 'a tree handed over';

  /// The first refusal met while taking a handed-over tree, kept until
  /// [held] so the trees handed over after it are still taken, and freed.
  Object? _refused;

  Value _taking(Value Function() take) {
    try {
      return _keep(take());
    } on Object catch (e) {
      _refused ??= e;
      return _keep(Value.absent());
    }
  }

  Value _keep(Value v) {
    _received.add(v);
    return v;
  }

  /// A tree the caller lends for the call: changed where it stands, built
  /// on through the allocator it records.
  Ref lentValue(Pointer<guatiao_value> p) {
    final node = _nonNull(p, 'value');
    return Ref(_CallOwner(node, this, _recorded(node)));
  }

  MapRef lentMap(Pointer<guatiao_map> p) =>
      _lentView(_nonNull(p, 'map'), Tag.map).asMap();

  ListRef lentList(Pointer<guatiao_list> p) =>
      _lentView(_nonNull(p, 'list'), Tag.list).asList();

  /// A node of this call's own over the struct at [arm].
  Pointer<guatiao_value> _nodeIn(Pointer<NativeType> arm, Tag tag) {
    final node = arena<guatiao_value>();
    node.ref.tag = tag.code;
    _copy(_armOf(node), arm, sizeOf<guatiao_map>());
    return node;
  }

  /// What the method changes lands in a node of this call's own, written
  /// back over the caller's struct once it returns.
  Ref _lentView(Pointer<NativeType> arm, Tag tag) {
    final node = _nodeIn(arm, tag);
    after(() => _copy(arm, _armOf(node), sizeOf<guatiao_map>()));
    return Ref(_CallOwner(node, this, _recorded(node)));
  }

  /// An object handed over: the method owns it from now on. One whose
  /// table is not the kind's is refused, at [held], and left alone: its
  /// slot after the header need not be a `destroy`.
  R takenObject<R extends KindRemote>(Kind<R> kind, guatiao_object o) {
    try {
      final handle = kind.handle(o.table, o.size, o.ctx);
      _received.add(handle);
      return handle;
    } on Object catch (e) {
      _refused ??= e;
      return kind.remote(kind._standIn, kind.tableSize, nullptr);
    }
  }

  /// An object lent for the call.
  R lentObject<R extends KindRemote>(Kind<R> kind, Pointer<guatiao_object> p) {
    final o = _nonNull(p, 'object').ref;
    return kind.remote(o.table, o.size, o.ctx, scope: this);
  }

  /// The answer, over the carrier in [out]: built in the allocator it
  /// names, or copied there.
  void mapAnswer(Pointer<guatiao_map> out, Object? result) =>
      _treeAnswer(out, result, Tag.map);

  void listAnswer(Pointer<guatiao_list> out, Object? result) =>
      _treeAnswer(out, result, Tag.list);

  void valueAnswer(Pointer<guatiao_value> out, Object? result) {
    final named = _armOf(out).cast<guatiao_map>().ref.alloc;
    final alloc = _complete(named) ? named : _binding;
    final node = _inAlloc(_ownedTree(result, null, alloc), alloc);
    _copy(out, node, sizeOf<guatiao_value>());
    calloc.free(node);
  }

  void _treeAnswer(Pointer<NativeType> out, Object? result, Tag tag) {
    final named = out.cast<guatiao_map>().ref.alloc;
    final alloc = _complete(named) ? named : _binding;
    final node = _inAlloc(_ownedTree(result, tag, alloc), alloc);
    _copy(out, _armOf(node), sizeOf<guatiao_map>());
    calloc.free(node);
  }

  void textAnswer(Pointer<guatiao_string> out, String text) =>
      _writeString(out, text);

  /// The answered object: a Dart object implementing [kind], served until
  /// the caller destroys it, or a handle given up.
  void objectAnswer(Kind kind, Pointer<guatiao_object> out, Object result) {
    if (result is KindRemote) {
      result._giveInto(out);
    } else {
      kind._serve(result, out);
    }
  }

  /// The method threw: its status and words cross, or, for a slot that
  /// cannot fail, its status alone, the error printed.
  void fail(Object error) {
    final (status, message) = statusOf(error);
    _status = status;
    final err = _err;
    if (err == null) {
      stderr.writeln('guatiao: $where cannot fail, and threw $message');
    } else {
      writeProviderError(err, status, message);
    }
  }

  /// Ends the call and answers its status: what was handed over to a
  /// method that never ran is released, lent trees are written back.
  int close() {
    final refused = _refused;
    if (refused != null && _status == Status.ok.code) fail(refused);
    if (!_ran) {
      for (final v in _received) {
        if (v is Value) v.close();
        if (v is KindRemote) v.close();
      }
    }
    try {
      _end();
    } on Object catch (e) {
      if (_status == Status.ok.code) fail(e);
    }
    return _status;
  }
}
