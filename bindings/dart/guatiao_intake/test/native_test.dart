// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What only a native build has: the library file intake opened, a
/// provider's form read through the core's registry, and the constant
/// against the generated bindings.
@TestOn('vm')
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
import 'package:guatiao_intake/src/bindings.g.dart'
    show GUATIAO_INTAKE_PROVIDER_FORM_KEY;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(libraryDir);

  test('the library is intake\'s own, and carries the core too', () {
    final lib = intake.library();
    expect(
      lib.path,
      namedLibraryFile() ?? endsWith(dllFilename('guatiao_intake')),
    );
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

  test('a catalog entry gives each provider its form', () {
    final reg = Registry('dart-tests', '0.1');
    addTearDown(reg.close);
    reg.scanDir(libraryDir(), rules: 'kind=greeter');
    final catalog = {for (final entry in reg.catalog()) entry['id']: entry};

    final shouter = catalog['derived_greeter_shouter']!;
    final form = intake.providerForm(shouter['config'], shouter['meta']);
    expect(form, isNotNull);
    addTearDown(form!.close);
    expect(
      [
        for (final s in (form.toDart()! as Map)['sections'] as List)
          (s as Map)['id'],
      ],
      ['words', 'volume'],
    );
    final schema = Value.fromDart(shouter['config']);
    addTearDown(schema.close);
    expect(intake.check(schema, form), isNull);

    final hello = catalog['derived_greeter_hello']!;
    expect(intake.providerForm(hello['config'], hello['meta']), isNull);
  });

  test('the form key is the header\'s', () {
    expect(intake.providerFormKey, GUATIAO_INTAKE_PROVIDER_FORM_KEY);
  });
}
