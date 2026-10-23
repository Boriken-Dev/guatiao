// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/// Greeters written in Dart: the same kinds the Rust, C and Python
/// greeters serve, offered by a library any host in this process calls.
///
/// The kinds are the classes `dart run guatiao:kind` wrote from their
/// description (package `greeter_kind`); nothing here says what a slot
/// takes. The library's id and version are this package's, from
/// `lib/src/library_identity.g.dart`, which the same command writes from
/// `pubspec.yaml`.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:greeter_kind/greeter_kind.dart';
import 'package:guatiao/guatiao.dart';
import 'package:guatiao/kinds.dart';

import 'src/library_identity.g.dart';

/// One conversation `start` hands back, holding the listener it was
/// handed: disposed when its holder destroys it, which lets the listener
/// go.
final class Chat implements Conversation, Disposable {
  Chat(this._listener);

  final ListenerRemote _listener;
  final List<String> _transcript = <String>[];

  @override
  void dispose() => _listener.close();

  @override
  void say(String what) {
    if (what.isEmpty) {
      throw BadValue(Status.badValue.code, 'nothing to say');
    }
    _transcript.add(what);
    _listener.heard(what);
  }

  @override
  int read(Uint8List dst) {
    final text = utf8.encode(_transcript.join('\n'));
    final n = text.length < dst.length ? text.length : dst.length;
    dst.setRange(0, n, text);
    return n;
  }

  @override
  int turns() => _transcript.length;
}

/// One provider serving two kinds, its own one instance.
final class Hello extends Provider
    implements
        Greeter,
        GreeterShout,
        GreeterStart,
        GreeterGreetInto,
        GreeterGreetFrom,
        GreeterCheck,
        Counter {
  int _asked = 0;

  @override
  Map<String, Object?> greet(String name) {
    if (name == 'panic') throw StateError('a provider bug, on request');
    if (name.isEmpty) {
      throw BadValue(Status.badValue.code, 'nobody to greet');
    }
    return {'greeting': 'hello, $name'};
  }

  @override
  String shout(String name) => (greet(name)['greeting']! as String).toUpperCase();

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
    config['name'] = name;
    config['greeting'] = 'hello, $name';
    return name.isNotEmpty;
  }

  @override
  int count() => ++_asked;
}

/// Answers the 64-bit integers it is given, unchanged.
final class Echo extends Provider implements Wide {
  @override
  int echoI64(int x) => x;

  @override
  int echoU64(int x) => x;
}

/// Built from a configuration: one instance per configuration the host
/// checked against [schema].
final class Shouter extends Provider implements Greeter {
  Shouter(Map<String, Object?> config) : _prefix = config['prefix']! as String;

  /// What a shouter is built from, as a JSON Schema.
  static const Map<String, Object?> schema = {
    'type': 'object',
    'properties': {
      'prefix': {'type': 'string'},
    },
    'required': ['prefix'],
    'additionalProperties': false,
  };

  final String _prefix;

  @override
  Map<String, Object?> greet(String name) => {
    'greeting': '$_prefix ${name.toUpperCase()}',
  };
}

/// The library this package is: `dart_greeter_hello`, `dart_greeter_echo`
/// and `dart_greeter_shouter`. Build it once, at the top of a program, and
/// register it with `Registry.registerLibrary`.
Library greeterLibrary() => Library.of(
  packageLibrary,
  offers: [
    Offer.single(
      Hello(),
      named: 'hello',
      kinds: [Greeter.kind, Counter.kind],
      displayName: 'Hello, in Dart',
    ),
    Offer.single(Echo(), named: 'echo', kinds: [Wide.kind]),
    Offer.configured(
      Shouter.new,
      named: 'shouter',
      kinds: [Greeter.kind],
      schema: Shouter.schema,
    ),
  ],
);
