// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'bindings.g.dart';

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

GuatiaoException _exceptionFor(int code, String message) {
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
  throw _exceptionFor(code, message);
}

/// Throws unless `code` is `GUATIAO_OK`, taking the provider's own words
/// from `err` first and freeing the owned string it built.
///
/// `err` is read and released whatever the status is, because a provider
/// may leave a message behind on a call that then reported success.
void checkProviderStatus(
  GuatiaoBindings bindings,
  int code,
  Pointer<guatiao_provider_error> err,
) {
  var message = '';
  final owned = err.ref.message;
  if (owned.len != 0 && owned.ptr != nullptr) {
    message = utf8.decode(
      Uint8List.fromList(owned.ptr.asTypedList(owned.len)),
      allowMalformed: true,
    );
  }
  if (owned.cap != 0) {
    // The message is an owned `guatiao_string`; freeing it means handing
    // the value model a string-tagged node that borrows those 32 bytes.
    final wrapper = calloc<guatiao_value>();
    try {
      wrapper.ref.tag = 4; // GUATIAO_STRING
      wrapper.ref.payload.text
        ..ptr = owned.ptr
        ..len = owned.len
        ..cap = owned.cap
        ..alloc = owned.alloc;
      bindings.guatiao_value_free(wrapper);
    } finally {
      calloc.free(wrapper);
    }
    err.ref.message
      ..ptr = nullptr
      ..len = 0
      ..cap = 0;
  }
  checkStatus(code, message);
}
