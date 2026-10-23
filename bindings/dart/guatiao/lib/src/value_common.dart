// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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
      return whole.isValidInt ? whole.toInt() : whole;
  }
}

/// The text an integral number has, or `null` when it has a fraction or
/// an exponent, is not a number, or is past what an `int` holds exactly.
int? intFromText(String text) {
  if (text.contains('.') || text.contains('e') || text.contains('E')) {
    return null;
  }
  final whole = BigInt.tryParse(text);
  return whole != null && whole.isValidInt ? whole.toInt() : null;
}
