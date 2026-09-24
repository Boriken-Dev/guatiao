// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `guatiao_intake` against a real `--all-features` build, over a schema
/// and a form built as plain Dart maps.
library;

import 'package:guatiao/intake.dart' as form;
import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

const _schema = <String, Object?>{
  'type': 'object',
  'properties': <String, Object?>{
    'name': <String, Object?>{'type': 'string', 'title': 'Name'},
  },
  'required': ['name'],
  'additionalProperties': false,
};

void main() {
  setUpAll(libraryDir);

  late Value schema;
  late Value blank;

  setUp(() {
    schema = Value.fromDart(_schema);
    blank = Value.fromDart(<String, Object?>{});
  });

  tearDown(() {
    schema.close();
    blank.close();
  });

  test('a form that fits passes', () {
    expect(form.check(schema, blank), isNull);
  });

  test('an unknown field is reported', () {
    final bad = Value.fromDart({
      'sections': <Object?>[],
      'fields': <String, Object?>{'nope': <String, Object?>{}},
    });
    addTearDown(bad.close);
    final error = form.check(schema, bad);
    expect(error, isNotNull);
    expect(error!['kind'], 'unknown_field');
  });

  test('the layout lists the declared field', () {
    final sections = form.layout(schema, blank);
    final fields = [
      for (final section in sections) ...(section['fields']! as List),
    ];
    expect(fields, ['name']);
  });

  test('a field with no condition is visible', () {
    final values = Value.fromDart({'name': 'ana'});
    addTearDown(values.close);
    expect(form.isVisible(schema, blank, 'name', values), isTrue);
  });

  test('an unknown key is not found', () {
    final values = Value.fromDart(<String, Object?>{});
    addTearDown(values.close);
    expect(
      () => form.isVisible(schema, blank, 'nope', values),
      throwsA(isA<NotFound>()),
    );
  });

  test('a path names one place inside a value', () {
    // A dot is a member, a bracket is a position or a key, and which one
    // a bracket means is decided by what it is applied to.
    final value = Value.fromDart({
      'agent': [
        {'name': 'one'},
        {'name': 'two'},
      ],
      'env': {'PATH': '/bin', 'a.b': 'dotted', '1': 'keyed'},
    });
    try {
      expect(form.at(value, 'agent[0].name')?.toDart(), 'one');
      expect(form.at(value, 'agent[1].name')?.toDart(), 'two');
      expect(form.at(value, 'env[PATH]')?.toDart(), '/bin');

      // No quoting needed inside brackets: the `]` ends the segment.
      expect(form.at(value, 'env[a.b]')?.toDart(), 'dotted');
      // But a key that would read as a position needs it.
      expect(form.at(value, 'env["1"]')?.toDart(), 'keyed');

      // Nothing there, and not a path at all, are the same answer.
      expect(form.at(value, 'agent[9].name'), isNull);
      expect(form.at(value, 'nonesuch'), isNull);
      expect(form.at(value, 'agent['), isNull);
    } finally {
      value.close();
    }
  });

  test('a form can be made out of a schema, and a member carries its own', () {
    // An object is a form: the schema groups the fields, so a renderer
    // with no form still has one to draw.
    final nested = Value.fromDart({
      'type': 'object',
      'properties': {
        'host': {'type': 'string', 'x-section': 'net'},
        'tls': {
          'type': 'object',
          'properties': {
            'verify': {'type': 'boolean', 'x-section': 'trust'},
          },
          'additionalProperties': false,
        },
        'label': {'type': 'string'},
      },
      'additionalProperties': false,
    });
    final blank = Value.fromDart(<String, Object?>{});
    try {
      final made = form.forSchema(nested);
      try {
        final doc = made.toDart() as Map;
        // Ids only, first-appearance order.
        expect(
          (doc['sections'] as List).map((s) => (s as Map)['id']).toList(),
          ['net'],
        );
        expect(
          ((doc['fields'] as Map)['tls'] as Map)['form'],
          {
            'sections': [
              {'id': 'trust'},
            ],
          },
        );
        expect(form.check(nested, made), isNull);
      } finally {
        made.close();
      }

      // Assigned wins; a field with no members has none.
      final implied = form.formFor(blank, nested, 'tls');
      expect(implied, isNotNull);
      implied!.close();
      expect(form.formFor(blank, nested, 'label'), isNull);
      expect(form.formFor(blank, nested, 'nonesuch'), isNull);
    } finally {
      nested.close();
      blank.close();
    }
  });
}
