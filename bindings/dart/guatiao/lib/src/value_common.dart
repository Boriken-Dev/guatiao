// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:collection';
import 'dart:typed_data';

/// The bytes [Ref.reveal] lends its callback: read where they live, in the
/// tree's own memory, and only while that callback runs.
///
/// Every access after it returns throws [StateError]: the memory may have
/// been freed or moved since. Copying out (`List.of`, `utf8.decode`) makes
/// a copy Dart keeps and nothing can wipe.
final class RevealedBytes extends ListBase<int> {
  RevealedBytes._(this._bytes);

  final Uint8List _bytes;
  bool _open = true;

  void _check() {
    if (!_open) {
      throw StateError(
        'revealed bytes are read only during the call that lent them',
      );
    }
  }

  @override
  int get length {
    _check();
    return _bytes.length;
  }

  @override
  int operator [](int index) {
    _check();
    return _bytes[index];
  }

  @override
  void operator []=(int index, int value) =>
      throw UnsupportedError('revealed bytes cannot be written');

  @override
  set length(int length) =>
      throw UnsupportedError('revealed bytes cannot change length');
}

/// How many reveals of each owner's tree are running.
final Expando<int> _revealing = Expando<int>('reveals running');

/// Runs [use] over [view], the bytes of a node of [owner]'s tree, refusing
/// every change to and free of that tree until it returns.
R revealBytes<R>(Object owner, Uint8List view, R Function(RevealedBytes) use) {
  final bytes = RevealedBytes._(view);
  _revealing[owner] = (_revealing[owner] ?? 0) + 1;
  try {
    return use(bytes);
  } finally {
    bytes._open = false;
    final left = _revealing[owner]! - 1;
    _revealing[owner] = left == 0 ? null : left;
  }
}

/// Throws while a reveal of [owner]'s tree runs: changing or freeing the
/// tree then would move or free the bytes being read.
void checkUnrevealed(Object owner) {
  if ((_revealing[owner] ?? 0) > 0) {
    throw StateError(
      'a reveal of this value is running: it cannot be changed or freed '
      'until the call that lent the bytes returns',
    );
  }
}

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

/// Whether `int` is a JavaScript number (dart2js).
const bool _jsNumbers = identical(1, 1.0);

/// Whether [whole] is an `int` here: 64 bits on the VM and dart2wasm, 53
/// under dart2js, where `isValidInt` also takes any integer a double holds
/// exactly (2^60 among them), past what one holds safely.
bool _fitsInt(BigInt whole) =>
    _jsNumbers ? whole.bitLength <= 53 : whole.isValidInt;

/// A number's exact text as Dart, by [numbers]: an integer is an `int`
/// only where the platform's `int` holds it exactly (64 bits on the VM
/// and dart2wasm, 53 under dart2js), a `BigInt` otherwise.
Object numberFromText(String text, Numbers numbers) {
  switch (numbers) {
    case Numbers.text:
      return text;
    case Numbers.bigInt:
      return BigInt.tryParse(text) ?? double.parse(text);
    case Numbers.auto:
      if (text.contains('.') || text.contains('e') || text.contains('E')) {
        return double.parse(text);
      }
      final whole = BigInt.parse(text);
      return _fitsInt(whole) ? whole.toInt() : whole;
  }
}

/// The text an integral number has, or `null` when it has a fraction or
/// an exponent, is not a number, or is past what an `int` holds exactly.
int? intFromText(String text) {
  if (text.contains('.') || text.contains('e') || text.contains('E')) {
    return null;
  }
  final whole = BigInt.tryParse(text);
  return whole != null && _fitsInt(whole) ? whole.toInt() : null;
}
