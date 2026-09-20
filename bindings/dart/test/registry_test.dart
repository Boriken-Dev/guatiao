// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `Registry` against a real `--all-features` build.
library;

import 'dart:ffi';

import 'package:guatiao/guatiao.dart';
import 'package:test/test.dart';

import 'support.dart';

Registry scanned() {
  final reg = Registry('dart-tests', '0.1')
    ..scanDir(libraryDir(), rules: 'kind=greeter');
  return reg;
}

Set<Object?> providerIds(Registry reg) =>
    reg.providers('greeter').map((p) => p['id']).toSet();

bool hasLibrary(Registry reg, String id) =>
    reg.libraries().any((lib) => lib['id'] == id);

void main() {
  setUpAll(libraryDir);

  test('a scan under kind=greeter finds the example providers', () {
    final reg = Registry('dart-tests', '0.1');
    addTearDown(reg.close);
    final report = reg.scanDir(libraryDir(), rules: 'kind=greeter');
    final loaded = (report['loaded']! as List).cast<String>();
    expect(loaded.any((p) => p.contains('derived_greeter')), isTrue);
    expect(loaded.any((p) => p.contains('hello_library')), isTrue);
    expect(
      providerIds(reg),
      containsAll(<String>['derived_greeter_hello', 'hello_library_greeter']),
    );
  });

  test('best carries the documented fields', () {
    final reg = scanned();
    addTearDown(reg.close);
    final best = reg.best('greeter');
    for (final field in [
      'key',
      'id',
      'version',
      'library',
      'display_name',
      'from',
      'kinds',
      'has_config',
      'vtable_size',
    ]) {
      expect(best, contains(field), reason: field);
    }
    expect(best['kinds'], contains('greeter'));
  });

  test('whyNot keeps the two refusals apart', () {
    final reg = scanned();
    addTearDown(reg.close);
    final why = reg.whyNot('nothing-claims-this-kind');
    expect(why['available'], isFalse);
    expect(why['why'], 'nothing-claims-it');
  });

  test('available and provider answer as plain Dart', () {
    final reg = scanned();
    addTearDown(reg.close);
    expect(reg.available('greeter'), isNotEmpty);
    expect(
        reg.provider('derived_greeter_hello')['id'], 'derived_greeter_hello');
    expect(reg.libraries(), isNotEmpty);
  });

  test('a priority is zero until this host says otherwise', () {
    final reg = scanned();
    addTearDown(reg.close);
    expect(reg.priority('derived_greeter_hello'), 0);
    reg.setPriority('derived_greeter_hello', 5);
    expect(reg.priority('derived_greeter_hello'), 5);
  });

  test('a provider with no per-kind table has no single table', () {
    final reg = scanned();
    addTearDown(reg.close);
    expect(reg.providerTable('hello_library_greeter').isEmpty, isFalse);
    expect(reg.providerTable('hello_library_greeter').size, greaterThan(0));
    expect(reg.providerTable('derived_greeter_hello').isEmpty, isTrue);
    expect(
      reg.providerTable('derived_greeter_hello', kind: 'codec').isEmpty,
      isTrue,
    );
  });

  test('create builds an instance and closing it releases it', () {
    final reg = scanned();
    addTearDown(reg.close);
    final instance = reg.create('derived_greeter_shouter', {'prefix': 'hey'});
    expect(instance.ctx, isNot(equals(nullptr)));
    instance.close();
    expect(instance.isClosed, isTrue);
    instance.close();
  });

  test('a provider config is a borrowed schema that refuses a write', () {
    final reg = scanned();
    addTearDown(reg.close);
    final schema = reg.providerConfig('derived_greeter_shouter');
    expect(schema, isNotNull);
    expect(schema!.tag, Tag.map);
    expect(() => schema.asMap()['x'] = 1, throwsStateError);
    expect(reg.providerConfig('derived_greeter_hello'), isNull);
  });

  test('retire frees the key and leaves the other library alone', () {
    final reg = scanned();
    addTearDown(reg.close);
    expect(providerIds(reg), contains('derived_greeter_hello'));

    reg.retire('derived_greeter');
    expect(providerIds(reg), isNot(contains('derived_greeter_hello')));
    expect(providerIds(reg), contains('hello_library_greeter'));
    expect(hasLibrary(reg, 'derived_greeter'), isFalse);

    final report = reg.scanDir(libraryDir(), rules: 'kind=greeter');
    final loaded = (report['loaded']! as List).cast<String>();
    expect(loaded.any((p) => p.contains('derived_greeter')), isTrue);
  });

  test('retire takes the library key, not a provider key', () {
    final reg = scanned();
    addTearDown(reg.close);
    expect(() => reg.retire('derived_greeter_hello'), throwsA(isA<NotFound>()));
    expect(hasLibrary(reg, 'derived_greeter'), isTrue);
  });

  test('a library refuses to unload while an instance is alive', () {
    final reg = scanned();
    addTearDown(reg.close);
    final instance = reg.create('derived_greeter_shouter', {'prefix': 'hey'});
    expect(() => reg.unload('derived_greeter'), throwsA(isA<Busy>()));
    expect(hasLibrary(reg, 'derived_greeter'), isTrue);
    instance.close();
    reg.unload('derived_greeter');
    expect(hasLibrary(reg, 'derived_greeter'), isFalse);
    expect(() => reg.unload('derived_greeter'), throwsA(isA<NotFound>()));
  });

  test('a closed registry throws on use, and closes again quietly', () {
    final reg = Registry('dart-tests', '0.1');
    reg.close();
    expect(reg.libraries, throwsStateError);
    expect(reg.isClosed, isTrue);
    reg.close();
  });
}
