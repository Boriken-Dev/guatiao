// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `guatiao_intake` against a real `--all-features` build, over a schema
/// and a form built as plain Dart maps.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
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
  setUpAll(setUpLibrary);

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
    expect(intake.check(schema, blank), isNull);
  });

  test('an unknown field is reported', () {
    final bad = Value.fromDart({
      'sections': <Object?>[],
      'fields': <String, Object?>{'nope': <String, Object?>{}},
    });
    addTearDown(bad.close);
    final error = intake.check(schema, bad);
    expect(error, isNotNull);
    expect(error!['kind'], 'unknown_field');
  });

  test('the layout lists the declared field', () {
    final sections = intake.layout(schema, blank);
    final fields = [
      for (final section in sections) ...(section['fields']! as List),
    ];
    expect(fields, ['name']);
  });

  test('a field with no condition is visible', () {
    final values = Value.fromDart({'name': 'ana'});
    addTearDown(values.close);
    expect(intake.isVisible(schema, blank, 'name', values), isTrue);
  });

  test('an unknown key is not found', () {
    final values = Value.fromDart(<String, Object?>{});
    addTearDown(values.close);
    expect(
      () => intake.isVisible(schema, blank, 'nope', values),
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
      expect(intake.at(value, 'agent[0].name')?.toDart(), 'one');
      expect(intake.at(value, 'agent[1].name')?.toDart(), 'two');
      expect(intake.at(value, 'env[PATH]')?.toDart(), '/bin');

      // No quoting needed inside brackets: the `]` ends the segment.
      expect(intake.at(value, 'env[a.b]')?.toDart(), 'dotted');
      // But a key that would read as a position needs it.
      expect(intake.at(value, 'env["1"]')?.toDart(), 'keyed');

      // Nothing there, and not a path at all, are the same answer.
      expect(intake.at(value, 'agent[9].name'), isNull);
      expect(intake.at(value, 'nonesuch'), isNull);
      expect(intake.at(value, 'agent['), isNull);
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
      final made = intake.forSchema(nested);
      try {
        final doc = made.toDart() as Map;
        // Ids only, first-appearance order.
        expect(
          (doc['sections'] as List).map((s) => (s as Map)['id']).toList(),
          ['net'],
        );
        expect(((doc['fields'] as Map)['tls'] as Map)['form'], {
          'sections': [
            {'id': 'trust'},
          ],
        });
        expect(intake.check(nested, made), isNull);
      } finally {
        made.close();
      }

      // Assigned wins; a field with no members has none.
      final implied = intake.formFor(blank, nested, 'tls');
      expect(implied, isNotNull);
      implied!.close();
      expect(intake.formFor(blank, nested, 'label'), isNull);
      expect(intake.formFor(blank, nested, 'nonesuch'), isNull);
    } finally {
      nested.close();
      blank.close();
    }
  });

  group('the flat projection', () {
    const flatSchema = <String, Object?>{
      'type': 'object',
      'properties': {
        'auth': {
          'type': 'object',
          'x-variant-tag': 'auth',
          'oneOf': [
            {
              'type': 'object',
              'properties': {
                'auth': {'const': 'sso'},
              },
              'required': ['auth'],
            },
            {
              'type': 'object',
              'properties': {
                'auth': {'const': 'userpass'},
                'username': {'type': 'string'},
                'password': {'type': 'string', 'x-sensitive': true},
              },
              'required': ['auth'],
            },
          ],
        },
        'agent': {
          'type': 'array',
          'items': {
            'type': 'object',
            'properties': {
              'name': {'type': 'string'},
            },
          },
        },
        'env': {
          'type': 'object',
          'additionalProperties': {'type': 'string'},
        },
        'host': {'type': 'string', 'title': 'Host'},
      },
    };

    late Value flat;

    setUp(() => flat = Value.fromDart(flatSchema));
    tearDown(() => flat.close());

    test('resolve answers the field that governs a flat key', () {
      expect(intake.resolve(flat, 'host')!.asMap()['title']!.toDart(), 'Host');
      // One level of projection: a key inside an arm answers that field.
      expect(
        intake.resolve(flat, 'auth.password')!.asMap()['x-sensitive']!.toDart(),
        true,
      );
      expect(intake.resolve(flat, 'nonesuch'), isNull);
    });

    test('flatKeys lists a variant field and its arms', () {
      expect(intake.flatKeys(flat, 'auth'), [
        'auth',
        'auth.username',
        'auth.password',
      ]);
      expect(
        () => intake.flatKeys(flat, 'nonesuch'),
        throwsA(isA<WrongKind>()),
      );
    });

    // A list, an open map and a variant each flatten and read back.
    final cases = <(String, Object, Map<String, String>)>[
      (
        'auth',
        {'auth': 'userpass', 'username': 'alice', 'password': 's3cret'},
        {
          'auth': 'userpass',
          'auth.username': 'alice',
          'auth.password': 's3cret',
        },
      ),
      ('auth', {'auth': 'sso'}, {'auth': 'sso'}),
      (
        'agent',
        [
          {'name': 'one'},
          {'name': 'two'},
        ],
        {'agent[0].name': 'one', 'agent[1].name': 'two'},
      ),
      (
        'env',
        {'PATH': '/bin', 'a.b': 'dotted'},
        {'env[PATH]': '/bin', 'env[a.b]': 'dotted'},
      ),
      ('host', 'example.org', {'host': 'example.org'}),
    ];
    for (final (key, value, store) in cases) {
      test('a $key value flattens and reads back', () {
        final given = Value.fromDart(value);
        addTearDown(given.close);
        expect(intake.flatten(flat, key, given), store);
        final back = intake.unflatten(flat, key, store);
        expect(back, isNotNull);
        addTearDown(back!.close);
        expect(back.toDart(), value);
      });
    }

    test(
      'unflatten takes a value, and answers null when it does not apply',
      () {
        final store = Value.fromDart({'env[PATH]': '/bin'});
        addTearDown(store.close);
        final env = intake.unflatten(flat, 'env', store)!;
        addTearDown(env.close);
        expect(env.toDart(), {'PATH': '/bin'});

        // A tag naming no arm, a key naming no field, and nothing stored
        // under the field are all "does not apply".
        expect(intake.unflatten(flat, 'auth', {'auth': 'nonesuch'}), isNull);
        expect(intake.unflatten(flat, 'nonesuch', <String, String>{}), isNull);
        expect(intake.unflatten(flat, 'host', <String, String>{}), isNull);
        expect(
          () => intake.unflatten(flat, 'host', {'host': 1}),
          throwsArgumentError,
        );
      },
    );

    test('a store that is not text to text is refused in every form', () {
      final numbers = Value.fromDart({'host': 1});
      addTearDown(numbers.close);
      final notAMap = Value.fromDart(['host']);
      addTearDown(notAMap.close);
      expect(
        () => intake.unflatten(flat, 'host', numbers),
        throwsArgumentError,
      );
      expect(
        () => intake.unflatten(flat, 'host', notAMap),
        throwsArgumentError,
      );
      expect(
        () => intake.unflatten(flat, 'host', {'host': 1}),
        throwsArgumentError,
      );
      // Nothing stored is not a malformed store.
      final empty = Value.fromDart(<String, Object?>{});
      addTearDown(empty.close);
      expect(intake.unflatten(flat, 'host', empty), isNull);
    });

    test('flatten refuses a value of the wrong shape', () {
      final given = Value.fromDart([1]);
      addTearDown(given.close);
      expect(
        () => intake.flatten(flat, 'auth', given),
        throwsA(isA<WrongKind>()),
      );
    });
  });
}
