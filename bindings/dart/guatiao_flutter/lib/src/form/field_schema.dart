// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:convert';

/// One place in a guatiao schema (JSON Schema 2020-12), read for what a
/// form needs to draw it.
///
/// It reads the schema's own keywords and guatiao_intake's `x-section`,
/// `x-order` and `x-advanced`; a key it does not know is carried and
/// ignored.
class FieldSchema {
  /// Reads [node], taking an optional value's `anyOf: [T, {type: null}]`
  /// wrapper off and keeping the words written beside it.
  FieldSchema(Map<String, Object?> node) : node = _peeled(node);

  /// The schema, with an optional value's wrapper taken off.
  final Map<String, Object?> node;

  static Map<String, Object?> _peeled(Map<String, Object?> node) {
    final arms = node['anyOf'];
    if (arms is List && arms.length == 2 && !node.containsKey('type')) {
      final others = arms
          .whereType<Map>()
          .where((arm) => arm['type'] != 'null')
          .toList();
      if (others.length == 1) {
        final words = {...node}..remove('anyOf');
        return _peeled({...others.single.cast<String, Object?>(), ...words});
      }
    }
    return node;
  }

  static List<String> _types(Object? type) => switch (type) {
    final String one => [one],
    final List<Object?> many => many.whereType<String>().toList(),
    _ => const [],
  };

  /// The `type` names, a list's or one; null among them for an optional
  /// value.
  List<String> get types => _types(node['type']);

  bool get isBool => types.contains('boolean');
  bool get isInteger => types.contains('integer');
  bool get isNumber => types.contains('number');
  bool get isText => types.contains('string');
  bool get isList => types.contains('array');
  bool get isObject => types.contains('object');

  /// guatiao's own `{"type": "bytes"}`.
  bool get isBytes => types.contains('bytes');

  /// Whether it says nothing about what it holds: any value.
  bool get isAny =>
      types.isEmpty &&
      !node.containsKey('anyOf') &&
      !node.containsKey('oneOf') &&
      !node.containsKey('enum') &&
      !node.containsKey('const');

  /// An object with declared fields.
  bool get hasMembers => node['properties'] is Map && variantTag == null;

  /// An object whose keys are data.
  bool get isOpenMap => isObject && !hasMembers && variantTag == null;

  String get title => node['title'] as String? ?? '';
  String get description => node['description'] as String? ?? '';
  String? get format => node['format'] as String?;

  /// guatiao_intake's `x-section`: `''` for the default section.
  String get section => node['x-section'] as String? ?? '';

  /// guatiao_intake's `x-order`: 0 when unset.
  int get order => (node['x-order'] as num?)?.toInt() ?? 0;

  /// guatiao_intake's `x-advanced`: rarely changed, shown folded.
  bool get advanced => node['x-advanced'] == true;

  /// `x-sensitive` or `writeOnly`: a secret, never shown back.
  bool get sensitive =>
      node['x-sensitive'] == true || node['writeOnly'] == true;

  /// `readOnly`: shown, never edited.
  bool get readOnly => node['readOnly'] == true;

  Object? get defaultValue => node['default'];
  bool get hasDefault => node.containsKey('default');

  /// `minimum`, when it is a number: a `num`, or a `BigInt` past what a
  /// 64-bit integer (or, on the web, a double) holds exactly.
  Object? get minimum => _bound(node['minimum']);

  /// `maximum`, as [minimum] is.
  Object? get maximum => _bound(node['maximum']);

  static Object? _bound(Object? value) =>
      value is num || value is BigInt ? value : null;

  /// A bound as a double, for a slider's ends.
  static double boundToDouble(Object bound) =>
      bound is BigInt ? bound.toDouble() : (bound as num).toDouble();

  /// Whether `minimum` and `maximum` are both there, the first below the
  /// second: what a slider needs.
  bool get hasRange {
    final low = minimum;
    final high = maximum;
    return low != null &&
        high != null &&
        boundToDouble(low) < boundToDouble(high);
  }

  /// `contentEncoding`, such as `base64` for bytes written as text.
  String? get contentEncoding => node['contentEncoding'] as String?;

  /// `contentMediaType`, such as `application/x-pem-file`.
  String? get contentMediaType => node['contentMediaType'] as String?;

  /// Text holding bytes in base64.
  bool get isBase64 => isText && contentEncoding?.toLowerCase() == 'base64';

  List<String> get requiredKeys =>
      (node['required'] as List?)?.whereType<String>().toList() ?? const [];

  /// The values it may hold, for a closed choice: its `enum`, its `const`,
  /// or a `oneOf` whose every arm is a `const`.
  List<Object?>? get choices {
    final values = node['enum'];
    if (values is List) return values.cast<Object?>();
    if (node.containsKey('const')) return [node['const']];
    final arms = node['oneOf'];
    if (arms is List &&
        arms.isNotEmpty &&
        variantTag == null &&
        arms.every((arm) => arm is Map && arm.containsKey('const'))) {
      return [for (final arm in arms) (arm as Map)['const']];
    }
    return null;
  }

