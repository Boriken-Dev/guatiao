// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:guatiao_flutter/guatiao_flutter.dart';

export 'support.dart';

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
  ChoicesFor? choicesFor,
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
            choicesFor: choicesFor,
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
