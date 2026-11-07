// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:guatiao_flutter/guatiao_flutter.dart';

import 'support.dart';

Map<String, Object?> obj(Map<String, Object?> props) => {
  'type': 'object',
  'properties': props,
};

Future<FormModel> show(
  WidgetTester tester,
  Map<String, Object?> schema, {
  Map<String, Object?>? form,
  Object? value,
  Map<String, String> problems = const {},
  Map<String, String> unavailable = const {},
  Map<String, Map<Object, String>> unavailableChoices = const {},
  PickBytes? onPickBytes,
  SaveBytes? onSaveBytes,
  bool enabled = true,
}) async {
  final model = FormModel(schema: schema, form: form, value: value);
  addTearDown(model.dispose);
  tester.view.physicalSize = const Size(900, 3000);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: IntakeFormView(
            model: model,
            problems: problems,
            unavailable: unavailable,
            unavailableChoices: unavailableChoices,
            onPickBytes: onPickBytes,
            onSaveBytes: onSaveBytes,
            enabled: enabled,
          ),
        ),
      ),
    ),
  );
  return model;
}

Finder field(String path) => find.byKey(Key('form-field-$path'));
Finder textIn(String path) =>
    find.descendant(of: field(path), matching: find.byType(EditableText));

void main() {
  setUpAll(setUpGuatiao);

  group('controls', () {
    testWidgets('text is typed into the value', (tester) async {
      final m = await show(
        tester,
        obj({
          'name': {'type': 'string'},
        }),
      );
      await tester.enterText(textIn('name'), 'abc');
      await tester.pump();
      expect(m.read('name'), 'abc');
    });

    testWidgets('number and integer read as their kind', (tester) async {
      final m = await show(
        tester,
        obj({
          'ratio': {'type': 'number'},
          'count': {'type': 'integer'},
        }),
      );
      await tester.enterText(textIn('ratio'), '1.5');
      await tester.enterText(textIn('count'), '7');
      await tester.pump();
      expect(m.read('ratio'), 1.5);
      expect(m.read('count'), 7);
    });

    testWidgets('boolean toggles', (tester) async {
      final m = await show(
        tester,
        obj({
          'on': {'type': 'boolean'},
        }),
        value: {'on': false},
      );
      final control = find.descendant(
        of: field('on'),
        matching: find.byWidgetPredicate((w) => w is Switch || w is Checkbox),
      );
      await tester.tap(control.first);
      await tester.pump();
      expect(m.read('on'), true);
    });

    testWidgets('an enum shows its labels', (tester) async {
      final m = await show(
        tester,
        obj({
          'mode': {
            'enum': ['a', 'b'],
            'x-enum-labels': {'a': 'Alpha', 'b': 'Beta'},
          },
        }),
        value: {'mode': 'a'},
      );
      expect(find.text('Alpha'), findsWidgets);
      m.setValue('mode', 'b');
      await tester.pump();
      expect(find.text('Beta'), findsWidgets);
    });

    testWidgets('a list of values adds and removes', (tester) async {
      final m = await show(
        tester,
        obj({
          'tags': {
            'type': 'array',
            'items': {'type': 'string'},
          },
        }),
        value: {
          'tags': ['x'],
        },
      );
      expect(find.byKey(const Key('form-entry-tags-0')), findsOneWidget);
      m.addToList('tags', 'y');
      await tester.pump();
      expect(find.byKey(const Key('form-entry-tags-1')), findsOneWidget);
      m.removeFromList('tags', 0);
      await tester.pump();
      expect(m.read('tags'), ['y']);
    });

    testWidgets('a list of groups draws each member', (tester) async {
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
      );
      expect(field('agents[0].name'), findsOneWidget);
      expect(field('agents[1].name'), findsOneWidget);
    });

    testWidgets('a map of values and a map of groups', (tester) async {
      final m = await show(
        tester,
        obj({
          'env': {
            'type': 'object',
            'additionalProperties': {'type': 'string'},
          },
          'hosts': {
            'type': 'object',
            'additionalProperties': obj({
              'port': {'type': 'integer'},
            }),
          },
        }),
        value: {
          'env': {'PATH': '/bin'},
          'hosts': {
            'a': {'port': 1},
          },
        },
      );
      expect(find.byKey(const ValueKey('form-entry-env-PATH')), findsOneWidget);
      expect(field('hosts[a].port'), findsOneWidget);
      m.addToMap('env', 'HOME', '/h');
      await tester.pump();
      expect(m.read('env'), {'PATH': '/bin', 'HOME': '/h'});
    });

    testWidgets('a nested group is drawn', (tester) async {
      await show(
        tester,
        obj({
          'tls': obj({
            'ca': {'type': 'string'},
          }),
        }),
        value: {
          'tls': {'ca': 'x'},
        },
      );
      expect(field('tls.ca'), findsOneWidget);
    });

    testWidgets('a union offers its arms', (tester) async {
      await show(
        tester,
        obj({
          'auth': {
            'x-variant-tag': 'kind',
            'oneOf': [
              obj({
                'kind': {'const': 'none'},
              }),
              obj({
                'kind': {'const': 'token'},
                'token': {'type': 'string'},
              }),
            ],
          },
        }),
      );
      expect(find.byType(DropdownButtonFormField<int>), findsOneWidget);
    });
  });

  group('form', () {
    testWidgets("sections come in the form's order", (tester) async {
      await show(
        tester,
        obj({
          'a': {'type': 'string', 'x-section': 'second'},
          'b': {'type': 'string', 'x-section': 'first'},
        }),
        form: {
          'sections': [
            {'id': 'first', 'title': 'First part'},
            {'id': 'second', 'title': 'Second part'},
          ],
        },
      );
      final first = tester.getTopLeft(find.text('First part')).dy;
      final second = tester.getTopLeft(find.text('Second part')).dy;
      expect(first, lessThan(second));
    });

    testWidgets('visibleWhen shows a field when its condition holds', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'tls': {'type': 'boolean'},
          'ca': {'type': 'string'},
        }),
        form: {
          'fields': {
            'ca': {
              'visibleWhen': {'field': 'tls', 'equals': true},
            },
          },
        },
        value: {'tls': false},
      );
      expect(field('ca'), findsNothing);
      m.setValue('tls', true);
      await tester.pump();
      expect(field('ca'), findsOneWidget);
    });

    testWidgets("a provider's form is the one in its meta", (tester) async {
      final m = FormModel.forProvider(
        schema: obj({
          'user': {'type': 'string', 'x-section': 'account'},
        }),
        meta: {
          'form': {
            'sections': [
              {'id': 'account', 'title': 'Account'},
            ],
          },
        },
      );
      addTearDown(m.dispose);
      expect(m.layout().map((g) => g.title), contains('Account'));
    });

    testWidgets('problems are shown', (tester) async {
      await show(
        tester,
        obj({
          'name': {'type': 'string'},
        }),
        problems: {'name': 'Needed here'},
      );
      expect(find.text('Needed here'), findsOneWidget);
    });

    testWidgets('disabled shows every control disabled', (tester) async {
      await show(
        tester,
        obj({
          'name': {'type': 'string'},
        }),
        enabled: false,
      );
      expect(tester.widget<EditableText>(textIn('name')).readOnly, isTrue);
    });
  });

  group('secrets and read-only', () {
    testWidgets('a sensitive writable field is obscured', (tester) async {
      await show(
        tester,
        obj({
          'token': {'type': 'string', 'x-sensitive': true},
        }),
      );
      expect(tester.widget<EditableText>(textIn('token')).obscureText, isTrue);
    });

    testWidgets('a sensitive read-only field is masked, then revealed', (
      tester,
    ) async {
      await show(
        tester,
        obj({
          'token': {'type': 'string', 'x-sensitive': true, 'readOnly': true},
        }),
        value: {'token': 'hunter2'},
      );
      expect(tester.widget<EditableText>(textIn('token')).obscureText, isTrue);
      expect(find.byKey(const Key('form-copy-token')), findsOneWidget);
      await tester.tap(find.byKey(const Key('form-reveal-token')));
      await tester.pump();
      expect(tester.widget<EditableText>(textIn('token')).obscureText, isFalse);
    });

    testWidgets('readOnly is not editable', (tester) async {
      await show(
        tester,
        obj({
          'id': {'type': 'string', 'readOnly': true},
        }),
        value: {'id': 'fixed'},
      );
      expect(tester.widget<EditableText>(textIn('id')).readOnly, isTrue);
    });
  });

  group('bytes and base64', () {
    final schema = obj({
      'blob': {'type': 'bytes', 'contentMediaType': 'image/png'},
    });

    testWidgets('pick fills the field and shows its size', (tester) async {
      final m = await show(
        tester,
        schema,
        onPickBytes: (path, field) async => Uint8List.fromList([1, 2, 3]),
        onSaveBytes: (path, bytes, field) async {},
      );
      await tester.tap(find.byKey(const Key('form-pick-blob')));
      await tester.pumpAndSettle();
      expect(m.holds('blob'), isTrue);
      expect(find.byKey(const Key('form-size-blob')), findsOneWidget);
      expect(find.byKey(const Key('form-save-blob')), findsOneWidget);
      expect(find.textContaining('image/png'), findsWidgets);
    });

    testWidgets('save hands the bytes out', (tester) async {
      Uint8List? saved;
      await show(
        tester,
        schema,
        onPickBytes: (path, field) async => Uint8List.fromList([9, 8]),
        onSaveBytes: (path, bytes, field) async => saved = bytes,
      );
      await tester.tap(find.byKey(const Key('form-pick-blob')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('form-save-blob')));
      await tester.pumpAndSettle();
      expect(saved, [9, 8]);
    });

    testWidgets('a base64 string is checked', (tester) async {
      final m = await show(
        tester,
        obj({
          'data': {'type': 'string', 'contentEncoding': 'base64'},
        }),
      );
      await tester.enterText(textIn('data'), '!!not base64!!');
      await tester.pump();
      expect(m.issues, contains('data'));
      await tester.enterText(textIn('data'), 'aGVsbG8=');
      await tester.pump();
      expect(m.issues, isNot(contains('data')));
    });
  });

  group('unavailable', () {
    testWidgets('a field is disabled with its reason, value kept', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'a': {'type': 'string'},
          'b': {'type': 'string'},
        }),
        value: {'a': 'kept', 'b': 'free'},
        unavailable: {'a': 'You lack the role'},
      );
      expect(find.text('You lack the role'), findsOneWidget);
      expect(find.byKey(const Key('form-unavailable-a')), findsOneWidget);
      expect(find.byKey(const Key('form-unavailable-b')), findsNothing);
      expect(tester.widget<EditableText>(textIn('a')).readOnly, isTrue);
      expect(tester.widget<EditableText>(textIn('b')).readOnly, isFalse);
      expect(m.read('a'), 'kept');
    });

    testWidgets('a radio choice is disabled with its reason, value kept', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'mode': {
            'enum': ['a', 'b'],
          },
        }),
        form: {
          'fields': {
            'mode': {'widget': 'radio'},
          },
        },
        value: {'mode': 'a'},
        unavailableChoices: {
          'mode': {'b': 'Needs approval'},
        },
      );
      expect(find.text('Needs approval'), findsOneWidget);
      final b = tester.widget<RadioListTile<int>>(
        find.byKey(const Key('form-choice-mode-1')),
      );
      final a = tester.widget<RadioListTile<int>>(
        find.byKey(const Key('form-choice-mode-0')),
      );
      expect(b.enabled, isFalse);
      expect(a.enabled, isTrue);
      expect(m.read('mode'), 'a');
    });

    testWidgets('a dropdown choice shows its reason, value kept', (
      tester,
    ) async {
      final m = await show(
        tester,
        obj({
          'mode': {
            'enum': ['a', 'b'],
          },
        }),
        value: {'mode': 'a'},
        unavailableChoices: {
          'mode': {'b': 'Needs approval'},
        },
      );
      await tester.tap(find.byType(DropdownButtonFormField<int>).first);
      await tester.pumpAndSettle();
      expect(find.text('Needs approval'), findsOneWidget);
      expect(m.read('mode'), 'a');
    });
  });
}
