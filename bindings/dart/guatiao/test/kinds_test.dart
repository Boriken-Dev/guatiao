// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Kinds and providers as classes: a Dart host calls the greeters written
/// in Rust, C and Python through the classes `dart run guatiao:kind`
/// wrote, and a greeter written in Dart is called from Dart and from a
/// Rust caller.
@TestOn('vm')
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';
import 'package:guatiao/ffi.dart'
    show checkStatus, guatiao_host_info, guatiao_str, guatiao_value, strOf;
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kinds.dart';
import 'package:test/test.dart';

// The generated kinds of the example package, which this package cannot
// depend on: it depends on this one.
// ignore: avoid_relative_lib_imports
import '../../../../examples/greeter_kind/dart/lib/greeter_kind.dart';
import 'support.dart';

final class Ear implements Listener {
  final List<String> heardSoFar = <String>[];

  @override
  void heard(String what) => heardSoFar.add(what);
}

final class Chat implements Conversation, Disposable {
  Chat(this._listener);

  final ListenerRemote _listener;
  final List<String> _said = <String>[];
  static int disposed = 0;

  @override
  void dispose() {
    disposed++;
    _listener.close();
  }

  @override
  void say(String what) {
    if (what.isEmpty) throw BadValue(Status.badValue.code, 'nothing to say');
    _said.add(what);
    _listener.heard(what);
  }

  @override
  int read(Uint8List dst) {
    final text = utf8.encode(_said.join('\n'));
    final n = text.length < dst.length ? text.length : dst.length;
    dst.setRange(0, n, text);
    return n;
  }

  @override
  int turns() => _said.length;
}

final class Hello extends Provider
    implements
        Greeter,
        GreeterShout,
        GreeterStart,
        GreeterGreetInto,
        GreeterGreetFrom,
        GreeterCheck,
        Counter {
  int asked = 0;

  @override
  Map<String, Object?> greet(String name) {
    if (name == 'panic') throw StateError('a provider bug, on request');
    if (name.isEmpty) throw BadValue(Status.badValue.code, 'nobody to greet');
    return {'greeting': 'hello, $name'};
  }

  @override
  String shout(String name) =>
      (greet(name)['greeting']! as String).toUpperCase();

  @override
  Conversation start(ListenerRemote listener) => Chat(listener);

  @override
  void greetInto(String name, ListenerRemote listener) =>
      listener.heard(greet(name)['greeting']! as String);

  @override
  Object greetFrom(Value config) {
    try {
      return greet(config.asMap()['name']?.stringOr('') ?? '');
    } finally {
      config.close();
    }
  }

  @override
  bool check(MapRef config) {
    final name = config['name']?.stringOr('world') ?? 'world';
    config['greeting'] = 'hello, $name';
    return name.isNotEmpty;
  }

  @override
  int count() => ++asked;
}

final class Echo extends Provider implements Wide {
  @override
  int echoI64(int x) => x;

  @override
  int echoU64(int x) => x;
}

final class Shouter extends Provider implements Greeter {
  Shouter(Map<String, Object?> config) : prefix = config['prefix']! as String;

  final String prefix;
  static int disposed = 0;

  @override
  void dispose() => disposed++;

  @override
  Map<String, Object?> greet(String name) => {
    'greeting': '$prefix ${name.toUpperCase()}',
  };
}

/// A provider that serves the greeter kind and lacks its required slot.
final class Silent extends Provider {}

/// The directory holding the greeter written in C, or null without a C
/// compiler. Built once, on first use. It stays in the system's temporary
/// directory: Windows refuses to delete a library that is still mapped,
/// and a loaded one can stay mapped until the process ends.
final String? cGreeterDir = _buildCGreeter();

/// Compiles with `CC` where it is set, as the other suites do: the default
/// compiler builds for the machine, which is not this process's
/// architecture under emulation. Else the first of clang and cc found.
String? _buildCGreeter() {
  final named = Platform.environment['CC'] ?? '';
  final compilers = named.isNotEmpty
      ? [named]
      : ['clang', r'C:\Program Files\LLVM\bin\clang.exe', 'cc'];
  final out = Directory.systemTemp.createTempSync('guatiao-dart-c-');
  for (final cc in compilers) {
    try {
      final result = Process.runSync(cc, [
        '-shared',
        '-std=c11',
        if (!Platform.isWindows) '-fPIC',
        '-I',
        repoPath(['crates', 'guatiao', 'include']),
        '-I',
        repoPath(['examples', 'greeter_kind', 'include']),
        '-o',
        '${out.path}/${dllFilename('c_greeter')}',
        repoPath([
          'crates',
          'guatiao',
          'tests',
          'c_consumer',
          'greeter_in_c.c',
        ]),
      ]);
      expect(result.exitCode, 0, reason: '${result.stderr}');
      return out.path;
    } on ProcessException {
      if (named.isNotEmpty) rethrow;
    }
  }
  out.deleteSync();
  return null;
}

