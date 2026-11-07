// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:guatiao/guatiao.dart' as guatiao;
import 'package:guatiao_intake/guatiao_intake.dart' as intake;

import 'field_schema.dart';
import 'paths.dart';

/// The hints a form gives one field: guatiao_intake's `widget`,
/// `placeholder`, `unit` and, for a field with members, its own `form`.
class FieldHints {
  const FieldHints(this.node);

  final Map<String, Object?> node;

  String? get widget => node['widget'] as String?;
  String? get placeholder => node['placeholder'] as String?;
  String? get unit => node['unit'] as String?;

  /// The form of the field's own members, when it has one.
  Map<String, Object?> get form =>
      (node['form'] as Map?)?.cast<String, Object?>() ?? const {};
}

/// A section as the form lays it out.
class FormGroup {
  const FormGroup({
    required this.id,
    required this.title,
    required this.help,
    required this.fields,
  });

  /// The section's id; `''` for the default section.
  final String id;
  final String title;
  final String help;

  /// The member keys in it, in display order.
  final List<String> fields;
}

/// A number kept as its exact text, written with `Value.number`.
class _NumberText {
  const _NumberText(this.text);
  final String text;
}

/// A text that does not read as its field's kind.
class _Untaken {
  const _Untaken(this.issue);
  final String issue;
}

/// What a person is filling in: a schema, the form it is shown with, and
/// the value that comes of it, held as a `guatiao.Value` so numbers keep
/// their text and bytes stay bytes.
///
/// Layout and visibility are guatiao_intake's (`layout`, `isVisible`).
/// Text that does not read as its field's kind (`12a` for an integer) is
/// kept as typed, left out of the value, and named in [issues].
///
/// It holds native or module memory: [close] it, or [dispose] it.
class FormModel extends ChangeNotifier {
  /// A model over [schema], shown with [form] (or, when it is `null`, the
  /// form guatiao_intake's `forSchema` implies), holding [value]: a plain
  /// Dart map or a `guatiao.Ref` onto one, copied either way.
  ///
  /// Throws [StateError] when the form does not fit the schema, and
  /// [ArgumentError] when [value] is not a map.
  FormModel({
    required Map<String, Object?> schema,
    Map<String, Object?>? form,
    Object? value,
  }) : schema = FieldSchema(schema),
       _scope = _Scope.of(schema, form) {
    try {
      final problem = intake.check(_scope.schema, _scope.form);
      if (problem != null) {
        throw StateError(
          'The form does not fit its schema: ${problem['message']}',
        );
      }
      // A schema with a secret in it keeps the whole value in the wiping
      // allocator, so what is freed is overwritten first.
      final initial = value ?? const <String, Object?>{};
      tree = _hasSecret(schema) || (value is guatiao.Ref && value.isSensitive)
          ? guatiao.Value.sensitive(initial)
          : value is guatiao.Ref
          ? value.clone()
          : guatiao.Value.fromDart(initial);
      if (tree.tag != guatiao.Tag.map) {
        tree.close();
        throw ArgumentError.value(value, 'value', 'not a map');
      }
    } catch (_) {
      _scope.close();
      _closed = true;
      rethrow;
    }
  }

  /// A model for a provider's configuration: [schema] is its config
  /// schema, [meta] its `meta` (either plain Dart, as a
  /// `Registry.catalog()` entry holds them, or a `guatiao.Ref`), shown
  /// with the form guatiao_intake's `providerForm` answers.
  factory FormModel.forProvider({
    required Object schema,
    Object? meta,
    Object? value,
  }) {
    final schemaJson = _plainMap(schema, 'schema');
    final found = intake.providerForm(schema, meta);
    Map<String, Object?> form = const {};
    if (found != null) {
      try {
        form = _plainMap(found, 'form');
      } finally {
        found.close();
      }
    }
    return FormModel(schema: schemaJson, form: form, value: value);
  }

