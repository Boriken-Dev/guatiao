// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// What `dart run guatiao:kind` writes: the committed files are what the
/// committed description and pubspec write, a document it cannot write is
/// refused naming where, and the analyzer refuses a provider class that
/// lacks or mistypes a slot of the kind it implements.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';

import 'package:guatiao/src/kind_generator.dart';
import 'package:test/test.dart';

import 'support.dart';

Map<String, Object?> _document() =>
    (jsonDecode(
              File(
                repoPath([
                  'examples',
                  'greeter_kind',
                  'include',
                  'greeter_kind.kinds.json',
                ]),
              ).readAsStringSync(),
            )
            as Map)
        .cast<String, Object?>();

Map<String, Object?> _one(
  List<Object?> slots, [
  List<Object?> more = const [],
]) => {
  'format': 1,
  'kinds': [
    {'name': 'k', 'floor_hash': 1, 'object': false, 'slots': slots},
    ...more,
  ],
};

void main() {
  test('the greeter kinds are what their description writes', () {
    final committed = File(
      repoPath([
        'examples',
        'greeter_kind',
        'dart',
        'lib',
        'greeter_kind.dart',
      ]),
    ).readAsStringSync();
    expect(
      generateKinds(
        _document(),
        source: '../../include/greeter_kind.kinds.json',
        command:
            'dart run guatiao:kind ../include/greeter_kind.kinds.json -o '
            'lib/greeter_kind.dart',
      ),
      committed,
      reason:
          'regenerate: in examples/greeter_kind/dart, dart run guatiao:kind '
          '../include/greeter_kind.kinds.json -o lib/greeter_kind.dart',
    );
  });

  test("the provider example's identity is what its pubspec writes", () {
    final dir = ['examples', 'dart_provider', 'dart_greeter'];
    expect(
      generateIdentity(
        File(repoPath([...dir, 'pubspec.yaml'])).readAsStringSync(),
        source: '../../pubspec.yaml',
        command: 'dart run guatiao:kind identity',
      ),
      File(
        repoPath([...dir, 'lib', 'src', 'library_identity.g.dart']),
      ).readAsStringSync(),
    );
  });

  test('names follow Dart', () {
    expect(typeNameOf('session.backend_v2'), 'SessionBackendV2');
    expect(typeNameOf('2fa'), 'Kind2fa');
    expect(memberNameOf('greet_into'), 'greetInto');
    final text = generateKinds(
      _one([
        {
          'name': 'open',
          'required': true,
          'args': [
            {'name': 'default', 'type': 'u8'},
            {'name': 'out', 'type': 'str'},
          ],
        },
      ]),
    );
    expect(text, contains('void open(int default_, String out_);'));
  });

  test('a document it cannot write is refused, naming where', () {
    Matcher refusal(String text) => throwsA(
      isA<FormatException>().having(
        (e) => e.message,
        'message',
        contains(text),
      ),
    );
    expect(
      () => generateKinds({'format': 2, 'kinds': []}),
      refusal('"format": 1'),
    );
    expect(
      () => generateKinds(
        _one([
          {
            'name': 'x',
            'args': [
              {'type': 'u128'},
            ],
          },
        ]),
      ),
      refusal('k.x argument 0: u128'),
    );
    expect(
      () => generateKinds(
        _one([
          {
            'name': 'x',
            'args': [
              {'type': 'object', 'kind': 'nowhere', 'pass': 'lent'},
            ],
          },
        ]),
      ),
      refusal('the object kind nowhere is not in this document'),
    );
    expect(
      () => generateKinds(
        _one([
          {'name': 'has'},
        ]),
      ),
      refusal('k.has: has is taken in Dart'),
    );
    expect(
      () => generateKinds(
        _one([
          {'name': 'a_b'},
          {'name': 'aB'},
        ]),
      ),
      refusal('k.aB: aB is taken'),
    );
    expect(
      () => generateKinds({
        'format': 1,
        'kinds': [
          {'name': 'a', 'floor_hash': 1, 'slots': []},
          {'name': 'A', 'floor_hash': 2, 'slots': []},
        ],
      }),
      refusal('two kinds are A'),
    );
  });

  test('a pubspec without a version names no library', () {
    expect(
      () => generateIdentity('name: x\n'),
      throwsA(isA<FormatException>()),
    );
    final text = generateIdentity(
      'name: x\nversion: 2.0.0 # pinned\ndescription: >-\n  two\n  lines\n'
      "homepage: 'https://example.org'\n",
    );
    expect(text, contains("'x',\n  '2.0.0',"));
    expect(text, contains("'summary': 'two lines',"));
    expect(text, contains("'Homepage': 'https://example.org',"));
  });

  test('a kind imports the object kinds another document describes, '
      'and a kind may name itself', () {
    final whole = _document();
    final kinds = (whole['kinds']! as List).cast<Map<String, Object?>>();
    final objects = {
      'format': 1,
      'kinds': [
        for (final k in kinds)
          if (k['object'] == true) k,
      ],
    };
    final greeters = {
      'format': 1,
      'kinds': [
        for (final k in kinds)
          if (k['object'] != true) k,
      ],
    };
    expect(() => generateKinds(greeters), throwsA(isA<FormatException>()));
    final imported = generateKinds(
      greeters,
      imports: {'listener': 'objects.dart', 'conversation': 'objects.dart'},
    );
    expect(imported, contains("import 'objects.dart';"));
    expect(imported, isNot(contains('abstract interface class Listener ')));
    final trees = {
      'format': 1,
      'kinds': [
        {
          'name': 'node',
          'floor_hash': 5,
          'object': true,
          'slots': [
            {
              'name': 'child',
              'required': true,
              'fallible': true,
              'args': [
                {'name': 'label', 'type': 'str'},
              ],
              'answer': {'type': 'object', 'kind': 'node'},
            },
            {
              'name': 'size_of',
              'required': false,
              'fallible': false,
              'args': [
                {
                  'name': 'other',
                  'type': 'object',
                  'kind': 'node',
                  'pass': 'lent',
                },
              ],
              'answer': {'type': 'u32'},
            },
          ],
        },
      ],
    };
    final dir = Directory.systemTemp.createTempSync('guatiao-imports-');
    addTearDown(() => dir.deleteSync(recursive: true));
    String posix(String path) => path.replaceAll(r'\', '/');
    File('${dir.path}/pubspec.yaml').writeAsStringSync(
      'name: split\npublish_to: none\nenvironment:\n  sdk: ^3.10.0\n'
      'dependencies:\n'
      '  guatiao:\n    path: ${posix(repoPath(['bindings', 'dart', 'guatiao']))}\n',
    );
    Directory('${dir.path}/lib').createSync();
    File(
      '${dir.path}/lib/objects.dart',
    ).writeAsStringSync(generateKinds(objects));
    File('${dir.path}/lib/greeters.dart').writeAsStringSync(imported);
    File('${dir.path}/lib/trees.dart').writeAsStringSync(generateKinds(trees));
    final dart = Platform.resolvedExecutable;
    final get = Process.runSync(dart, [
      'pub',
      'get',
      '--offline',
    ], workingDirectory: dir.path);
    expect(get.exitCode, 0, reason: '${get.stdout}${get.stderr}');
    final analyze = Process.runSync(dart, [
      'analyze',
      '--format=machine',
    ], workingDirectory: dir.path);
    final errors = [
      for (final line in LineSplitter.split(
        '${analyze.stdout}${analyze.stderr}',
      ))
        if (line.startsWith('ERROR|')) line,
    ];
    expect(errors, isEmpty, reason: errors.join('\n'));
  });

  test('the analyzer refuses a provider lacking or mistyping a slot', () {
    final dir = Directory.systemTemp.createTempSync('guatiao-analyzer-');
    addTearDown(() => dir.deleteSync(recursive: true));
    String posix(String path) => path.replaceAll(r'\', '/');
    File('${dir.path}/pubspec.yaml').writeAsStringSync(
      'name: cases\npublish_to: none\nenvironment:\n  sdk: ^3.10.0\n'
      'dependencies:\n'
      '  guatiao:\n    path: ${posix(repoPath(['bindings', 'dart', 'guatiao']))}\n'
      '  greeter_kind:\n    path: ${posix(repoPath(['examples', 'greeter_kind', 'dart']))}\n',
    );
    Directory('${dir.path}/lib').createSync();
    File('${dir.path}/lib/cases.dart').writeAsStringSync('''
import 'package:greeter_kind/greeter_kind.dart';
import 'package:guatiao/kinds.dart';

class Missing extends Provider implements Greeter {}

class WrongArgument extends Provider implements Greeter {
  @override
  Object greet(int name) => name;
}

class WrongOptional extends Provider implements Greeter, GreeterShout {
  @override
  Object greet(String name) => name;
  @override
  int shout(String name) => 1;
}

class WrongAnswer extends Provider implements Counter {
  @override
  String count() => '1';
}

class Fine extends Provider implements Greeter, GreeterShout {
  @override
  Map<String, Object?> greet(String name) => {'greeting': name};
  @override
  String shout(String name) => name;
}
''');
    final dart = Platform.resolvedExecutable;
    final get = Process.runSync(dart, [
      'pub',
      'get',
      '--offline',
    ], workingDirectory: dir.path);
    expect(get.exitCode, 0, reason: '${get.stdout}${get.stderr}');
    final analyze = Process.runSync(dart, [
      'analyze',
      '--format=machine',
    ], workingDirectory: dir.path);
    final errors = [
      for (final line in LineSplitter.split(
        '${analyze.stdout}${analyze.stderr}',
      ))
        if (line.startsWith('ERROR|')) line,
    ];
    String at(int line) => errors.firstWhere(
      (e) => e.contains('cases.dart|$line|'),
      orElse: () => 'none at $line',
    );
    expect(at(4), contains('NON_ABSTRACT_CLASS_INHERITS_ABSTRACT_MEMBER'));
    expect(at(8), contains('INVALID_OVERRIDE'));
    expect(at(15), contains('INVALID_OVERRIDE'));
    expect(at(20), contains('INVALID_OVERRIDE'));
    expect(errors, hasLength(4), reason: errors.join('\n'));
  });
}
