// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import '../status.dart';

/// A kind as a peer's catalog describes it: each slot's arguments and
/// answer, which type a call's bytes.
final class RemoteKind {
  RemoteKind._(this.name, this.object, this.slots, this.doc);

  /// Reads one kind's description (the format of "A kind, described" in
  /// guatiao's encodings), as a catalog's `kinds` holds it. Throws
  /// [BadValue] for one that is not a kind.
  factory RemoteKind.fromDescription(Object? description) {
    final d = _map(description, 'a kind description');
    final name = _text(d, 'name', 'a kind');
    final slots = <RemoteSlot>[];
    final listed = d['slots'];
    if (listed is! List) throw _bad('kind `$name`: `slots` is not a list');
    for (var i = 0; i < listed.length; i++) {
      slots.add(RemoteSlot._of(listed[i], i, name));
    }
    final doc = d['doc'];
    return RemoteKind._(
      name,
      d['object'] == true,
      List.unmodifiable(slots),
      doc is String ? doc : null,
    );
  }

  /// What a provider lists and a call is typed by.
  final String name;

  /// An object kind: its values are objects a call hands or lends.
  final bool object;

  /// Every slot, in table order.
  final List<RemoteSlot> slots;

  final String? doc;

  /// The slot called [name]; throws [NotFound] when the kind has none.
  RemoteSlot slot(String name) {
    for (final s in slots) {
      if (s.name == name) return s;
    }
    throw NotFound(
      Status.notFound.code,
      'the kind `${this.name}` has no slot `$name`',
    );
  }

  @override
  String toString() => 'RemoteKind($name)';
}

/// One slot of a [RemoteKind].
final class RemoteSlot {
  RemoteSlot._(
    this.name,
    this.index,
    this.required,
    this.fallible,
    this.args,
    this.answer,
  );

  factory RemoteSlot._of(Object? description, int index, String kind) {
    final at = 'kind `$kind`, slot $index';
    final d = _map(description, at);
    final name = _text(d, 'name', at);
    final listed = d['args'] ?? const <Object?>[];
    if (listed is! List) throw _bad('$at: `args` is not a list');
    final args = [
      for (var i = 0; i < listed.length; i++)
        RemoteArg._of(listed[i], '$at, argument $i'),
    ];
    final answer = d['answer'];
    return RemoteSlot._(
      name,
      index,
      d['required'] == true,
      d['fallible'] == true,
      List.unmodifiable(args),
      answer == null ? null : RemoteAnswer._of(answer, '$at, answer'),
    );
  }

  /// Its field in the table.
  final String name;

  /// Its index in the kind: what a CALL names.
  final int index;

  /// A slot every table fills, not null.
  final bool required;

  /// It can refuse.
  final bool fallible;

  /// Its arguments, in order.
  final List<RemoteArg> args;

  /// Its answer; `null` when it answers nothing.
  final RemoteAnswer? answer;

  @override
  String toString() => 'RemoteSlot($name)';
}

/// An argument of a [RemoteSlot].
final class RemoteArg {
  RemoteArg._(this.name, this.type, this.kind, this.pass, this.nullable);

  factory RemoteArg._of(Object? description, String at) {
    final d = _map(description, at);
    final pass = d['pass'];
    return RemoteArg._(
      d['name'] is String ? d['name'] as String : '',
      _text(d, 'type', at),
      d['kind'] is String ? d['kind'] as String : null,
      pass is String ? pass : 'borrowed',
      d['nullable'] == true,
    );
  }

  final String name;

  /// `bool`, `i8` ... `u64`, `isize`, `usize`, `f32`, `f64`, `str`,
  /// `bytes`, `bytes_mut`, `value`, `map`, `list` or `object`.
  final String type;

  /// The object kind, for an `object`.
  final String? kind;

  /// `borrowed`, `taken` or `lent`.
  final String pass;

  /// A borrowed `value` that may be absent.
  final bool nullable;
}

/// What a [RemoteSlot] answers.
final class RemoteAnswer {
  RemoteAnswer._(this.type, this.kind);

  factory RemoteAnswer._of(Object? description, String at) {
    final d = _map(description, at);
    return RemoteAnswer._(
      _text(d, 'type', at),
      d['kind'] is String ? d['kind'] as String : null,
    );
  }

  /// A scalar's name, `text`, `value`, `map`, `list` or `object`.
  final String type;

  /// The object kind, for an `object`.
  final String? kind;
}

BadValue _bad(String why) => BadValue(Status.badValue.code, why);

Map<Object?, Object?> _map(Object? v, String at) {
  if (v is Map) return v;
  throw _bad('$at is not a map');
}

String _text(Map<Object?, Object?> d, String key, String at) {
  final v = d[key];
  if (v is String) return v;
  throw _bad('$at: `$key` is not text');
}