  /// Whether any field the schema describes is `x-sensitive` or
  /// `writeOnly`.
  static bool _hasSecret(Object? node) {
    if (node is Map) {
      if (node['x-sensitive'] == true || node['writeOnly'] == true) return true;
      return node.values.any(_hasSecret);
    }
    if (node is List) return node.any(_hasSecret);
    return false;
  }

  static Map<String, Object?> _plainMap(Object source, String name) {
    final dart = source is guatiao.Ref ? source.toDart() : source;
    if (dart is! Map) throw ArgumentError.value(source, name, 'not a map');
    return dart.cast<String, Object?>();
  }

  /// The schema of the whole record.
  final FieldSchema schema;

  final _Scope _scope;
  final _scopes = <String, _Scope>{};

  /// The value being filled in. Read it freely; change it through this
  /// model, so the view hears of it.
  late final guatiao.Value tree;

  /// The form in use, as plain Dart.
  Map<String, Object?> get form => _scope.formJson;

  final _drafts = <String, String>{};
  final _issues = <String, String>{};
  final _ids = <String, List<int>>{};
  var _nextId = 0;
  bool _closed = false;

  /// What will not read as its kind, or breaks a bound, by path.
  Map<String, String> get issues => Map.unmodifiable(_issues);

  /// Lays out the members of what is at [path] (`''` for the record), in
  /// sections, in order. [at] is that field's schema, when the caller has
  /// it.
  List<FormGroup> layout([String path = '', FieldSchema? at]) {
    final scope = _scopeAt(path, at);
    if (scope == null) return const [];
    return [
      for (final group in intake.layout(scope.schema, scope.form))
        () {
          final section = (group['section'] as Map?)?.cast<String, Object?>();
          return FormGroup(
            id: section?['id'] as String? ?? '',
            title: section?['title'] as String? ?? '',
            help: section?['description'] as String? ?? '',
            fields: (group['fields'] as List).cast<String>(),
          );
        }(),
    ];
  }

  /// The schema of the field at [path].
  FieldSchema? field(String path) {
    final resolved = intake.resolve(_scope.schema, path);
    if (resolved == null) return null;
    return FieldSchema((resolved.toDart() as Map).cast<String, Object?>());
  }

  /// The form's hints for [key] (a member, or `owner.member` for a
  /// variant's field) of what is at [path].
  FieldHints hints(String path, String key, [FieldSchema? at]) {
    final fields = _scopeAt(path, at)?.formJson['fields'];
    final found = fields is Map ? fields[key] : null;
    return FieldHints(found is Map ? found.cast<String, Object?>() : const {});
  }

  /// Whether [key] of what is at [path] shows now, by what it holds.
  bool isVisible(String path, String key, [FieldSchema? at]) {
    final scope = _scopeAt(path, at);
    if (scope == null) return true;
    final held = path.isEmpty ? tree : _at(path);
    if (held != null && held.tag == guatiao.Tag.map) {
      return intake.isVisible(scope.schema, scope.form, key, held);
    }
    final empty = guatiao.Value.map();
    try {
      return intake.isVisible(scope.schema, scope.form, key, empty);
    } finally {
      empty.close();
    }
  }

  _Scope? _scopeAt(String path, [FieldSchema? at]) {
    if (path.isEmpty) return _scope;
    final known = _scopes[path];
    if (known != null) return known;
    final found = at ?? field(path);
    final members = found?.hasMembers == true ? found : found?.memberArm;
    if (members == null) return null;
    Map<String, Object?>? formJson;
    // A list's or a map's own form is its entries'.
    guatiao.Value? assigned;
    try {
      assigned = intake.formFor(
        _scope.form,
        _scope.schema,
        _ownerOfEntry(path),
      );
    } on guatiao.GuatiaoException {
      assigned = null;
    }
    if (assigned != null) {
      try {
        formJson = (assigned.toDart() as Map).cast<String, Object?>();
      } finally {
        assigned.close();
      }
    }
    return _scopes[path] = _Scope.of(members.node, formJson);
  }