/// The guatiao-py bridge's file, or null where it was not built.
String? bridge() {
  final named =
      Platform.environment['GUATIAO_PY_BRIDGE'] ??
      repoPath([
        'crates',
        'guatiao-py',
        'target',
        'debug',
        dllFilename('guatiao_py'),
      ]);
  return File(named).existsSync() ? named : null;
}

/// The report of `greeter_caller_wide` for the provider filed under `key`:
/// what Rust's proxy got back for each limit, as exact integers.
Map<Object?, Object?> callWide(
  Registry reg,
  DynamicLibrary caller,
  String key,
) {
  final wide = caller
      .lookupFunction<
        Uint32 Function(
          Pointer<guatiao_host_info>,
          guatiao_str,
          Pointer<guatiao_value>,
        ),
        int Function(
          Pointer<guatiao_host_info>,
          guatiao_str,
          Pointer<guatiao_value>,
        )
      >('greeter_caller_wide');
  final out = calloc<guatiao_value>();
  final status = using((arena) => wide(reg.host(), strOf(arena, key).ref, out));
  final limits = Value.adopt(out);
  addTearDown(limits.close);
  checkStatus(status);
  return limits.toDart(numbers: Numbers.bigInt)! as Map<Object?, Object?>;
}

void expectWideLimits(Map<Object?, Object?> read) {
  expect(read['i64'], [
    BigInt.parse('9007199254740993'),
    BigInt.parse('-9223372036854775808'),
    BigInt.parse('9223372036854775807'),
  ]);
  expect(read['u64'], [
    BigInt.parse('9007199254740993'),
    BigInt.parse('9223372036854775808'),
    BigInt.parse('18446744073709551615'),
  ]);
}

// A Dart `int` is a signed 64-bit integer, so every `i64` is one, and a
// `u64` is carried as its bits: past 2^63 it is negative, and 2^64 - 1 is -1.
const twoPow53Plus1 = 9007199254740993;
const int64Min = -9223372036854775808;
const int64Max = 9223372036854775807;
final i64Limits = <int>[twoPow53Plus1, int64Min, int64Max];
// 2^53 + 1, INT64_MAX + 1 and UINT64_MAX, as Dart carries them.
final u64Limits = <int>[twoPow53Plus1, int64Min, -1];
final u64Exact = <BigInt>[
  BigInt.parse('9007199254740993'),
  BigInt.parse('9223372036854775808'),
  BigInt.parse('18446744073709551615'),
];

/// What a `u64` slot carries, as the number it stands for.
BigInt unsigned(int bits) => BigInt.from(bits).toUnsigned(64);

void crossesExactly(WideRemote wide) {
  for (final x in i64Limits) {
    expect(wide.echoI64(x), x);
  }
  for (var i = 0; i < u64Limits.length; i++) {
    final got = wide.echoU64(u64Limits[i]);
    expect(got, u64Limits[i]);
    expect(unsigned(got), u64Exact[i]);
  }
}

void exerciseAll(GreeterRemote greeter, String greeting) {
  final answer = greeter.greet('ana');
  addTearDown(answer.close);
  expect(answer.toDart(), {'greeting': '$greeting, ana'});
  expect(
    () => greeter.greet('').close(),
    throwsA(
      isA<BadValue>().having(
        (e) => e.message,
        'message',
        contains('nobody to greet'),
      ),
    ),
  );
  if (greeter.has('check')) {
    final config = Value.fromDart({'name': 'bo'});
    addTearDown(config.close);
    expect(greeter.check(config), isTrue);
    expect((config.toDart()! as Map).keys, containsAll(['name']));
    expect((config.toDart()! as Map).length, greaterThan(1));
  }
  if (greeter.has('greet_from')) {
    final taken = Value.fromDart({'name': 'cy'});
    final answer = greeter.greetFrom(taken);
    addTearDown(answer.close);
    expect(answer.toDart(), {'greeting': '$greeting, cy'});
    expect(taken.tag, Tag.absent, reason: 'a Value handed over is moved');
  }
  if (greeter.has('greet_into')) {
    final ear = Ear();
    greeter.greetInto('dee', ear);
    expect(ear.heardSoFar, ['$greeting, dee']);
  }
}

