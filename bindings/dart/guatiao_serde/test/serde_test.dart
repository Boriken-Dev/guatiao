// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// `guatiao_serde` against a real `--all-features` build, or the module.
library;

import 'package:guatiao/guatiao.dart';
import 'package:guatiao_serde/guatiao_serde.dart' as serde;
import 'package:guatiao_serde/guatiao_serde.dart' show Format, ParseError;
import 'package:test/test.dart';

import 'support.dart';

void main() {
  setUpAll(setUpLibrary);

  test('a JSON round trip keeps key order and integers', () {
    final doc = Value.fromDart({
      'z': 3,
      'a': 1,
      'list': [1, 'two', null, true],
    });
    addTearDown(doc.close);
    final parsed = serde.loads(serde.dumps(doc));
    addTearDown(parsed.close);
    final map = parsed.asMap();
    expect(map.keys, ['z', 'a', 'list']);
    expect(map['z']!.toDart(), 3);
    expect(map['list']!.toDart(), [1, 'two', null, true]);
  });

  test('a number keeps its exact text through JSON', () {
    final doc = Value.fromDart({'n': Value.number('1.10')});
    addTearDown(doc.close);
    final parsed = serde.loads(serde.dumps(doc));
    addTearDown(parsed.close);
    expect(parsed.asMap()['n']!.toDart(numbers: Numbers.text), '1.10');
  });

  test('pretty JSON is longer than compact', () {
    final doc = Value.fromDart({'a': 1});
    addTearDown(doc.close);
    expect(
      serde.dumps(doc, pretty: true).length,
      greaterThan(serde.dumps(doc).length),
    );
  });

  test('a TOML round trip', () {
    final doc = Value.fromDart({'a': 1, 'b': 'x'});
    addTearDown(doc.close);
    final parsed = serde.loads(
      serde.dumps(doc, format: Format.toml),
      format: Format.toml,
    );
    addTearDown(parsed.close);
    expect(parsed.toDart(), {'a': 1, 'b': 'x'});
  });

  test('a YAML round trip', () {
    final doc = Value.fromDart({'a': 1, 'b': 'x'});
    addTearDown(doc.close);
    final parsed = serde.loads(
      serde.dumps(doc, format: Format.yaml),
      format: Format.yaml,
    );
    addTearDown(parsed.close);
    expect(parsed.toDart(), {'a': 1, 'b': 'x'});
  });

  test('a TOML document must be a map', () {
    final doc = Value.fromDart([1, 2]);
    addTearDown(doc.close);
    expect(
      () => serde.dumps(doc, format: Format.toml),
      throwsA(isA<WrongKind>()),
    );
  });

  test('parsing text that is not the format fails', () {
    expect(() => serde.loads('{not json'), throwsA(isA<BadValue>()));
  });

  const documents = {
    Format.json: [
      '{"z": 3, "a": [1.5, true, null, "x", {}, []], "n": 1.10, "e": 1e400}',
      '[1, -0, 12345678901234567890123, 2.5e-3]',
      '"text"',
      '17',
      '{"blob": "data:;base64,AAEC", "deep": {"a": {"b": {"c": [[[]]]}}}}',
    ],
    Format.toml: ['a = 1\nb = "x"\n[t]\nc = [1.5, true]\n[[arr]]\nd = 2\n'],
    Format.yaml: ['a: 1\nb: [x, 2.5, null]\nc:\n  d: true\n', '- 1\n- two\n'],
  };

  test('parse answers what loads then toDart answers', () {
    for (final MapEntry(key: format, value: texts) in documents.entries) {
      for (final text in texts) {
        for (final how in [0, serde.readDataUris]) {
          for (final numbers in Numbers.values) {
            final value = serde.loads(text, format: format, how: how);
            final expected = value.toDart(numbers: numbers);
            value.close();
            final got = serde.parse(
              text,
              format: format,
              how: how,
              numbers: numbers,
            );
            expect(got, expected, reason: text);
            expect(got.toString(), expected.toString(), reason: text);
          }
        }
      }
    }
  });

  test('render writes what dumps of fromDart writes', () {
    for (final MapEntry(key: format, value: texts) in documents.entries) {
      // No Dart number holds 1e400.
      for (final text in texts.where((t) => !t.contains('1e400'))) {
        final obj = serde.parse(
          text,
          format: format,
          how: serde.readDataUris,
          numbers: Numbers.bigInt,
        );
        final value = Value.fromDart(obj);
        for (final pretty in [false, true]) {
          for (final how in [
            serde.bytesDataUri,
            serde.bytesBase64,
            serde.bytesArray,
          ]) {
            expect(
              serde.render(obj, format: format, pretty: pretty, how: how),
              serde.dumps(value, format: format, pretty: pretty, how: how),
              reason: text,
            );
          }
        }
        value.close();
      }
    }
  });

  test('parse and render refuse as loads and dumps do', () {
    expect(
      () => serde.parse('{"a":\n 1,,}'),
      throwsA(
        isA<ParseError>()
            .having((e) => e.line, 'line', 2)
            .having((e) => e.column, 'column', 4),
      ),
    );
    expect(
      () => serde.parse('a = = 1', format: Format.toml),
      throwsA(isA<ParseError>().having((e) => e.format, 'format', Format.toml)),
    );
    expect(
      () => serde.render([1, 2], format: Format.toml),
      throwsA(isA<WrongKind>()),
    );
    expect(() => serde.render(double.infinity), throwsArgumentError);
    expect(() => serde.render({1: 'x'}), throwsArgumentError);
  });

  test('a malformed document carries its format and position', () {
    // Both catches find it: it is what a malformed document threw before,
    // and a FormatException.
    expect(
      () => serde.loads('{"a":\n 1,,}'),
      throwsA(
        isA<ParseError>()
            .having((e) => e, 'a BadValue', isA<BadValue>())
            .having((e) => e, 'a FormatException', isA<FormatException>())
            .having((e) => e.format, 'format', Format.json)
            .having((e) => e.line, 'line', 2)
            .having((e) => e.column, 'column', 4)
            .having((e) => e.message, 'message', isNotEmpty),
      ),
    );
    expect(
      () => serde.loads('a = = 1', format: Format.toml),
      throwsA(
        isA<ParseError>()
            .having((e) => e.format, 'format', Format.toml)
            .having((e) => e.line, 'line', 1)
            .having((e) => e.column, 'column', 5),
      ),
    );
    expect(
      () => serde.loads('a: [1', format: Format.yaml),
      throwsA(
        isA<ParseError>()
            .having((e) => e.format, 'format', Format.yaml)
            .having((e) => e.line, 'line', 1),
      ),
    );
  });
}