  /// The texts it offers while accepting others: its `examples`, for text.
  List<String>? get suggestions {
    final values = node['examples'];
    return values is List && isText
        ? values.whereType<String>().toList()
        : null;
  }

  /// What a person is shown for the choice [value]: its `x-enum-labels`
  /// entry, the `title` of its `oneOf` arm, or the value itself.
  String labelOf(Object? value) {
    final key = value is String ? value : jsonEncode(value);
    final labels = node['x-enum-labels'];
    if (labels is Map && labels[key] is String) return labels[key] as String;
    final arms = node['oneOf'];
    if (arms is List) {
      for (final arm in arms) {
        if (arm is Map && arm['const'] == value && arm['title'] is String) {
          return arm['title'] as String;
        }
      }
    }
    return key;
  }

  /// Its alternatives, when it is one of several kinds (`anyOf`), the null
  /// arm left out.
  List<FieldSchema> get arms => [
    for (final arm in (node['anyOf'] as List?) ?? const [])
      if (arm is Map && arm['type'] != 'null')
        FieldSchema(arm.cast<String, Object?>()),
  ];

  /// The arm that holds a number, when one does.
  FieldSchema? get numberArm {
    for (final arm in arms) {
      if (arm.isInteger || arm.isNumber) return arm;
    }
    return null;
  }

  /// The arm that holds a fixed word or several, when one does.
  FieldSchema? get wordArm {
    for (final arm in arms) {
      if (arm.choices != null) return arm;
    }
    return null;
  }

  /// The arm with fields of its own, when one has them.
  FieldSchema? get memberArm {
    for (final arm in arms) {
      if (arm.hasMembers) return arm;
    }
    return null;
  }

  /// The key a tagged variant stores its arm's name under
  /// (`x-variant-tag`), when it is one.
  String? get variantTag {
    final tag = node['x-variant-tag'];
    return tag is String && node['oneOf'] is List ? tag : null;
  }

  /// A tagged variant's arms, in the schema's order.
  List<VariantArm> get variantArms {
    final tag = variantTag;
    if (tag == null) return const [];
    return [
      for (final arm in node['oneOf'] as List)
        if (arm is Map) ?VariantArm._read(tag, arm.cast<String, Object?>()),
    ];
  }

  /// The arm of a tagged variant named [name], when there is one.
  VariantArm? variantArm(Object? name) {
    for (final arm in variantArms) {
      if (arm.name == name) return arm;
    }
    return null;
  }

  /// The schema of the member [key], when it declares one.
  FieldSchema? member(String key) {
    final properties = node['properties'];
    final member = properties is Map ? properties[key] : null;
    return member is Map ? FieldSchema(member.cast<String, Object?>()) : null;
  }

  /// The declared member keys, those with an `x-order` first (lowest
  /// first), then the rest in the schema's order.
  List<String> get memberKeys {
    final properties = node['properties'];
    if (properties is! Map) return const [];
    final keys = properties.keys.whereType<String>().toList();
    final ordered = [
      for (final key in keys)
        if ((member(key)?.order ?? 0) != 0) key,
    ]..sort((a, b) => member(a)!.order.compareTo(member(b)!.order));
    return [
      ...ordered,
      for (final key in keys)
        if (!ordered.contains(key)) key,
    ];
  }

  /// What every entry of a list is.
  FieldSchema? get items {
    final items = node['items'];
    return items is Map ? FieldSchema(items.cast<String, Object?>()) : null;
  }

  /// What every value of an open map is.
  FieldSchema? get values {
    final values = node['additionalProperties'];
    return values is Map ? FieldSchema(values.cast<String, Object?>()) : null;
  }
}

/// One arm of a tagged variant: the name its tag holds, and its fields.
class VariantArm {
  VariantArm._(this.tag, this.name, this.schema);

  static VariantArm? _read(String tag, Map<String, Object?> node) {
    final properties = node['properties'];
    final named = properties is Map ? properties[tag] : null;
    if (named is! Map || !named.containsKey('const')) return null;
    return VariantArm._(tag, named['const'], FieldSchema(node));
  }

  /// The variant's tag key.
  final String tag;

  /// The value its tag holds for this arm.
  final Object? name;

  /// The arm's own schema: its tag and its fields.
  final FieldSchema schema;

  /// What a person is shown for it: its `title`, else its name.
  String get label => schema.title.isNotEmpty
      ? schema.title
      : (name is String ? name as String : jsonEncode(name));

  /// Its fields, the tag left out.
  List<String> get memberKeys => [
    for (final key in schema.memberKeys)
      if (key != tag) key,
  ];
}
