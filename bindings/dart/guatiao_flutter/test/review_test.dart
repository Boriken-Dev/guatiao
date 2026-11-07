// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:guatiao/guatiao.dart' as guatiao;
import 'package:guatiao_flutter/guatiao_flutter.dart';

import 'harness.dart';

List<String> mockClipboard(WidgetTester tester) {
  final copied = <String>[];
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    SystemChannels.platform,
    (call) async {
      if (call.method == 'Clipboard.setData') {
        copied.add((call.arguments as Map)['text'] as String);
      }
      return null;
    },
  );
  addTearDown(
    () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    ),
  );
  return copied;
}

TextField textFieldIn(WidgetTester tester, String path) => tester.widget(
  find.descendant(of: field(path), matching: find.byType(TextField)).first,
);

IconButton removeButton(WidgetTester tester, int at) =>
    tester.widget(find.widgetWithIcon(IconButton, Icons.delete_outline).at(at));

FormModel plain(Map<String, Object?> schema, [Object? value]) {
  final model = FormModel(schema: schema, value: value);
  addTearDown(model.dispose);
  return model;
}

void main() {
  setUpAll(setUpGuatiao);

  group('bounds past 64 bits', () {
    final big = BigInt.parse('18446744073709551615');

    test('a bound read back from the schema is exact', () {
      final m = plain(
        obj({
          'n': {'type': 'integer', 'maximum': big},
        }),
      );
      expect(m.field('n')!.maximum, big);
    });

    test('an integer is compared exactly with a BigInt bound', () {
      final m = plain(
        obj({
          'n': {'type': 'integer', 'minimum': 0, 'maximum': big},
        }),
      );
      m.setText('n', '18446744073709551615');
      expect(m.issues, isEmpty);
      expect(m.text('n'), '18446744073709551615');
      m.setText('n', '18446744073709551616');
      expect(m.issues['n'], 'At most 18446744073709551615.');
    });

    test('a bound just past 2^53 does not round', () {
      final bound = BigInt.parse('9007199254740993');
      final m = plain(
        obj({
          'n': {'type': 'integer', 'maximum': bound},
        }),
      );
      m.setText('n', '9007199254740993');
      expect(m.issues, isEmpty);
      expect(m.text('n'), '9007199254740993');
      m.setText('n', '9007199254740994');
      expect(m.issues['n'], 'At most 9007199254740993.');
    });

    test('a 128-bit lower bound refuses what is below it', () {
      final low = BigInt.parse('-170141183460469231731687303715884105728');
      final m = plain(
        obj({
          'n': {'type': 'integer', 'minimum': low},
        }),
      );
      m.setText('n', '-170141183460469231731687303715884105729');
      expect(m.issues['n'], startsWith('At least -17014118346'));
    });

    test('a number is compared with a BigInt bound', () {
      final m = plain(
        obj({
          'x': {'type': 'number', 'maximum': big},
        }),
      );
      m.setText('x', '1e30');
      expect(m.issues['x'], 'At most 18446744073709551615.');
    });

    testWidgets('a slider over such a range builds', (tester) async {
      await show(
        tester,
        obj({
          'n': {'type': 'integer', 'minimum': 0, 'maximum': big},
        }),
        form: {
          'fields': {
            'n': {'widget': 'slider'},
          },
        },
      );
      expect(find.byType(Slider), findsOneWidget);
      expect(tester.takeException(), isNull);
    });

    testWidgets('typing a large integer into the box is read', (tester) async {
      final m = await show(
        tester,
        obj({
          'n': {'type': 'integer', 'maximum': big},
        }),
      );
      await tester.enterText(textIn('n'), '18446744073709551615');
      await tester.pump();
      expect(m.issues, isEmpty);
      expect(tester.takeException(), isNull);
    });
  });

  group('numbers as typed', () {
    final schema = obj({
      'count': {'type': 'integer', 'minimum': 0},
      'ratio': {'type': 'number'},
    });

    test('text that is not a whole number is a draft and an issue', () {
      final m = plain(schema);
      m.setText('count', '12a');
      expect(m.issues['count'], 'A whole number, please.');
      expect(m.read('count'), isA<guatiao.Absent>());
      expect(m.text('count'), '12a');
    });

    test('a value below its minimum is held and flagged', () {
      final m = plain(schema);
      m.setText('count', '-1');
      expect(m.issues['count'], 'At least 0.');
      expect(m.read('count'), -1);
    });

    test('hex, infinity and a huge exponent are not numbers', () {
      final m = plain(schema);
      for (final text in ['0x1F', 'Infinity', 'NaN', '1e400']) {
        m.setText('ratio', text);
        expect(m.issues['ratio'], 'A number, please.', reason: text);
        expect(m.read('ratio'), isA<guatiao.Absent>(), reason: text);
      }
    });

    test('decimal spellings are taken', () {
      final m = plain(schema);
      m.setText('ratio', '.5');
      expect(m.read('ratio'), 0.5);
      m.setText('ratio', '+2.');
      expect(m.read('ratio'), 2);
    });

    test('an integer past 2^53 keeps its digits', () {
      final m = plain(schema);
      m.setText('count', '9007199254740993');
      expect(m.text('count'), '9007199254740993');
      expect(m.issues, isEmpty);
    });
  });

  group('a secret is kept in the wiping allocator', () {
    final secret = obj({
      'token': {'type': 'string', 'x-sensitive': true},
    });

    test('the value and a copy of the secret are sensitive', () {
      final m = plain(secret, {'token': 'x'});
      expect(m.tree.isSensitive, isTrue);
      final copy = m.copyOf('token')!;
      addTearDown(copy.close);
      expect(copy.isSensitive, isTrue);
    });

    test('a schema with no secret is not made sensitive', () {
      final m = plain(
        obj({
          'a': {'type': 'string'},
        }),
        {'a': 'x'},
      );
      expect(m.tree.isSensitive, isFalse);
    });

    test('a sensitive value the app passes stays sensitive', () {
      final held = guatiao.Value.sensitive({'a': 'x'});
      addTearDown(held.close);
      final m = plain(
        obj({
          'a': {'type': 'string'},
        }),
        held,
      );
      expect(m.tree.isSensitive, isTrue);
    });

    test('a secret nested in a list is found', () {
      final m = plain(
        obj({
          'items': {
            'type': 'array',
            'items': obj({
              'k': {'type': 'string', 'writeOnly': true},
            }),
          },
        }),
      );
      expect(m.tree.isSensitive, isTrue);
    });

    testWidgets(
      'a writable secret shows no stored value, only that one is set',
      (tester) async {
        final m = await show(tester, secret, value: {'token': 'old'});
        expect(
          tester.widget<EditableText>(textIn('token')).controller.text,
          '',
        );
        expect(find.text('Set; type to replace it'), findsOneWidget);
        expect(find.text('old'), findsNothing);
        await tester.enterText(textIn('token'), 'new');
        await tester.pump();
        expect(m.read('token'), 'new');
        await tester.enterText(textIn('token'), '');
        await tester.pump();
        expect(m.read('token'), 'old');
      },
    );

    testWidgets('a masked secret keeps no plaintext in its box', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'token': {'type': 'string', 'x-sensitive': true, 'readOnly': true},
        }),
        value: {'token': 'hunter2'},
      );
      final box = tester.widget<EditableText>(textIn('token'));
      expect(box.controller.text, isNot(contains('hunter2')));
      expect(box.controller.toString(), isNot(contains('hunter2')));
      expect(box.controller.text, '•' * 8);
      await tester.tap(find.byKey(const Key('form-reveal-token')));
      await tester.pump();
      expect(
        tester.widget<EditableText>(textIn('token')).controller.text,
        'hunter2',
      );
      await tester.tap(find.byKey(const Key('form-reveal-token')));
      await tester.pump();
      expect(
        tester.widget<EditableText>(textIn('token')).controller.text,
        isNot(contains('hunter2')),
      );
    });

    testWidgets('copy puts the secret on the clipboard, and only on a tap', (
      tester,
    ) async {
      final copied = mockClipboard(tester);
      await show(
        tester,
        obj({
          'token': {'type': 'string', 'x-sensitive': true, 'readOnly': true},
        }),
        value: {'token': 'hunter2'},
      );
      await tester.pump();
      expect(copied, isEmpty);
      await tester.tap(find.byKey(const Key('form-copy-token')));
      await tester.pump();
      expect(copied, ['hunter2']);
    });

    testWidgets('a secret map value is not put back in its box', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'env': {
            'type': 'object',
            'additionalProperties': {'type': 'string', 'x-sensitive': true},
          },
        }),
        value: {
          'env': {'K': 's3cret'},
        },
      );
      final box = tester.widget<EditableText>(textIn('env[K]'));
      expect(box.controller.text, '');
      expect(box.obscureText, isTrue);
      expect(box.enableSuggestions, isFalse);
      expect(box.autocorrect, isFalse);
      await tester.enterText(textIn('env[K]'), 'zz');
      await tester.pump();
      expect(m.read('env[K]'), 'zz');
      await tester.enterText(textIn('env[K]'), '');
      await tester.pump();
      expect(m.read('env[K]'), 's3cret');
    });
  });

  group('bytes', () {
    testWidgets('with no callbacks there is nothing to pick or save', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'blob': {'type': 'bytes'},
        }),
        value: {
          'blob': Uint8List.fromList([1, 2]),
        },
      );
      expect(find.byKey(const Key('form-pick-blob')), findsNothing);
      expect(find.byKey(const Key('form-save-blob')), findsNothing);
      expect(find.text('2 bytes'), findsOneWidget);
    });

    testWidgets('an empty field says so', (tester) async {
      await show(
        tester,
        obj({
          'blob': {'type': 'bytes'},
        }),
      );
      expect(find.text('Empty'), findsOneWidget);
    });

    testWidgets('secret bytes show no length, and save still hands them out', (
      tester,
    ) async {
      Uint8List? saved;
      await show(
        tester,
        obj({
          'blob': {'type': 'bytes', 'x-sensitive': true},
        }),
        value: {
          'blob': Uint8List.fromList([1, 2, 3]),
        },
        onSaveBytes: (path, bytes, field) async => saved = bytes,
      );
      expect(find.text('Set'), findsOneWidget);
      expect(find.textContaining('bytes'), findsNothing);
      await tester.tap(find.byKey(const Key('form-save-blob')));
      await tester.pump();
      expect(saved, [1, 2, 3]);
    });
  });

  group('unavailable', () {
    testWidgets('an edit attempted on an unavailable field keeps its value', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'a': {'type': 'string'},
        }),
        value: {'a': 'kept'},
        unavailable: {'a': 'No'},
      );
      await tester.enterText(textIn('a'), 'changed');
      await tester.pump();
      expect(m.read('a'), 'kept');
    });

    testWidgets('a group takes everything under it along', (tester) async {
      final m = await show(
        tester,
        obj({
          'tls': obj({
            'ca': {'type': 'string'},
          }),
        }),
        value: {
          'tls': {'ca': 'x'},
        },
        unavailable: {'tls': 'Managed by policy'},
      );
      expect(find.text('Managed by policy'), findsOneWidget);
      expect(tester.widget<EditableText>(textIn('tls.ca')).readOnly, isTrue);
      await tester.enterText(textIn('tls.ca'), 'y');
      await tester.pump();
      expect(m.read('tls.ca'), 'x');
    });

    testWidgets('an entry of a list of values is locked and shows why', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'tags': {
            'type': 'array',
            'items': {'type': 'string'},
          },
        }),
        value: {
          'tags': ['a', 'b'],
        },
        unavailable: {'tags[0]': 'Pinned'},
      );
      expect(
        tester
            .widget<InputChip>(find.byKey(const Key('form-entry-tags-0')))
            .onDeleted,
        isNull,
      );
      expect(
        tester
            .widget<InputChip>(find.byKey(const Key('form-entry-tags-1')))
            .onDeleted,
        isNotNull,
      );
      expect(find.text('Pinned'), findsOneWidget);
    });

    testWidgets('an entry of a list of groups cannot be removed or edited', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'agents': {
            'type': 'array',
            'items': obj({
              'name': {'type': 'string'},
            }),
          },
        }),
        value: {
          'agents': [
            {'name': 'one'},
            {'name': 'two'},
          ],
        },
        unavailable: {'agents[0]': 'Fixed agent'},
      );
      expect(removeButton(tester, 0).onPressed, isNull);
      expect(removeButton(tester, 1).onPressed, isNotNull);
      expect(find.text('Fixed agent'), findsOneWidget);
      await tester.enterText(textIn('agents[0].name'), 'x');
      await tester.pump();
      expect(m.read('agents[0].name'), 'one');
    });

    testWidgets('a chip before a locked one cannot be taken out', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'tags': {
            'type': 'array',
            'items': {'type': 'string'},
          },
        }),
        value: {
          'tags': ['a', 'b', 'c'],
        },
        unavailable: {'tags[1]': 'Pinned'},
      );
      Widget chip(int at) =>
          find.byKey(Key('form-entry-tags-$at')).evaluate().single.widget;
      expect((chip(0) as InputChip).onDeleted, isNull);
      expect((chip(1) as InputChip).onDeleted, isNull);
      expect((chip(2) as InputChip).onDeleted, isNotNull);
    });

    testWidgets('an unavailable field locks its entry and those before it', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'agents': {
            'type': 'array',
            'items': obj({
              'name': {'type': 'string'},
            }),
          },
        }),
        value: {
          'agents': [
            {'name': 'one'},
            {'name': 'two'},
          ],
        },
        unavailable: {'agents[1].name': 'Name is fixed'},
      );
      // Taking [0] out would move the locked entry up to [0].
      expect(removeButton(tester, 0).onPressed, isNull);
      expect(removeButton(tester, 1).onPressed, isNull);
    });

    testWidgets('an entry of a map of values is locked and shows why', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'env': {
            'type': 'object',
            'additionalProperties': {'type': 'string'},
          },
        }),
        value: {
          'env': {'PATH': '/bin', 'HOME': '/h'},
        },
        unavailable: {'env[PATH]': 'Managed'},
      );
      expect(find.text('Managed'), findsOneWidget);
      expect(tester.widget<EditableText>(textIn('env[PATH]')).readOnly, isTrue);
      expect(
        tester.widget<EditableText>(textIn('env[HOME]')).readOnly,
        isFalse,
      );
      expect(
        tester
            .widget<IconButton>(
              find.ancestor(
                of: find.byTooltip('Managed'),
                matching: find.byType(IconButton),
              ),
            )
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<IconButton>(
              find.ancestor(
                of: find.byTooltip('Remove HOME'),
                matching: find.byType(IconButton),
              ),
            )
            .onPressed,
        isNotNull,
      );
      await tester.enterText(textIn('env[PATH]'), '/x');
      await tester.pump();
      expect(m.read('env[PATH]'), '/bin');
    });

    testWidgets('an entry of a map of groups is locked and shows why', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'hosts': {
            'type': 'object',
            'additionalProperties': obj({
              'port': {'type': 'integer'},
            }),
          },
        }),
        value: {
          'hosts': {
            'a': {'port': 1},
            'b': {'port': 2},
          },
        },
        unavailable: {'hosts[a]': 'Locked host'},
      );
      expect(find.text('Locked host'), findsOneWidget);
      expect(removeButton(tester, 0).onPressed, isNull);
      expect(removeButton(tester, 1).onPressed, isNotNull);
    });

    testWidgets('a union arm cannot be switched over an unavailable field', (
      tester,
    ) async {
      final schema = obj({
        'auth': {
          'type': 'object',
          'x-variant-tag': 'kind',
          'oneOf': [
            {
              'title': 'None',
              'type': 'object',
              'properties': {
                'kind': {'const': 'none'},
              },
            },
            {
              'title': 'Token',
              'type': 'object',
              'properties': {
                'kind': {'const': 'token'},
                'token': {'type': 'string'},
              },
            },
          ],
        },
      });
      final value = {
        'auth': {'kind': 'token', 'token': 't'},
      };
      await show(tester, schema, value: value);
      expect(
        tester
            .widget<DropdownButtonFormField<int>>(
              find.byType(DropdownButtonFormField<int>),
            )
            .onChanged,
        isNotNull,
      );
      await show(
        tester,
        schema,
        value: value,
        unavailable: {'auth.token': 'Rotated by an admin'},
      );
      expect(
        tester
            .widget<DropdownButtonFormField<int>>(
              find.byType(DropdownButtonFormField<int>),
            )
            .onChanged,
        isNull,
      );
      expect(find.text('Rotated by an admin'), findsWidgets);
    });

    testWidgets('a union arm is picked from the dropdown', (tester) async {
      final m = await show(
        tester,
        obj({
          'auth': {
            'type': 'object',
            'x-variant-tag': 'kind',
            'oneOf': [
              {
                'title': 'None',
                'type': 'object',
                'properties': {
                  'kind': {'const': 'none'},
                },
              },
              {
                'title': 'Token',
                'type': 'object',
                'properties': {
                  'kind': {'const': 'token'},
                  'token': {'type': 'string'},
                },
              },
            ],
          },
        }),
      );
      await tester.tap(find.byType(DropdownButtonFormField<int>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Token').last);
      await tester.pumpAndSettle();
      expect(m.variantName('auth', m.field('auth')!), 'token');
      expect(field('auth.token'), findsOneWidget);
    });

    test('switching a variant keeps what the new arm also declares', () {
      final m = plain(
        obj({
          'v': {
            'type': 'object',
            'x-variant-tag': 'kind',
            'oneOf': [
              obj({
                'kind': {'const': 'a'},
                'shared': {'type': 'string'},
                'x': {'type': 'string'},
              }),
              obj({
                'kind': {'const': 'b'},
                'shared': {'type': 'string'},
                'y': {'type': 'string'},
              }),
            ],
          },
        }),
        {
          'v': {'kind': 'a', 'shared': 's', 'x': '1'},
        },
      );
      m.chooseVariant('v', m.field('v')!, 'b');
      expect(m.value()['v'], {'kind': 'b', 'shared': 's'});
    });
  });

  group('one of several kinds', () {
    final schema = obj({
      'mode': {
        'anyOf': [
          {
            'type': 'string',
            'enum': ['none', 'auto'],
          },
          obj({
            'a': {'type': 'string'},
          }),
        ],
      },
    });

    testWidgets('a word the schema does not list is shown', (tester) async {
      await show(tester, schema, value: {'mode': 'weird'});
      expect(tester.takeException(), isNull);
      expect(find.text('weird'), findsWidgets);
    });

    testWidgets('a number or a null held there does not break the box', (
      tester,
    ) async {
      await show(tester, schema, value: {'mode': 5});
      expect(tester.takeException(), isNull);
      await show(tester, schema, value: {'mode': null});
      expect(tester.takeException(), isNull);
    });

    testWidgets('its own fields hold the arm while one is unavailable', (
      tester,
    ) async {
      final value = {
        'mode': {'a': 'x'},
      };
      await show(tester, schema, value: value);
      expect(
        tester
            .widget<DropdownButtonFormField<String>>(
              find.byType(DropdownButtonFormField<String>),
            )
            .onChanged,
        isNotNull,
      );
      await show(
        tester,
        schema,
        value: value,
        unavailable: {'mode.a': 'Locked'},
      );
      expect(
        tester
            .widget<DropdownButtonFormField<String>>(
              find.byType(DropdownButtonFormField<String>),
            )
            .onChanged,
        isNull,
      );
    });
  });

  group('a number or a word', () {
    final schema = obj({
      'limit': {
        'anyOf': [
          {'type': 'integer'},
          {
            'type': 'string',
            'enum': ['unlimited'],
          },
        ],
      },
    });

    testWidgets('the word is chosen, and the number text is read', (
      tester,
    ) async {
      final m = await show(tester, schema);
      await tester.enterText(textIn('limit'), '7');
      await tester.pump();
      expect(m.read('limit'), 7);
      await tester.tap(find.byKey(const Key('form-word-limit')));
      await tester.pump();
      expect(m.read('limit'), 'unlimited');
    });

    testWidgets('an unavailable word is not newly chosen', (tester) async {
      final m = await show(
        tester,
        schema,
        value: {'limit': 3},
        unavailableChoices: {
          'limit': {'unlimited': 'Not for you'},
        },
      );
      expect(find.text('Not for you'), findsOneWidget);
      expect(
        tester
            .widget<CheckboxListTile>(find.byKey(const Key('form-word-limit')))
            .onChanged,
        isNull,
      );
      expect(m.read('limit'), 3);
    });

    testWidgets('a held unavailable word is locked', (tester) async {
      await show(
        tester,
        schema,
        value: {'limit': 'unlimited'},
        unavailableChoices: {
          'limit': {'unlimited': 'Not for you'},
        },
      );
      expect(
        tester
            .widget<CheckboxListTile>(find.byKey(const Key('form-word-limit')))
            .onChanged,
        isNull,
      );
    });

    testWidgets('the box follows a value the app sets', (tester) async {
      final m = await show(tester, schema, value: {'limit': 3});
      m.setValue('limit', 9);
      await tester.pump();
      expect(tester.widget<EditableText>(textIn('limit')).controller.text, '9');
    });
  });

  group('a choice the person lacks', () {
    final mode = obj({
      'mode': {
        'enum': ['a', 'b', 'c'],
      },
    });

    testWidgets('the dropdown item is disabled and picking it is refused', (
      tester,
    ) async {
      final m = await show(
        tester,
        mode,
        value: {'mode': 'a'},
        unavailableChoices: {
          'mode': {'b': 'Needs approval'},
        },
      );
      await tester.tap(find.byType(DropdownButtonFormField<int>));
      await tester.pumpAndSettle();
      final item = tester.widget<DropdownMenuItem<int>>(
        find
            .ancestor(
              of: find.text('b'),
              matching: find.byType(DropdownMenuItem<int>),
            )
            .last,
      );
      expect(item.enabled, isFalse);
      await tester.tap(find.text('b').last);
      await tester.pumpAndSettle();
      expect(m.read('mode'), 'a');
    });

    testWidgets('a held unavailable choice locks the dropdown', (tester) async {
      final m = await show(
        tester,
        mode,
        value: {'mode': 'b'},
        unavailableChoices: {
          'mode': {'b': 'Needs approval'},
        },
      );
      expect(
        tester
            .widget<DropdownButtonFormField<int>>(
              find.byType(DropdownButtonFormField<int>),
            )
            .onChanged,
        isNull,
      );
      expect(m.read('mode'), 'b');
    });

    testWidgets('a switch will not turn on to an unavailable value', (
      tester,
    ) async {
      final schema = obj({
        'on': {'type': 'boolean'},
      });
      await show(
        tester,
        schema,
        value: {'on': false},
        unavailableChoices: {
          'on': {true: 'Not allowed'},
        },
      );
      expect(tester.widget<SwitchListTile>(field('on')).onChanged, isNull);
    });

    testWidgets('a switch holding an unavailable value is locked', (
      tester,
    ) async {
      final schema = obj({
        'on': {'type': 'boolean'},
      });
      await show(
        tester,
        schema,
        value: {'on': true},
        unavailableChoices: {
          'on': {true: 'Not allowed'},
        },
      );
      expect(tester.widget<SwitchListTile>(field('on')).onChanged, isNull);
    });

    testWidgets('an unavailable choice cannot be typed into an open picker', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'h': {'type': 'string', 'format': 'host'},
        }),
        choicesFor: (format) => FieldChoices(const ['good', 'bad'], open: true),
        unavailableChoices: {
          'h': {'bad': 'Not allowed'},
        },
      );
      await tester.enterText(textIn('h'), 'bad');
      await tester.pump();
      expect(m.read('h'), isA<guatiao.Absent>());
      expect(find.text('Not allowed'), findsOneWidget);
      await tester.enterText(textIn('h'), 'good');
      await tester.pump();
      expect(m.read('h'), 'good');
    });

    testWidgets('a held unavailable choice locks an open picker', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'h': {'type': 'string', 'format': 'host'},
        }),
        value: {'h': 'bad'},
        choicesFor: (format) => FieldChoices(const ['good', 'bad'], open: true),
        unavailableChoices: {
          'h': {'bad': 'Not allowed'},
        },
      );
      expect(textFieldIn(tester, 'h').onChanged, isNull);
    });

    testWidgets('an open picker holding an unavailable choice is read-only', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'h': {'type': 'string', 'format': 'host'},
        }),
        value: {'h': 'bad'},
        choicesFor: (format) => FieldChoices(const ['good', 'bad'], open: true),
        unavailableChoices: {
          'h': {'bad': 'Not allowed'},
        },
      );
      expect(textFieldIn(tester, 'h').readOnly, isTrue);
      expect(
        tester.widget<Autocomplete<String>>(field('h')).onSelected,
        isNull,
      );
    });

    testWidgets('an open picker that is read-only picks nothing', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'h': {'type': 'string', 'format': 'host', 'readOnly': true},
        }),
        value: {'h': 'good'},
        choicesFor: (format) => FieldChoices(const ['good', 'bad'], open: true),
      );
      expect(
        tester.widget<Autocomplete<String>>(field('h')).onSelected,
        isNull,
      );
    });

    testWidgets('an open picker over a free value picks', (tester) async {
      await show(
        tester,
        obj({
          'h': {'type': 'string', 'format': 'host'},
        }),
        value: {'h': 'good'},
        choicesFor: (format) => FieldChoices(const ['good', 'bad'], open: true),
      );
      expect(
        tester.widget<Autocomplete<String>>(field('h')).onSelected,
        isNotNull,
      );
    });

    testWidgets('an unavailable choice cannot be added to a list by typing', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'tags': {
            'type': 'array',
            'items': {'type': 'string'},
          },
        }),
        unavailableChoices: {
          'tags': {'x': 'Not here'},
        },
      );
      await tester.enterText(find.byKey(const Key('form-add-tags')), 'x');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(m.read('tags'), isA<guatiao.Absent>());
      expect(find.text('Not here'), findsOneWidget);
      await tester.enterText(find.byKey(const Key('form-add-tags')), 'y');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(m.read('tags'), ['y']);
    });
  });

  group('dropdowns over what is held', () {
    testWidgets('a closed picker holding the empty text lists None once', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'c': {'type': 'string', 'format': 'color'},
        }),
        value: {'c': ''},
        choicesFor: (format) => FieldChoices(const ['red', 'blue']),
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('the app supplies choices for a format', (tester) async {
      final m = await show(
        tester,
        obj({
          'c': {'type': 'string', 'format': 'color'},
        }),
        choicesFor: (format) => format == 'color'
            ? FieldChoices(const ['red', 'blue'], labels: const {'red': 'Red'})
            : null,
      );
      await tester.tap(find.byType(DropdownButtonFormField<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Red').last);
      await tester.pumpAndSettle();
      expect(m.read('c'), 'red');
    });
  });

  group('maps', () {
    final schema = obj({
      'env': {
        'type': 'object',
        'additionalProperties': {'type': 'string'},
      },
    });

    test('adding a key the map holds is refused and changes nothing', () {
      final m = plain(schema, {
        'env': {'PATH': '/bin'},
      });
      expect(m.addToMap('env', 'PATH', 'x'), contains('already'));
      expect(m.read('env[PATH]'), '/bin');
      expect(m.addToMap('env', 'HOME', '/h'), isNull);
      expect(m.read('env[HOME]'), '/h');
    });

    testWidgets('the new-key box says so and keeps the value', (tester) async {
      final m = await show(
        tester,
        schema,
        value: {
          'env': {'PATH': '/bin'},
        },
      );
      await tester.enterText(find.byKey(const Key('form-add-env')), 'PATH');
      await tester.tap(
        find.descendant(
          of: find.byKey(const Key('form-add-env')),
          matching: find.byIcon(Icons.add),
        ),
      );
      await tester.pump();
      expect(find.textContaining('already'), findsOneWidget);
      expect(m.read('env[PATH]'), '/bin');
    });

    testWidgets('the entry box follows a value the app sets', (tester) async {
      final m = await show(
        tester,
        schema,
        value: {
          'env': {'PATH': '/bin'},
        },
      );
      m.setValue('env[PATH]', '/new');
      await tester.pump();
      expect(
        tester.widget<EditableText>(textIn('env[PATH]')).controller.text,
        '/new',
      );
    });
  });

  group('model edges', () {
    test('taking out a list entry keeps the others drafts and issues', () {
      final m = plain(
        obj({
          'nums': {
            'type': 'array',
            'items': {'type': 'integer'},
          },
        }),
      );
      m.addToList('nums', 1);
      m.addToList('nums', 2);
      m.setText('nums[1]', 'zz', at: m.field('nums')!.items);
      expect(m.issues, contains('nums[1]'));
      m.removeFromList('nums', 0);
      expect(m.issues, contains('nums[0]'));
      expect(m.issues, isNot(contains('nums[1]')));
      expect(m.text('nums[0]'), 'zz');
    });

    test('a form that cannot be held is refused', () {
      expect(
        () => FormModel(
          schema: obj({
            'a': {'type': 'string'},
          }),
          form: {
            'sections': [Object()],
          },
        ),
        throwsA(anything),
      );
    });

    testWidgets('a text box follows a value the app sets', (tester) async {
      final m = await show(
        tester,
        obj({
          'name': {'type': 'string'},
        }),
        value: {'name': 'first'},
      );
      m.setValue('name', 'changed');
      await tester.pump();
      expect(
        tester.widget<EditableText>(textIn('name')).controller.text,
        'changed',
      );
    });

    testWidgets('typing is never rewritten under the person', (tester) async {
      final m = await show(
        tester,
        obj({
          'n': {'type': 'integer'},
        }),
      );
      await tester.enterText(textIn('n'), ' 7 ');
      await tester.pump();
      expect(tester.widget<EditableText>(textIn('n')).controller.text, ' 7 ');
      expect(m.read('n'), 7);
    });
  });

  group('other controls', () {
    testWidgets('a slider sets the value', (tester) async {
      final m = await show(
        tester,
        obj({
          'n': {'type': 'integer', 'minimum': 0, 'maximum': 10},
        }),
        form: {
          'fields': {
            'n': {'widget': 'slider'},
          },
        },
      );
      await tester.tap(find.byType(Slider));
      await tester.pump();
      expect(m.read('n'), 5);
    });

    testWidgets('a group can open in a dialog', (tester) async {
      final m = await show(
        tester,
        obj({
          'tls': obj({
            'ca': {'type': 'string'},
          }),
        }),
        form: {
          'fields': {
            'tls': {'widget': 'dialog'},
          },
        },
      );
      expect(field('tls.ca'), findsNothing);
      await tester.tap(find.byKey(const Key('form-open-tls')));
      await tester.pumpAndSettle();
      await tester.enterText(textIn('tls.ca'), 'pem');
      await tester.pump();
      await tester.tap(find.byKey(const Key('form-close-tls')));
      await tester.pumpAndSettle();
      expect(m.read('tls.ca'), 'pem');
    });
  });
}