  /// A borrowed read-only view of what is at [path], or `null`.
  guatiao.Ref? _at(String path) {
    if (_closed) throw StateError('the model is closed');
    return intake.at(tree, path);
  }

  /// What is at [path] as plain Dart; `guatiao.absent` when nothing is.
  Object? read(String path, {guatiao.Numbers numbers = guatiao.Numbers.auto}) {
    final at = _at(path);
    return at == null ? guatiao.absent : at.toDart(numbers: numbers);
  }

  /// Whether anything is held at [path], or typed there.
  bool holds(String path) => _drafts.containsKey(path) || _at(path) != null;

  /// What a text box shows for [path]: the text as typed when it did not
  /// read as its kind, else the text of what is held, a number as its
  /// exact text and anything with no one-line spelling as JSON.
  String? text(String path) {
    final draft = _drafts[path];
    if (draft != null) return draft;
    return _textOf(_at(path));
  }

  static String? _textOf(guatiao.Ref? at) {
    if (at == null) return null;
    switch (at.tag) {
      case guatiao.Tag.string:
        return at.toDart() as String;
      case guatiao.Tag.number:
        return at.toDart(numbers: guatiao.Numbers.text) as String;
      case guatiao.Tag.boolean:
        return at.toDart() == true ? 'true' : 'false';
      case guatiao.Tag.list:
      case guatiao.Tag.map:
      case guatiao.Tag.nullValue:
        try {
          return jsonEncode(at.toDart());
        } on JsonUnsupportedObjectError {
          return null;
        }
      case guatiao.Tag.bytes:
      case guatiao.Tag.absent:
        return null;
    }
  }

  /// Sets the field at [path] from what a person typed, read as [at] (the
  /// field's schema, found by [path] when not given) says it holds; an
  /// empty or `null` text clears it.
  void setText(String path, String? text, {FieldSchema? at}) {
    final schema = at ?? field(path) ?? FieldSchema(const {});
    _drafts.remove(path);
    _issues.remove(path);
    if (text == null || text.isEmpty) {
      _remove(path);
    } else if (schema.isAny && text == _textOf(_at(path))) {
      // An untouched value of no declared kind keeps the kind it came as.
    } else {
      final (typed, issue) = _typed(schema, text);
      if (typed is _Untaken) {
        _drafts[path] = text;
        _issues[path] = typed.issue;
        // An entry of a list or map stays where it is, holding nothing.
        if (path.endsWith(']')) {
          _put(path, null);
        } else {
          _remove(path);
        }
      } else {
        _put(path, typed);
        if (issue != null) _issues[path] = issue;
      }
    }
    _changed();
  }

  /// Sets the field at [path] to [value], plain Dart (`TypedData` for
  /// bytes) or a `guatiao.Ref`, copied.
  void setValue(String path, Object? value) {
    _drafts.remove(path);
    _issues.remove(path);
    _put(path, value);
    _changed();
  }

  /// A copy of what is at [path], or `null`; the caller closes it. A
  /// secret's copy lives in the wiping allocator.
  guatiao.Value? copyOf(String path) {
    final at = _at(path);
    if (at == null) return null;
    return field(path)?.sensitive == true || tree.isSensitive
        ? guatiao.Value.sensitive(at)
        : at.clone();
  }

  /// Clears what is at [path] and everything under it.
  void remove(String path) {
    _forgetUnder(path);
    _remove(path);
    _changed();
  }

