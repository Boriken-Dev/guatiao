// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Kinds declared in Dart (`package:guatiao/describe.dart`) read into the
/// description document: byte for byte what Rust writes for the greeter
/// kinds and TypeScript for the tally kinds, the floor hash Rust computes,
/// and a declaration it cannot read refused naming the place.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';

import 'package:guatiao/describe.dart';
import 'package:guatiao/src/kind_describer.dart'
    show describedText, floorHashOf;
import 'package:test/test.dart';

import 'support.dart';

String _read(List<String> path) => File(repoPath(path)).readAsStringSync();

String _fixture(String name) =>
    _read(['bindings', 'dart', 'guatiao', 'test', 'fixtures', name]);

const _head = '''
import 'package:guatiao/describe.dart';
import 'package:guatiao/guatiao.dart' show MapRef, Ref;
''';

Map<String, Object?> _describe(String body) =>
    describeDeclarations('$_head$body', file: 'kinds.dart');

void main() {
  test('the greeter kinds declared in Dart describe as the trait does', () {
    expect(
      describedText(
        describeDeclarations(_fixture('greeter_declarations.dart')),
      ),
      _read(['examples', 'greeter_kind', 'include', 'greeter_kind.kinds.json']),
    );
  });

  test('the tally kinds declared in Dart describe as TypeScript does', () {
    expect(
      describedText(describeDeclarations(_fixture('tally_declarations.dart'))),
      _read(['examples', 'tally_kind', 'include', 'tally_kind.kinds.json']),
    );
  });

  test('the floor hash is the one Rust gives the shared vectors', () {
    final file =
        jsonDecode(
              _read([
                'crates',
                'guatiao-derive',
                'tests',
                'vectors',
                'floor_hash.json',
              ]),
            )
            as Map;
    final vectors = (file['vectors'] as List).cast<Map>();
    expect(vectors.length, greaterThanOrEqualTo(6));
    for (final v in vectors) {
      expect(
        floorHashOf(v['object'] as bool, [
          for (final s in v['slots'] as List)
            (s as Map).cast<String, Object?>(),
        ]),
        v['floor_hash'],
        reason: v['name'] as String,
      );
    }
  });

  test('a name, a pinned hash, nullable and lent trees, import prefixes', () {
    final doc = describeDeclarations('''
import 'package:guatiao/describe.dart' as k;
import 'package:guatiao/guatiao.dart' as g;

/// Kept apart.
@k.KindName('store.v2')
@k.FloorHash(4242)
abstract interface class Store {
  /* A comment that is no doc. */
  /// Reads.
  ///
  /// Twice.
  k.ValueAnswer read(g.Ref? v, k.Lent<g.ListRef> items, k.Usize n);
}
''');
    expect(doc, {
      'kinds': [
        {
          'name': 'store.v2',
          'floor_hash': 4242,
          'object': false,
          'doc': 'Kept apart.',
          'slots': [
            {
              'name': 'read',
              'required': true,
              'fallible': false,
              'args': [
                {
                  'name': 'v',
                  'type': 'value',
                  'pass': 'borrowed',
                  'nullable': true,
                },
                {'name': 'items', 'type': 'list', 'pass': 'lent'},
                {'name': 'n', 'type': 'usize'},
              ],
              'answer': {'type': 'value'},
              'doc': 'Reads.\n\nTwice.',
            },
          ],
        },
      ],
    });
  });

  test('docs as Rust writes them, wherever the annotations stand', () {
    final doc = describeDeclarations(
      '\uFEFF/// The library.\n'
      'library;\n'
      '$_head\n'
      '/// Hard break.  \n'
      '///\t\n'
      '/// After.\n'
      '@KindName(\'k\')\n'
      '/// More.\n'
      'abstract interface class K {\n'
      '  @fallible\n'
      '  /// Below the mark.\n'
      '  void f();\n'
      '}\n',
    );
    final kind = (doc['kinds'] as List).single as Map;
    expect(kind['doc'], 'Hard break.\n\nAfter.\nMore.');
    expect(((kind['slots'] as List).single as Map)['doc'], 'Below the mark.');
  });

  test('a declaration it cannot read is refused, naming the place', () {
    const cases = {
      'class Plain {}': 'declares abstract interface classes',
      'abstract interface class A extends B {}': 'A extends nothing',
      'abstract interface class A implements Other {}':
          'implements ObjectKind or nothing',
      'abstract interface class A { int get x; }': 'methods alone',
      'abstract interface class A { void f([int x]); }':
          'not optional or named',
      'abstract interface class A { void f() {} }': 'declared, not defined',
      'abstract interface class A { void f(ListRef l); }':
          'a list is not borrowed',
      'abstract interface class A { void f(Value v); }': 'Taken<Ref>',
      'abstract interface class A { void f(B b); }\nabstract interface class B implements ObjectKind {}':
          'an object is passed Taken<B> or Lent<B>',
      'abstract interface class A { void f(Lent<B> b); }\nabstract interface class B {}':
          'B is a kind but no object kind',
      'abstract interface class A { void f(Lent<Other> b); }':
          'Other is no tree view or kind',
      'abstract interface class A { Object f(); }':
          'not a type a slot can answer',
      'abstract interface class A { String? f(); }':
          'an answer is not nullable',
      'abstract interface class A { void f(MapRef? m); }':
          'only a Ref may be null',
      'abstract interface class A { void parseHTTP(); }': 'capitals in a row',
      'abstract interface class A { @falible void f(); }':
          '@falible is not a mark of a slot',
      'abstract interface class A { @appended void f(); void g(); }':
          'the required slots come first',
      '@FloorHash(-1) abstract interface class A {}':
          '"-" is not part of a kind declaration',
      "@FloorHash('x') abstract interface class A {}":
          '@FloorHash takes a 32-bit unsigned integer',
      'abstract interface class A {}\nabstract interface class A {}':
          'two kinds are A',
      "abstract interface class A { void f(String s = 'x'); }":
          '")" was expected',
      'abstract interface class A { void f(Map<String, int> m); }':
          'not a type an argument can have',
      'abstract interface class A { void f(); void f(); }':
          'two slots of A are f',
      'abstract interface class A { void greetInto(); void greet_into(); }':
          'no slot\'s name comes back',
      'abstract interface class A { void f(String a, String a); }':
          'two arguments are a',
      "@KindName('a') @KindName('b') abstract interface class A {}":
          '@KindName is written twice',
      'abstract interface class A implements ObjectKind { void destroy(); }':
          'destroy is the slot',
      'abstract interface class A { void f(BytesMut b); }':
          'on an object kind only',
      'abstract interface class A { void close(); }':
          'a remote answers close itself',
      'abstract interface class A { void _hidden(); }':
          'no slot\'s name comes back',
      'abstract interface class Ref {}': 'one of the marks',
      'abstract interface class A { ValueAnswer? f(); }':
          'an answer is not nullable',
      'abstract interface class A { void f(@fallible String s); }':
          'an argument carries no annotation',
      'abstract interface class A {}\n@fallible': 'an annotation on nothing',
      'abstract interface class Watcher {}\nabstract interface class WatcherKind {}':
          'would not be written as Dart',
      'abstract interface class HTTPServer {}': 'capitals in a row',
    };
    for (final MapEntry(key: body, value: says) in cases.entries) {
      expect(
        () => _describe(body),
        throwsA(
          isA<FormatException>().having(
            (e) => e.message,
            'message',
            allOf(contains(says), startsWith('kinds.dart:')),
          ),
        ),
        reason: body,
      );
    }
  });

  test('describe writes the document and checks it', () async {
    final dir = Directory.systemTemp.createTempSync('guatiao-describe-');
    addTearDown(() => dir.deleteSync(recursive: true));
    final out = '${dir.path}${Platform.pathSeparator}tally.kinds.json';
    final source = repoPath([
      'bindings',
      'dart',
      'guatiao',
      'test',
      'fixtures',
      'tally_declarations.dart',
    ]);
    Future<ProcessResult> run(List<String> more) => Process.run(
      Platform.resolvedExecutable,
      ['run', 'guatiao:kind', 'describe', source, '-o', out, ...more],
      workingDirectory: repoPath(['bindings', 'dart', 'guatiao']),
    );
    final wrote = await run(const []);
    expect(wrote.exitCode, 0, reason: '${wrote.stderr}');
    expect(
      File(out).readAsStringSync(),
      _read(['examples', 'tally_kind', 'include', 'tally_kind.kinds.json']),
    );
    expect((await run(const ['--check'])).exitCode, 0);
    File(out).writeAsStringSync('{}\n');
    final stale = await run(const ['--check']);
    expect(stale.exitCode, 1);
    expect('${stale.stderr}', contains('is not what it would write'));
  });
}
