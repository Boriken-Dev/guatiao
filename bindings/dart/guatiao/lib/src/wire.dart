// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'status.dart';

export 'wire_native.dart'
    if (dart.library.js_interop) 'web/wire_web.dart'
    show Event, Receiver, announce, close, send;

/// A frame's kind byte: `GUATIAO_FRAME_*`.
enum Frame {
  schema(0),
  value(1),
  close(2);

  const Frame(this.code);

  /// The byte the frame starts with.
  final int code;
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

/// The exception a refused frame throws: [refusal] is the event's
/// `GUATIAO_WIRE_*` code, [words] its message, [detail] the validator's
/// map for an invalid value. Not part of the package's API.
GuatiaoException refusalFor(
  int status,
  int refusal,
  String words,
  int channel,
  int kind,
  int at,
  Object? detail,
) {
  switch (refusal) {
    case 1:
      return Malformed(status, words, channel, kind, at);
    case 2:
      return NoSchema(status, words, channel, kind);
    case 3:
      return NotASchema(status, words, channel, kind);
    case 4:
      final map = detail is Map ? detail : const <String, Object?>{};
      final known = map['known'];
      return Invalid(
        status,
        words,
        channel,
        kind,
        map['key'] as String? ?? '',
        map['expected'] as String?,
        known is List ? known.cast<String>() : null,
      );
  }
  return statusException(status, words);
}
