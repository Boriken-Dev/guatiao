// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:guatiao_flutter/guatiao_flutter.dart';

import 'support.dart';

// The README's example, as it is written there.
FormModel readmeModel() {
  return FormModel(
    schema: {
      'type': 'object',
      'properties': {
        'name': {'type': 'string', 'title': 'Name'},
        'retries': {'type': 'integer', 'title': 'Retries', 'minimum': 0},
        'verbose': {'type': 'boolean', 'title': 'Verbose'},
      },
    },
    value: {'name': 'demo', 'retries': 3},
  );
}

Widget readmeExample(FormModel model) {
  return MaterialApp(
    home: Scaffold(
      body: SingleChildScrollView(child: IntakeFormView(model: model)),
    ),
  );
}

void main() {
  setUpAll(setUpGuatiao);

  testWidgets('the README example draws its three fields', (tester) async {
    final model = readmeModel();
    addTearDown(model.dispose);
    await tester.pumpWidget(readmeExample(model));
    expect(find.text('Name'), findsWidgets);
    expect(find.text('Retries'), findsWidgets);
    expect(find.text('Verbose'), findsWidgets);
    expect(find.text('demo'), findsOneWidget);
    // What the README says the model answers.
    expect(model.value(), {'name': 'demo', 'retries': 3});
  });
}