  (Object?, String?) _typed(FieldSchema at, String text) {
    if (at.isBool) {
      return switch (text.trim()) {
        'true' => (true, null),
        'false' => (false, null),
        _ => (const _Untaken('true or false, please.'), null),
      };
    }
    if (at.isInteger) {
      final trimmed = text.trim();
      if (!RegExp(r'^[+-]?[0-9]+$').hasMatch(trimmed)) {
        return (const _Untaken('A whole number, please.'), null);
      }
      final whole = BigInt.parse(trimmed);
      return (_NumberText(whole.toString()), _outOfBoundsWhole(at, whole));
    }
    if (at.isNumber) {
      final trimmed = text.trim();
      final json = RegExp(r'^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$');
      // Decimal text only: `num.tryParse` also reads `0x1F`.
      final decimal = RegExp(
        r'^[+-]?([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?$',
      );
      final number = decimal.hasMatch(trimmed)
          ? double.tryParse(trimmed)
          : null;
      if (number == null || !number.isFinite) {
        return (const _Untaken('A number, please.'), null);
      }
      final spelled = json.hasMatch(trimmed) ? trimmed : '$number';
      return (_NumberText(spelled), _outOfBounds(at, number));
    }
    if (at.isText) {
      if (at.isBase64 && !_isBase64(text)) return (text, 'Not base64.');
      return (text, null);
    }
    final arms = at.arms;
    if (arms.isNotEmpty) {
      final words = at.wordArm?.choices;
      if (words != null && words.contains(text)) return (text, null);
      final number = at.numberArm;
      if (number != null) return _typed(number, text);
      return (text, null);
    }
    if (at.isAny) {
      try {
        return (jsonDecode(text), null);
      } on FormatException {
        return (text, null);
      }
    }
    return (text, null);
  }

  static String? _outOfBounds(FieldSchema at, double number) {
    final low = at.minimum;
    final high = at.maximum;
    if (low != null && number < FieldSchema.boundToDouble(low)) {
      return 'At least $low.';
    }
    if (high != null && number > FieldSchema.boundToDouble(high)) {
      return 'At most $high.';
    }
    return null;
  }

  /// [_outOfBounds] for an integer, compared exactly however large.
  static String? _outOfBoundsWhole(FieldSchema at, BigInt number) {
    final low = at.minimum;
    final high = at.maximum;
    final floor = low == null ? null : _whole(low, up: true);
    final ceiling = high == null ? null : _whole(high, up: false);
    if (floor != null && number < floor) return 'At least $low.';
    if (ceiling != null && number > ceiling) return 'At most $high.';
    return null;
  }

  /// A bound as the nearest whole number inside it, or `null` when it is
  /// not finite.
  static BigInt? _whole(Object bound, {required bool up}) {
    if (bound is BigInt) return bound;
    final number = bound as num;
    if (!number.isFinite) return null;
    if (number is int || number == number.truncateToDouble()) {
      return BigInt.from(number);
    }
    return BigInt.from(up ? number.ceil() : number.floor());
  }

  static bool _isBase64(String text) {
    final compact = text.replaceAll(RegExp(r'\s'), '');
    if (compact.length % 4 != 0 ||
        !RegExp(r'^[A-Za-z0-9+/]*={0,2}$').hasMatch(compact)) {
      return false;
    }
    try {
      base64.decode(compact);
      return true;
    } on FormatException {
      return false;
    }
  }

  void _changed() {
    if (!_closed) notifyListeners();
  }

  // --- writing the tree ---------------------------------------------------

  void _put(String path, Object? value) {
    final steps = parsePath(path);
    guatiao.Ref at = tree;
    for (var i = 0; i < steps.length - 1; i++) {
      at = _child(at, steps[i], steps[i + 1]);
    }
    _set(at, steps.last, value);
  }

  guatiao.Ref _child(guatiao.Ref at, Step step, Step next) {
    Object fresh() => next is PositionStep ? <Object?>[] : <String, Object?>{};
    bool container(guatiao.Ref ref) =>
        ref.tag == guatiao.Tag.map || ref.tag == guatiao.Tag.list;
    if (at.tag == guatiao.Tag.list && step is PositionStep) {
      final list = at.asList();
      if (step.position < list.length) {
        final child = list[step.position];
        if (container(child)) return child;
        list[step.position] = fresh();
        return list[step.position];
      }
      if (step.position != list.length) {
        throw RangeError.index(step.position, list, 'position');
      }
      list.add(fresh());
      return list[step.position];
    }
    final map = at.asMap();
    final key = _keyOf(step);
    final child = map[key];
    if (child != null && container(child)) return child;
    map[key] = fresh();
    return map[key]!;
  }

