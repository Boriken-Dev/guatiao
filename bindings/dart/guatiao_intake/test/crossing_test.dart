// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A tree crosses between guatiao_intake and guatiao: built by one library,
/// read and freed through the other.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(setUpLibrary);

  test('a tree intake built is changed and freed through the core', () {
    final schema = Value.fromDart({
      'type': 'object',
      'properties': {
        'name': {'type': 'string', 'x-section': 'who'},
      },
    });
    addTearDown(schema.close);
    final form = intake.forSchema(schema);
    form.asMap()['note'] = 'added through the core';
    expect(form.asMap()['sections']!.toDart(), [
      {'id': 'who'},
    ]);
    form.close();
    expect(() => form.toDart(), throwsStateError);
  });

  test(
    'a tree the core built is read by intake, and the view refuses once it closes',
    () {
      final value = Value.fromDart({
        'agent': [
          {'name': 'a'},
          {'name': 'b'},
        ],
      });
      final found = intake.at(value, 'agent[1].name')!;
      expect(found.toDart(), 'b');
      value.close();
      expect(() => found.toDart(), throwsStateError);
    },
  );
}