void main() {
  setUpAll(libraryDir);

  group('a Dart host calls greeters through the kind classes', () {
    late Registry reg;
    setUp(() {
      reg = Registry('dart-kinds', '0.1')
        ..scanDir(libraryDir(), rules: 'kind=greeter');
      addTearDown(reg.close);
    });

    test('written in Rust: every slot, objects both ways', () {
      final greeter = reg.bind('derived_greeter_hello', Greeter.kind);
      exerciseAll(greeter, 'hello');
      expect(greeter.shout('ana'), 'HELLO, ANA');
      final ear = Ear();
      final conversation = greeter.start(ear);
      conversation
        ..say('hi')
        ..say('again');
      final transcript = Uint8List(64);
      final n = conversation.read(transcript);
      expect(utf8.decode(transcript.sublist(0, n)), 'hi\nagain');
      expect(conversation.turns(), 2);
      expect(ear.heardSoFar, ['hi', 'again']);
      conversation.close();
      expect(conversation.isClosed, isTrue);
      expect(
        liveObjects(),
        0,
        reason: 'the listener handed over was destroyed',
      );
    });

    test('written in C: an older table answers has() and Unsupported', () {
      final dir = cGreeterDir;
      if (dir == null) {
        markTestSkipped('no C compiler');
        return;
      }
      expect(
        reg.scanDir(dir, rules: 'kind=greeter')['failed'],
        isEmpty,
        reason: 'set CC to a compiler for this architecture',
      );
      final greeter = reg.bind('c_greeter', Greeter.kind);
      exerciseAll(greeter, 'hello from C');
      expect(greeter.has('shout'), isFalse);
      expect(() => greeter.shout('ana'), throwsA(isA<Unsupported>()));
    });

    test('written in Python, through the bridge', () {
      final file = bridge();
      if (file == null) {
        markTestSkipped('no guatiao-py build: set GUATIAO_PY_BRIDGE');
        return;
      }
      reg.addSourceFile(file, {
        'dirs': [
          repoPath(['examples', 'python_provider']),
        ],
        'paths': [
          repoPath(['bindings', 'python', 'guatiao', 'src']),
          repoPath(['examples', 'greeter_kind', 'python']),
        ],
      });
      expect(reg.refresh()['refused'], isEmpty);
      final greeter = reg.bind('py_greeter_hello', Greeter.kind);
      exerciseAll(greeter, 'hello');
      expect(greeter.shout('ana'), 'HELLO, ANA');
    });

    test('a table of another kind is refused before anything runs', () {
      expect(
        () => reg.bind('hello_library_greeter', Greeter.kind),
        throwsA(isA<FloorMismatch>()),
      );
      expect(
        () => reg.bind('derived_greeter_shouter', Counter.kind),
        throwsA(isA<NotFound>()),
      );
    });
  });

  group('a Dart host crosses 64-bit integers through a slot', () {
    late Registry reg;
    setUp(() {
      reg = Registry('dart-wide', '0.1')
        ..scanDir(libraryDir(), rules: 'kind=greeter');
      addTearDown(reg.close);
    });

    test('written in Rust, exactly', () {
      crossesExactly(reg.bind('derived_greeter_hello', Wide.kind));
    });

    test('written in C, exactly', () {
      final dir = cGreeterDir;
      if (dir == null) {
        markTestSkipped('no C compiler');
        return;
      }
      expect(reg.scanDir(dir, rules: 'kind=greeter')['failed'], isEmpty);
      crossesExactly(reg.bind('c_greeter', Wide.kind));
    });

    test('written in Dart, exactly, and Rust calls each of them', () {
      final library = Library(
        'dart_wide',
        '0.1.0',
        offers: [
          Offer.single(Echo(), named: 'echo', kinds: [Wide.kind]),
        ],
      );
      addTearDown(library.close);
      reg.registerLibrary(library);
      crossesExactly(reg.bind('dart_wide_echo', Wide.kind));
      final caller = DynamicLibrary.open(
        '${libraryDir()}/${dllFilename('greeter_caller')}',
      );
      for (final key in ['dart_wide_echo', 'derived_greeter_hello']) {
        expectWideLimits(callWide(reg, caller, key));
      }
      final dir = cGreeterDir;
      if (dir != null) {
        reg.scanDir(dir, rules: 'kind=greeter');
        expectWideLimits(callWide(reg, caller, 'c_greeter'));
      }
    });

    test('a u64 past INT64_MAX is the negative int holding its bits', () {
      final wide = reg.bind('derived_greeter_hello', Wide.kind);
      expect(wide.echoU64(-1), -1);
      expect(unsigned(wide.echoU64(-1)), BigInt.two.pow(64) - BigInt.one);
      expect(unsigned(wide.echoU64(int64Min)), BigInt.two.pow(63));
      expect(unsigned(wide.echoU64(int64Max)), BigInt.two.pow(63) - BigInt.one);
      // An i64 carries the same bits as a signed number.
      expect(wide.echoI64(-1), -1);
    });
  });

  group('a provider written in Dart', () {
    late Registry reg;
    late Library library;
    late Hello hello;
    setUp(() {
      hello = Hello();
      library = Library(
        'dart_tests',
        '1.2.3',
        offers: [
          Offer.single(
            hello,
            named: 'hello',
            kinds: [Greeter.kind, Counter.kind],
            displayName: 'Hello, in Dart',
          ),
          Offer.single(Echo(), named: 'echo', kinds: [Wide.kind]),
          Offer.configured(
            Shouter.new,
            named: 'shouter',
            kinds: [Greeter.kind],
            schema: {
              'type': 'object',
              'properties': {
                'prefix': {'type': 'string'},
              },
              'required': ['prefix'],
            },
          ),
        ],
      );
      reg = Registry('dart-provider', '0.1');
      reg.registerLibrary(library);
      addTearDown(() {
        reg.close();
        library.close();
      });
    });

    test('is listed under its library, and called through its own table', () {
      final row = reg.provider('dart_tests_hello');
      expect(row['library'], 'dart_tests');
      expect(row['version'], '1.2.3');
      expect(row['display_name'], 'Hello, in Dart');
      final greeter = reg.bind('dart_tests_hello', Greeter.kind);
      exerciseAll(greeter, 'hello');
      expect(reg.bind('dart_tests_hello', Counter.kind).count(), 1);
      expect(hello.asked, 1);
      final conversation = greeter.start(Ear());
      conversation.say('hi');
      expect(conversation.turns(), 1);
      final before = Chat.disposed;
      conversation.close();
      expect(Chat.disposed, before + 1, reason: 'destroy disposes the object');
      expect(
        liveObjects(),
        2,
        reason: 'what stays reachable: the two single instances',
      );
    });

    test('is called from Rust, every slot', () {
      final caller = DynamicLibrary.open(
        '${libraryDir()}/${dllFilename('greeter_caller')}',
      );
      final run = caller
          .lookupFunction<
            Uint32 Function(
              Pointer<guatiao_host_info>,
              guatiao_str,
              guatiao_str,
              Pointer<guatiao_value>,
            ),
            int Function(
              Pointer<guatiao_host_info>,
              guatiao_str,
              guatiao_str,
              Pointer<guatiao_value>,
            )
          >('greeter_caller_run');
      final out = calloc<guatiao_value>();
      final status = using(
        (arena) => run(
          reg.host(),
          strOf(arena, 'dart_tests_hello').ref,
          strOf(arena, 'ana').ref,
          out,
        ),
      );
      final answer = Value.adopt(out);
      addTearDown(answer.close);
      checkStatus(status);
      final got = (answer.toDart()! as Map).cast<String, Object?>();
      expect(got['greet'], 'hello, ana');
      expect(got['refusal'], {
        'status': Status.badValue.code,
        'message': 'nobody to greet',
      });
      expect(got['failure'], Status.internalError.code);
      expect(got['shout'], 'HELLO, ANA');
      expect(got['checked'], isTrue);
      expect(got['greet_from'], 'hello, ana');
      expect(got['heard'], ['hello, ana']);
      expect(got['conversation'], containsPair('turns', 2));
      expect(got['conversation'], containsPair('dropped', true));

      expectWideLimits(callWide(reg, caller, 'dart_tests_echo'));
    });

    test('builds an instance per configuration the host checked', () {
      expect(
        () => reg.create('dart_tests_shouter', <String, Object?>{}),
        throwsA(
          isA<BadValue>().having(
            (e) => e.message,
            'message',
            contains('prefix'),
          ),
        ),
      );
      final before = Shouter.disposed;
      final instance = reg.create('dart_tests_shouter', {'prefix': 'hey'});
      final answer = instance.bind(Greeter.kind).greet('ana');
      addTearDown(answer.close);
      expect(answer.toDart(), {'greeting': 'hey ANA'});
      expect(library.liveInstances(), 1);
      instance.close();
      expect(library.liveInstances(), 0);
      expect(Shouter.disposed, before + 1);
    });

    test('a missing required slot is refused, naming it', () {
      expect(
        () => Library(
          'silent',
          '1.0',
          offers: [
            Offer.single(Silent(), named: 'silent', kinds: [Greeter.kind]),
          ],
        ),
        throwsA(
          isA<ArgumentError>().having(
            (e) => e.message,
            'message',
            'silent does not implement greet, which the greeter kind requires '
                '(its required slots: greet)',
          ),
        ),
      );
      expect(
        () => Library(
          'silent',
          '1.0',
          offers: [
            Offer.single(Echo(), named: 'echo', kinds: [Listener.kind]),
          ],
        ),
        throwsA(isA<ArgumentError>()),
        reason: 'an object kind is not a provider kind',
      );
    });

    test('a closed library lets its instances go', () {
      final before = liveObjects();
      library.close();
      expect(liveObjects(), before - 2);
    });
  });
}