  static String _keyOf(Step step) => switch (step) {
    KeyStep(:final key) => key,
    PositionStep(:final key) => key,
  };

  void _set(guatiao.Ref at, Step step, Object? value) {
    if (value is _NumberText) {
      final number = guatiao.Value.number(value.text);
      try {
        _set(at, step, number);
      } finally {
        number.close();
      }
      return;
    }
    if (at.tag == guatiao.Tag.list && step is PositionStep) {
      final list = at.asList();
      if (step.position < list.length) {
        list[step.position] = value;
      } else if (step.position == list.length) {
        list.add(value);
      } else {
        throw RangeError.index(step.position, list, 'position');
      }
      return;
    }
    at.asMap()[_keyOf(step)] = value;
  }

  void _remove(String path) {
    final steps = parsePath(path);
    guatiao.Ref? at = tree;
    for (var i = 0; i < steps.length - 1 && at != null; i++) {
      at = _existing(at, steps[i]);
    }
    if (at == null) return;
    final last = steps.last;
    if (at.tag == guatiao.Tag.list && last is PositionStep) {
      final list = at.asList();
      if (last.position < list.length) list.removeAt(last.position);
    } else if (at.tag == guatiao.Tag.map) {
      // Removing a key that is not there is an error for the map, and a
      // no-op here.
      final map = at.asMap();
      final key = _keyOf(last);
      if (map.containsKey(key)) map.remove(key);
    }
  }

  guatiao.Ref? _existing(guatiao.Ref at, Step step) {
    if (at.tag == guatiao.Tag.list && step is PositionStep) {
      final list = at.asList();
      return step.position < list.length ? list[step.position] : null;
    }
    if (at.tag == guatiao.Tag.map) return at.asMap()[_keyOf(step)];
    return null;
  }

  /// Moves what [table] holds at or under [from] to [to].
  void _rekey<T>(Map<String, T> table, String from, String to) {
    final moved = {
      for (final MapEntry(:key, :value) in table.entries)
        if (isAtOrUnder(key, from)) key: value,
    };
    for (final key in moved.keys) {
      table.remove(key);
    }
    for (final MapEntry(:key, :value) in moved.entries) {
      table[to + key.substring(from.length)] = value;
    }
  }

  void _forgetUnder(String path) {
    _drafts.removeWhere((key, _) => isAtOrUnder(key, path));
    _issues.removeWhere((key, _) => isAtOrUnder(key, path));
    _ids.removeWhere((key, _) => isAtOrUnder(key, path));
  }

  // --- lists ----------------------------------------------------------------

  /// How many entries the list at [path] holds.
  int lengthOf(String path) {
    final at = _at(path);
    return at != null && at.tag == guatiao.Tag.list ? at.asList().length : 0;
  }

  /// The positions of the list at [path], in order.
  List<int> positions(String path) =>
      List<int>.generate(lengthOf(path), (at) => at);

  /// A stable identity per entry of the list at [path], for a widget's
  /// key: an entry keeps its identity when one before it is removed.
  List<int> entryIds(String path) {
    final length = lengthOf(path);
    final known = _ids[path];
    if (known != null && known.length == length) return List.of(known);
    return List.of(_ids[path] = List<int>.generate(length, (_) => _nextId++));
  }

  /// Adds [value] to the end of the list at [path], making the list when
  /// there is none.
  void addToList(String path, Object? value) {
    final ids = entryIds(path);
    final length = ids.length;
    if (_at(path)?.tag != guatiao.Tag.list) _put(path, <Object?>[]);
    _put(entryPath(path, length), value);
    _ids[path] = [...ids, _nextId++];
    _changed();
  }

