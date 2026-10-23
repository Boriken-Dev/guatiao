// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `guatiao_status`: what an entry point reports.
enum Status {
  ok(0),
  badValue(7),
  allocFailed(8),
  wrongKind(9),
  notFound(10),
  nullArgument(11),
  gone(12),
  unsupported(13),
  busy(14),
  internalError(100);

  const Status(this.code);

  /// The number the C ABI carries.
  final int code;

  /// The status `code` names, or `null` when this binding does not know it.
  static Status? fromCode(int code) {
    for (final status in Status.values) {
      if (status.code == code) return status;
    }
    return null;
  }
}

/// One failing `guatiao_status`.
///
/// [code] is the raw number; [status] is `null` for a code this binding
/// does not know. [message] carries a provider's own words when the call
/// produced any.
class GuatiaoException implements Exception {
  GuatiaoException(this.code, [this.message = '']);

  final int code;
  final String message;

  Status? get status => Status.fromCode(code);

  @override
  String toString() {
    final name = status?.name ?? 'status $code';
    return message.isEmpty
        ? 'GuatiaoException: $name'
        : 'GuatiaoException: $name: $message';
  }
}

class BadValue extends GuatiaoException {
  BadValue(super.code, [super.message]);
}

class AllocFailed extends GuatiaoException {
  AllocFailed(super.code, [super.message]);
}

class WrongKind extends GuatiaoException {
  WrongKind(super.code, [super.message]);
}

class NotFound extends GuatiaoException {
  NotFound(super.code, [super.message]);
}

class NullArgument extends GuatiaoException {
  NullArgument(super.code, [super.message]);
}

/// Not now: something the call needs is still in use.
class Busy extends GuatiaoException {
  Busy(super.code, [super.message]);
}

/// What was asked is not something the other side offers.
class Unsupported extends GuatiaoException {
  Unsupported(super.code, [super.message]);
}

class Gone extends GuatiaoException {
  Gone(super.code, [super.message]);
}

class InternalError extends GuatiaoException {
  InternalError(super.code, [super.message]);
}

/// The exception a failing [code] is, with [message]. Not part of the
/// package's API.
GuatiaoException statusException(int code, String message) {
  switch (Status.fromCode(code)) {
    case Status.badValue:
      return BadValue(code, message);
    case Status.allocFailed:
      return AllocFailed(code, message);
    case Status.wrongKind:
      return WrongKind(code, message);
    case Status.notFound:
      return NotFound(code, message);
    case Status.nullArgument:
      return NullArgument(code, message);
    case Status.gone:
      return Gone(code, message);
    case Status.unsupported:
      return Unsupported(code, message);
    case Status.busy:
      return Busy(code, message);
    case Status.internalError:
      return InternalError(code, message);
    case Status.ok:
    case null:
      return GuatiaoException(code, message);
  }
}

/// Throws unless `code` is `GUATIAO_OK`.
void checkStatus(int code, [String message = '']) {
  if (code == Status.ok.code) return;
  throw statusException(code, message);
}
