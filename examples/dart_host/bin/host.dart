// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// A plugin host written in Dart.
///
/// Scans the directories it is given for libraries declaring the `greeter`
/// kind, whatever language wrote them, registers the greeters written in
/// Dart (package `dart_greeter`), and calls each through `Greeter.kind`:
/// a map back, a map lent and changed in place, a map handed over, an
/// object handed in and one handed back, an object lent for one call.
/// Then, with `--caller`, it has the Rust caller (`greeter_caller`) call
/// the Dart greeter through the kind's Rust proxy.
///
///     dart run bin/host.dart DIR [DIR ...] [--caller FILE]
///         [--python-source BRIDGE --python-dir DIR --python-path DIR]
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:dart_greeter/dart_greeter.dart';
import 'package:ffi/ffi.dart';
import 'package:greeter_kind/greeter_kind.dart';
import 'package:guatiao/ffi.dart' show checkStatus, guatiao_host_info, guatiao_str, guatiao_value, strOf;
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kinds.dart';

/// A listener: what a conversation tells, kept.
final class Ear implements Listener {
  final List<String> heardSoFar = <String>[];

  @override
  void heard(String what) => heardSoFar.add(what);
}

void exercise(GreeterRemote greeter, void Function(String) say) {
  final answer = greeter.greet('ana');
  say('greet: ${jsonEncode(answer.toDart())}');
  answer.close();
  try {
    greeter.greet('').close();
  } on GuatiaoException catch (e) {
    say('greet refused: ${e.status?.name}: ${e.message}');
  }
  if (greeter.has('shout')) say('shout: ${greeter.shout('ana')}');
  if (greeter.has('check')) {
    final config = Value.fromDart({'name': 'bo'});
    final usable = greeter.check(config);
    say('check: $usable, lent map now ${jsonEncode(config.toDart())}');
    config.close();
  }
  if (greeter.has('greet_from')) {
    final answer = greeter.greetFrom({'name': 'cy'});
    say('greetFrom: ${jsonEncode(answer.toDart())}');
    answer.close();
  }
  if (greeter.has('greet_into')) {
    final ear = Ear();
    greeter.greetInto('dee', ear);
    say('greetInto: the lent listener heard ${jsonEncode(ear.heardSoFar)}');
  }
  if (greeter.has('start')) {
    final ear = Ear();
    ConversationRemote? conversation;
    try {
      conversation = greeter.start(ear);
    } on GuatiaoException catch (e) {
      say('start refused: ${e.message}');
    }
    if (conversation != null) {
      try {
        conversation
          ..say('hi')
          ..say('again');
        final transcript = Uint8List(64);
        final n = conversation.read(transcript);
        say(
          'conversation: ${conversation.turns()} turns, transcript '
          '${jsonEncode(utf8.decode(transcript.sublist(0, n)))}, listener '
          'heard ${jsonEncode(ear.heardSoFar)}',
        );
      } finally {
        conversation.close();
      }
    }
  }
}

/// A configuration naming every required text field of [schema] `hey`.
Map<String, Object?> configFor(Ref? schema) {
  final s = (schema?.toDart() as Map?) ?? const {};
  final properties = (s['properties'] as Map?) ?? const {};
  return {
    for (final name in (s['required'] as List?) ?? const [])
      if ((properties[name] as Map?)?['type'] == 'string') '$name': 'hey',
  };
}

typedef _RunC =
    Uint32 Function(
      Pointer<guatiao_host_info>,
      guatiao_str,
      guatiao_str,
      Pointer<guatiao_value>,
    );
typedef _RunD =
    int Function(
      Pointer<guatiao_host_info>,
      guatiao_str,
      guatiao_str,
      Pointer<guatiao_value>,
    );

/// `greeter_caller_run`: the Rust caller calls every slot of the greeter
/// filed under [key] through the kind's Rust proxy, and answers a map of
/// what each gave.
Object? rustCalls(String caller, Registry reg, String key) {
  final run = DynamicLibrary.open(
    caller,
  ).lookupFunction<_RunC, _RunD>('greeter_caller_run');
  final out = calloc<guatiao_value>();
  final status = using(
    (arena) => run(
      reg.host(),
      strOf(arena, key).ref,
      strOf(arena, 'ana').ref,
      out,
    ),
  );
  final answer = Value.adopt(out);
  try {
    checkStatus(status);
    return answer.toDart();
  } finally {
    answer.close();
  }
}

void main(List<String> argv) {
  final args = [...argv];
  List<String> all(String name) {
    final found = <String>[];
    for (var at = args.indexOf(name); at >= 0; at = args.indexOf(name)) {
      found.add(args[at + 1]);
      args.removeRange(at, at + 2);
    }
    return found;
  }

  final caller = all('--caller');
  final pythonSource = all('--python-source');
  final pythonDirs = all('--python-dir');
  final pythonPaths = all('--python-path');

  final library = greeterLibrary();
  final reg = Registry('dart-host', '1.0');
  try {
    for (final dir in args) {
      final report = reg.scanDir(dir, rules: 'kind=greeter');
      for (final loaded in report['loaded']! as List) {
        stdout.writeln('loaded ${File('$loaded').uri.pathSegments.last}');
      }
    }
    reg.registerLibrary(library);
    stdout.writeln('registered ${library.id} ${library.version} from Dart');
    if (pythonSource.isNotEmpty) {
      // The bridge imports python3.dll from the Python installation, on PATH.
      reg.importSearch = ImportSearch.path;
      reg.addSourceFile(pythonSource.single, {
        'dirs': pythonDirs,
        'paths': pythonPaths,
      });
      final changed = reg.refresh();
      for (final served in changed['loaded']! as List) {
        stdout.writeln('served ${(served! as Map)['id']} from Python');
      }
    }

    for (final provider in reg.providers('greeter')) {
      final key = '${provider['key']}';
      stdout.writeln('\n$key (${provider['library']} ${provider['version']})');
      void say(String line) => stdout.writeln('  $line');
      final GreeterRemote greeter;
      try {
        greeter = reg.bind(key, Greeter.kind);
      } on FloorMismatch catch (e) {
        say('not this greeter kind: ${e.reason}');
        continue;
      }
      if (provider['has_config'] != true) {
        exercise(greeter, say);
        continue;
      }
      // Built from a configuration: one the schema refuses, then one it
      // accepts. The registry checks it before the provider sees it.
      try {
        reg.create(key, <String, Object?>{}).close();
      } on GuatiaoException catch (e) {
        say('an empty configuration is refused: ${e.message}');
      }
      final config = configFor(reg.providerConfig(key));
      final instance = reg.create(key, config);
      try {
        say('instance from ${jsonEncode(config)}');
        exercise(instance.bind(Greeter.kind), say);
      } finally {
        instance.close();
      }
    }

    if (caller.isNotEmpty) {
      final answer = rustCalls(caller.single, reg, 'dart_greeter_hello');
      stdout.writeln('\nthe Rust caller, on the Dart greeter:');
      for (final MapEntry(:key, :value) in (answer! as Map).entries) {
        stdout.writeln('  $key: ${jsonEncode(value)}');
      }
    }
  } finally {
    reg.close();
    library.close();
  }
  stdout.writeln('objects a library can still reach: ${liveObjects()}');
}