  /// Adds [text], read as [at] says each entry holds, to the end of the
  /// list at [path]. Answers why it was refused, with nothing added, or
  /// `null` once it is added.
  String? addTextToList(String path, String text, {FieldSchema? at}) {
    final schema =
        at ?? field(path)?.items ?? FieldSchema(const <String, Object?>{});
    final (typed, issue) = _typed(schema, text);
    if (typed is _Untaken) return typed.issue;
    if (issue != null) return issue;
    addToList(path, typed);
    return null;
  }

  /// Takes out the entry at position [at] of the list at [path]; the list
  /// stays, empty or not.
  void removeFromList(String path, int at) {
    final ids = entryIds(path);
    final removed = entryPath(path, at);
    _forgetUnder(removed);
    _remove(removed);
    if (at < ids.length) ids.removeAt(at);
    // The entries after it move up one: what was typed or refused for
    // them goes with them.
    final length = lengthOf(path);
    for (var next = at; next < length; next++) {
      final from = entryPath(path, next + 1);
      final to = entryPath(path, next);
      for (final table in [_drafts, _issues]) {
        _rekey(table, from, to);
      }
      _rekey(_ids, from, to);
    }
    _ids[path] = ids;
    _changed();
  }

  // --- maps -----------------------------------------------------------------

  /// The keys of the open map at [path], in the order the value holds them.
  List<String> mapKeys(String path) {
    final at = _at(path);
    return at != null && at.tag == guatiao.Tag.map ? at.asMap().keys : const [];
  }

  /// Adds an entry under [key] of the map at [path], holding [value]
  /// (empty text by default); the map is made when there is none.
  ///
  /// A key the map holds already is refused, with nothing changed: the
  /// answer says why. `null` once it is added.
  String? addToMap(String path, String key, [Object? value = '']) {
    if (mapKeys(path).contains(key)) return '$key is there already.';
    if (_at(path)?.tag != guatiao.Tag.map) _put(path, <String, Object?>{});
    _put(keyedPath(path, key), value);
    _changed();
    return null;
  }

  /// Takes out the entry under [key] of the map at [path].
  void removeFromMap(String path, String key) {
    final entry = keyedPath(path, key);
    _forgetUnder(entry);
    _remove(entry);
    _changed();
  }

  // --- one of several -------------------------------------------------------

  /// Whether the field at [path], one of several kinds (`anyOf`), holds
  /// its arm with fields of its own.
  bool usesMemberArm(String path) => _at(path)?.tag == guatiao.Tag.map;

  /// Chooses a word at [path], or with `null` the arm with its own fields;
  /// `''` clears the field.
  void chooseArm(String path, String? word) {
    _forgetUnder(path);
    if (word == null) {
      if (!usesMemberArm(path)) _put(path, <String, Object?>{});
    } else if (word.isEmpty) {
      _remove(path);
    } else {
      _put(path, word);
    }
    _changed();
  }

  /// The name the tagged variant at [path] holds, or `null`.
  Object? variantName(String path, FieldSchema at) {
    final tag = at.variantTag;
    final held = _at(path);
    if (tag == null || held == null || held.tag != guatiao.Tag.map) {
      return null;
    }
    return held.asMap()[tag]?.toDart();
  }

  /// Picks the arm named [name] of the tagged variant at [path], whose
  /// schema is [at]: what the new arm also declares is kept, the rest
  /// dropped. `null` clears the field.
  void chooseVariant(String path, FieldSchema at, Object? name) {
    final arm = at.variantArm(name);
    if (name == null || arm == null) {
      remove(path);
      return;
    }
    final keep = arm.memberKeys.toSet();
    final held = _at(path);
    if (held != null && held.tag == guatiao.Tag.map) {
      for (final key in held.asMap().keys) {
        if (key != arm.tag && !keep.contains(key)) {
          final entry = memberPath(path, key);
          _forgetUnder(entry);
          _remove(entry);
        }
      }
    } else {
      _forgetUnder(path);
      _put(path, <String, Object?>{});
    }
    _put(memberPath(path, arm.tag), name);
    _changed();
  }

