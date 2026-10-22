// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A tree crosses between guatiao_intake and guatiao: built by one library,
/// read and freed through the other; and a provider's form, read through
/// the core's registry, checked here.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

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
  });

  test('the library is intake\'s own, and carries the core too', () {
    final lib = intake.library();
    expect(lib.path, endsWith(dllFilename('guatiao_intake')));
    expect(lib.provides('guatiao_value_free'), isTrue);
  });

  test('a provider carries its form in meta, which fits its schema', () {
    final reg = Registry('dart-tests', '0.1');
    addTearDown(reg.close);
    reg.scanDir(libraryDir(), rules: 'kind=greeter');
    final meta = reg.providerMeta('derived_greeter_shouter');
    expect(meta, isNotNull);
    addTearDown(meta!.close);
    final form = meta.asMap()[intake.providerFormKey];
    final schema = reg.providerConfig('derived_greeter_shouter');
    expect(form, isNotNull);
    expect(schema, isNotNull);

    expect(intake.check(schema!, form!), isNull);
    final sections = form.toDart()! as Map;
    expect(
      [for (final s in sections['sections']! as List) (s as Map)['id']],
      ['words', 'volume'],
    );
    final marks = (sections['fields']! as Map)['marks']! as Map;
    expect(marks['unit'], 'marks');
    expect(
      ((sections['fields']! as Map)['prefix']! as Map)['placeholder'],
      'hey',
    );
    for (final exclaim in [false, true]) {
      final values = Value.fromDart({'exclaim': exclaim});
      addTearDown(values.close);
      expect(intake.isVisible(schema, form, 'marks', values), exclaim);
    }
  });
}
