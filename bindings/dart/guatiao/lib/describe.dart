// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A kind born in Dart: declared as abstract interface classes spelled
/// with the marks here, and read into the description document every
/// language generates from.
///
/// A declaration says what a description says and nothing else; nothing
/// implements it. `dart run guatiao:kind describe FILE.dart -o
/// DOCUMENT.json` reads it ([describeDeclarations]), and `dart run
/// guatiao:kind DOCUMENT.json -o FILE.dart` writes the kind's classes from
/// the document, as from any other:
///
/// ```dart
/// import 'package:guatiao/describe.dart';
/// import 'package:guatiao/guatiao.dart' show MapRef;
///
/// /// What a tally tells whoever is watching it.
/// abstract interface class Watcher implements ObjectKind {
///   /// `name` now stands at `total`.
///   void changed(String name, I64 total);
/// }
///
/// /// Counts by name.
/// abstract interface class Tally {
///   /// Adds `by` to the count under `name`.
///   @fallible
///   I64 add(String name, U32 by);
///
///   /// Appended: tells `watcher` each count, for this call alone.
///   @appended
///   @fallible
///   void report(Lent<Watcher> watcher);
///
///   /// Appended: adds the counts handed over.
///   @appended
///   @fallible
///   U32 load(Taken<MapRef> counts);
/// }
/// ```
///
/// Every `abstract interface class` of the file is a kind, named in
/// snake_case unless [KindName] says; one implementing [ObjectKind] is an
/// object kind. Its methods are the slots in table order, the required
/// ones first; [appended] marks a slot added after them and [fallible] one
/// that can refuse. `///` comments are the docs. The floor hash is the one
/// `#[guatiao::kind]` gives the same required slots, unless [FloorHash]
/// pins one.
///
/// | Dart | in the description |
/// | --- | --- |
/// | `String` | `str`; answered, `text` |
/// | `Uint8List`, [BytesMut] | `bytes`, `bytes_mut` |
/// | `bool`, `int`, `double`, [I8]..[U64], [Isize], [Usize], [F32], [F64] | the scalar (`int` is `i64`, `double` `f64`) |
/// | `Ref`, `Ref?`, `MapRef` | a `value`, a `value` that may be null, a `map`, borrowed |
/// | [Lent] of `Ref`, `MapRef`, `ListRef` or an object kind | lent for the call |
/// | [Taken] of the same | handed over |
/// | [ValueAnswer], [MapAnswer], [ListAnswer] | the tree answered |
/// | an object kind's class | the object answered |
library;

import 'dart:typed_data';

export 'src/kind_describer.dart' show describeDeclarations;

/// `i8`.
typedef I8 = int;

/// `i16`.
typedef I16 = int;

/// `i32`.
typedef I32 = int;

/// `i64`, which a plain `int` is too.
typedef I64 = int;

/// `isize`, C's `intptr_t`.
typedef Isize = int;

/// `u8`.
typedef U8 = int;

/// `u16`.
typedef U16 = int;

/// `u32`.
typedef U32 = int;

/// `u64`.
typedef U64 = int;

/// `usize`, C's `size_t`.
typedef Usize = int;

/// `f32`.
typedef F32 = double;

/// `f64`, which a plain `double` is too.
typedef F64 = double;

/// The caller's bytes, written in place (`bytes_mut`); a plain
/// `Uint8List` is read-only `bytes`.
typedef BytesMut = Uint8List;

/// An argument lent for the call: a tree written where it stands, or an
/// object of an object kind.
typedef Lent<T> = T;

/// An argument handed over: a tree, or an object, the callee then owns.
typedef Taken<T> = T;

/// A slot answering a tree of any shape (`value`).
typedef ValueAnswer = Object?;

/// A slot answering a map (`map`).
typedef MapAnswer = Object;

/// A slot answering a list (`list`).
typedef ListAnswer = Object;

/// What an object kind's class implements: its table has a `destroy`
/// slot, and one holder owns each object.
abstract interface class ObjectKind {}

/// The kind's name, when it is not the class's in snake_case.
final class KindName {
  const KindName(this.name);

  final String name;
}

/// The kind's floor hash, when it is not to be computed.
final class FloorHash {
  const FloorHash(this.value);

  final int value;
}

/// Marks a slot that can refuse (`fallible`).
const Object fallible = _Mark('fallible');

/// Marks an optional slot, after the kind's required ones: a provider may
/// leave it null.
const Object appended = _Mark('appended');

final class _Mark {
  const _Mark(this.name);

  final String name;
}
