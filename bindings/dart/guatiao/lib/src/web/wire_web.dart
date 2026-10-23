// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// The frame builders and the receiver, over the loaded WebAssembly
/// module.
library;

import 'dart:typed_data';

import '../status.dart';
import '../wire.dart';
import 'module.dart';
import 'value_web.dart';

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

// `guatiao_wire_event` on wasm32: `channel` (u64) at 0, `kind` at 8,
// `refusal` at 12, `at` (size_t) at 16, then three values; aligned to 8.
const int _valueAt = 16 + wordSize;
const int _eventSize = (_valueAt + 3 * valueSize + 7) & ~7;

Uint8List _frame(int Function(WasmModule, Address alloc, Address out) build) {
  final m = WasmModule.current;
  final out = m.alloc(valueSize);
  try {
    checkStatus(build(m, m.defaultAlloc, out));
    return Ref.borrowed(out).toDart() as Uint8List;
  } finally {
    freeRoot(out);
  }
}

T _withNode<T>(Object? value, T Function(Address) use) {
  if (value is Ref) return use(value.requireNode());
  final temporary = Value.fromDart(value);
  try {
    return use(temporary.requireNode());
  } finally {
    temporary.close();
  }
}

/// The frame that announces [schema] on [channel]. Under dart2js a
/// channel past 2^53 is not exact.
Uint8List announce(int channel, Object? schema) => _withNode(
  schema,
  (node) => _frame(
    (m, alloc, out) =>
        m.call('guatiao_wire_announce', [alloc, I64(channel), node, out]),
  ),
);

/// The frame that carries [value] on [channel].
Uint8List send(int channel, Object? value) => _withNode(
  value,
  (node) => _frame(
    (m, alloc, out) =>
        m.call('guatiao_wire_send', [alloc, I64(channel), node, out]),
  ),
);

/// The frame that ends [channel].
Uint8List close(int channel) => _frame(
  (m, alloc, out) => m.call('guatiao_wire_close', [alloc, I64(channel), out]),
);

final Finalizer<(WasmModule, Address)> _backstop =
    Finalizer<(WasmModule, Address)>((held) {
      final (module, handle) = held;
      if (identical(WasmModule.loaded, module)) {
        module.call('guatiao_wire_receiver_free', [handle]);
      }
    });

/// Keeps each channel's schema and checks every value against it.
/// [close] releases it; a finalizer is attached as a backstop.
class Receiver {
  Receiver._(this._module, this._handle) {
    _backstop.attach(this, (_module, _handle), detach: this);
  }

  factory Receiver() {
    final m = WasmModule.current;
    final handle = m.callAddress('guatiao_wire_receiver_new', [m.defaultAlloc]);
    if (handle.isNull) throw AllocFailed(Status.allocFailed.code);
    return Receiver._(m, handle);
  }

  final WasmModule _module;
  final Address _handle;
  bool _closed = false;

  /// Reads one frame: an [Event], or the refusal thrown as a [Refused].
  Event accept(List<int> frame) {
    if (_closed) throw StateError('this receiver is closed');
    if (!identical(_module, WasmModule.loaded)) {
      throw StateError(
        'this receiver lives in a module that is no longer loaded',
      );
    }
    final m = _module;
    return withScratch((s) {
      final view = s.view(frame);
      final event = s.alloc(_eventSize);
      final status = m.call('guatiao_wire_receiver_accept', [
        _handle,
        view,
        event,
      ]);
      final value = event + _valueAt;
      final message = value + valueSize;
      final detail = message + valueSize;
      final words = Ref.borrowed(message).stringOr('');
      final found = Ref.borrowed(detail).toDart(numbers: Numbers.text);
      Value? taken;
      if (status == Status.ok.code && m.u32(value) != Tag.absent.code) {
        final root = m.alloc(valueSize);
        m.setBytes(root, m.bytes(value, valueSize));
        m.setU32(value, Tag.absent.code);
        taken = Value.adopt(root, alloc: m.defaultAlloc);
      }
      m.call('guatiao_value_free', [value]);
      m.call('guatiao_value_free', [message]);
      m.call('guatiao_value_free', [detail]);
      final channel = m.u64(event);
      final kind = m.u32(event + 8);
      if (status == Status.ok.code) {
        return Event(Frame.values[kind], channel, taken);
      }
      throw refusalFor(
        status,
        m.u32(event + 12),
        words,
        channel,
        kind,
        m.u32(event + 16),
        found,
      );
    });
  }

  /// Frees the receiver and the schemas it keeps. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _backstop.detach(this);
    if (identical(_module, WasmModule.loaded)) {
      _module.call('guatiao_wire_receiver_free', [_handle]);
    }
  }
}