  // --- the whole value ------------------------------------------------------

  /// The value entered so far as plain Dart, with [issues] given every
  /// entry of [this.issues].
  Map<String, Object?> value({Map<String, String>? issues}) {
    issues?.addAll(_issues);
    return (tree.toDart() as Map).cast<String, Object?>();
  }

  /// The longest field path [reason] names, for showing a refusal on its
  /// field.
  String? fieldNamedIn(String reason) {
    final paths = <String>[];
    void walk(String path, FieldSchema at) {
      if (at.node['properties'] is! Map) {
        final arm = at.memberArm;
        if (arm != null) walk(path, arm);
        for (final variant in at.variantArms) {
          for (final key in variant.memberKeys) {
            final here = memberPath(path, key);
            paths.add(here);
            walk(here, variant.schema.member(key)!);
          }
        }
        return;
      }
      for (final key in at.memberKeys) {
        final here = memberPath(path, key);
        paths.add(here);
        walk(here, at.member(key)!);
      }
    }

    walk('', schema);
    paths.sort((a, b) => b.length.compareTo(a.length));
    for (final path in paths) {
      if (RegExp(
        '(^|[^A-Za-z0-9_.])${RegExp.escape(path)}(\$|[^A-Za-z0-9_])',
      ).hasMatch(reason)) {
        return path;
      }
    }
    return null;
  }

  /// Releases the value and the schemas and forms held for it. The
  /// model is unusable afterwards.
  void close() {
    if (_closed) return;
    _closed = true;
    tree.close();
    _scope.close();
    for (final scope in _scopes.values) {
      scope.close();
    }
  }

  @override
  void dispose() {
    close();
    super.dispose();
  }
}

/// The list or map [path] is an entry of, or [path] itself when it ends in
/// a member.
String _ownerOfEntry(String path) {
  if (!path.endsWith(']')) return path;
  var quoted = false;
  var open = -1;
  for (var at = 0; at < path.length; at++) {
    final character = path[at];
    if (quoted) {
      if (character == r'\') {
        at += 1;
      } else if (character == '"') {
        quoted = false;
      }
    } else if (character == '[') {
      open = at;
      quoted = at + 1 < path.length && path[at + 1] == '"';
      if (quoted) at += 1;
    }
  }
  return open > 0 ? path.substring(0, open) : path;
}

/// A schema and a form held as guatiao values.
class _Scope {
  _Scope._(this.formJson, this.schema, this.form);

  factory _Scope(
    Map<String, Object?> schemaJson,
    Map<String, Object?> formJson,
  ) {
    final schema = guatiao.Value.fromDart(schemaJson);
    try {
      return _Scope._(formJson, schema, guatiao.Value.fromDart(formJson));
    } catch (_) {
      schema.close();
      rethrow;
    }
  }

  /// [formJson], or the form guatiao_intake implies for [schemaJson].
  factory _Scope.of(
    Map<String, Object?> schemaJson,
    Map<String, Object?>? formJson,
  ) {
    if (formJson != null) return _Scope(schemaJson, formJson);
    final schema = guatiao.Value.fromDart(schemaJson);
    try {
      final implied = intake.forSchema(schema);
      try {
        return _Scope(
          schemaJson,
          (implied.toDart() as Map).cast<String, Object?>(),
        );
      } finally {
        implied.close();
      }
    } finally {
      schema.close();
    }
  }

  final Map<String, Object?> formJson;
  final guatiao.Value schema;
  final guatiao.Value form;

  void close() {
    schema.close();
    form.close();
  }
}
