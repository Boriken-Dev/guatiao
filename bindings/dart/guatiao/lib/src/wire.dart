// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';
import 'errors.dart';
import 'library.dart' as native;
import 'value.dart';

/// A frame's kind byte: `GUATIAO_FRAME_*`.
enum Frame {
  schema(0),
  value(1),
  close(2);

  const Frame(this.code);

  /// The byte the frame starts with.
  final int code;
}

/// What a frame said.
///
/// [value] is the schema now the channel's, or the value its schema
/// accepted, as a [Value] the caller closes; `null` for a channel's end.
class Event {
  Event(this.kind, this.channel, this.value);

  final Frame kind;

  /// The channel, its 64 bits as Dart's `int` holds them.
  final int channel;
  final Value? value;

  @override
  String toString() => 'Event(${kind.name}, $channel)';
}

/// A frame a [Receiver] refused: also the status's exception.
///
/// [channel] is the channel the frame named (0 when it ended before one),
/// [kind] its kind byte. Nothing the frame carried is quoted.
abstract interface class Refused implements GuatiaoException {
  int get channel;
  int get kind;
}

/// Bytes that are not a frame; [at] is the byte of the frame it went
/// wrong at.
class Malformed extends BadValue implements Refused {
  Malformed(super.code, super.message, this.channel, this.kind, this.at);

  @override
  final int channel;
  @override
  final int kind;
  final int at;
}

/// A value on a channel no schema was announced on.
class NoSchema extends NotFound implements Refused {
  NoSchema(super.code, super.message, this.channel, this.kind);

  @override
  final int channel;
  @override
  final int kind;
}

/// A schema frame whose value is not a schema.
class NotASchema extends WrongKind implements Refused {
  NotASchema(super.code, super.message, this.channel, this.kind);

  @override
  final int channel;
  @override
  final int kind;
}

/// A value its channel's schema refuses: [key] is the field, and
/// [expected] what it accepts or [known] the fields there are.
class Invalid extends BadValue implements Refused {
  Invalid(
    super.code,
    super.message,
    this.channel,
    this.kind,
    this.key,
    this.expected,
    this.known,
  );

  @override
  final int channel;
  @override
  final int kind;
  final String key;
  final String? expected;
  final List<String>? known;
}

/// Builds a frame through [build], answering its bytes.
Uint8List _frame(
    int Function(Pointer<guatiao_alloc>, Pointer<guatiao_value>) build) {
  final core = native.core();
  final out = calloc<guatiao_value>();
  try {
    checkStatus(build(core.alloc, out));
    return Ref.borrowed(out).toDart() as Uint8List;
  } finally {
    core.bindings.guatiao_value_free(out);
    calloc.free(out);
  }
}

/// Calls [use] with [value]'s node, converting plain Dart into a
/// temporary [Value] first.
T _withNode<T>(Object? value, T Function(Pointer<guatiao_value>) use) {
  if (value is Ref) return use(value.requireNode());
  final temporary = Value.fromDart(value);
  try {
    return use(temporary.requireNode());
  } finally {
    temporary.close();
  }
}

/// The frame that announces [schema] (a [Value], a [Ref] or what
/// [Value.fromDart] takes) on [channel]. A later one replaces it.
Uint8List announce(int channel, Object? schema) => _withNode(
      schema,
      (node) => _frame((alloc, out) => native
          .core()
          .bindings
          .guatiao_wire_announce(alloc, channel, node, out)),
    );

/// The frame that carries [value] on [channel].
Uint8List send(int channel, Object? value) => _withNode(
      value,
      (node) => _frame((alloc, out) =>
          native.core().bindings.guatiao_wire_send(alloc, channel, node, out)),
    );

/// The frame that ends [channel]: a receiver forgets its schema.
Uint8List close(int channel) => _frame((alloc, out) =>
    native.core().bindings.guatiao_wire_close(alloc, channel, out));

/// Where the event's three values sit: after `channel`, `kind`, `refusal`
/// and `at`, one after another.
int get _valueAt => 16 + sizeOf<Size>();

/// Keeps each channel's schema and checks every value against it.
///
/// [close] releases it; a finalizer is attached as a backstop. One
/// receiver serves one conversation, and a Dart isolate is one thread.
class Receiver implements Finalizable {
  Receiver._(this._handle)
      : _finalizer = NativeFinalizer(native.core().wireReceiverFreeFinalizer) {
    _finalizer.attach(this, _handle.cast<Void>(), detach: this);
  }

  factory Receiver() {
    final core = native.core();
    final handle = core.bindings.guatiao_wire_receiver_new(core.alloc);
    if (handle == nullptr) throw AllocFailed(Status.allocFailed.code);
    return Receiver._(handle);
  }

  final Pointer<guatiao_wire_receiver> _handle;
  final NativeFinalizer _finalizer;
  bool _closed = false;

  /// Reads one frame: an [Event], or the refusal thrown as a [Refused].
  Event accept(List<int> frame) {
    if (_closed) throw StateError('this receiver is closed');
    final core = native.core();
    final bindings = core.bindings;
    return using((arena) {
      final view = arena<guatiao_bytes>();
      if (frame.isEmpty) {
        view.ref
          ..ptr = nullptr
          ..len = 0;
      } else {
        final buffer = arena<Uint8>(frame.length);
        buffer.asTypedList(frame.length).setAll(0, frame);
        view.ref
          ..ptr = buffer
          ..len = frame.length;
      }
      final event = arena<guatiao_wire_event>();
      final status =
          bindings.guatiao_wire_receiver_accept(_handle, view.ref, event);
      final at = event.address + _valueAt;
      final value = Pointer<guatiao_value>.fromAddress(at);
      final message =
          Pointer<guatiao_value>.fromAddress(at + sizeOf<guatiao_value>());
      final detail =
          Pointer<guatiao_value>.fromAddress(at + 2 * sizeOf<guatiao_value>());
      final words = Ref.borrowed(message).stringOr('');
      final found = Ref.borrowed(detail).toDart(numbers: Numbers.text);
      Value? taken;
      if (status == Status.ok.code && value.ref.tag != Tag.absent.code) {
        final root = calloc<guatiao_value>();
        root.ref = value.ref;
        value.ref.tag = Tag.absent.code;
        taken = Value.adopt(root, alloc: core.alloc);
      }
      bindings.guatiao_value_free(value);
      bindings.guatiao_value_free(message);
      bindings.guatiao_value_free(detail);
      final channel = event.ref.channel;
      final kind = event.ref.kind;
      if (status == Status.ok.code) {
        return Event(Frame.values[kind], channel, taken);
      }
      switch (event.ref.refusal) {
        case 1:
          throw Malformed(status, words, channel, kind, event.ref.at);
        case 2:
          throw NoSchema(status, words, channel, kind);
        case 3:
          throw NotASchema(status, words, channel, kind);
        case 4:
          final map = found is Map ? found : const <String, Object?>{};
          final known = map['known'];
          throw Invalid(
            status,
            words,
            channel,
            kind,
            map['key'] as String? ?? '',
            map['expected'] as String?,
            known is List ? known.cast<String>() : null,
          );
      }
      checkStatus(status, words);
      throw StateError('a refused status throws');
    });
  }

  /// Frees the receiver and the schemas it keeps. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _finalizer.detach(this);
    native.core().bindings.guatiao_wire_receiver_free(_handle);
  }
}
