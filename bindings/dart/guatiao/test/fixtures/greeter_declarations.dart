// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The greeter kinds declared in Dart, with no description behind them:
// describe_test.dart checks they describe byte for byte as the document
// #[guatiao::kind] writes (examples/greeter_kind/include/greeter_kind.kinds.json).

import 'package:guatiao/describe.dart';
import 'package:guatiao/guatiao.dart' show MapRef;

/// Greets by name.
abstract interface class Greeter {
  /// `{"greeting": "..."}`, or why not.
  @fallible
  MapAnswer greet(String name);

  /// Optional: a provider that leaves it out has the proxy run this
  /// body instead.
  @appended
  String shout(String name);

  /// Optional: a conversation the caller then drives, told to the
  /// listener the caller handed in. A greeter that leaves it out, or
  /// one that holds no conversations, answers the default.
  @appended
  @fallible
  Conversation start(Taken<Listener> listener);

  /// Optional: the greeting, told to a listener the caller LENDS for
  /// this call alone. It stays on the caller's side: nothing is
  /// allocated to pass it and nothing is destroyed after. A greeter
  /// leaving it out greets through `greet`.
  @appended
  @fallible
  void greetInto(String name, Lent<Listener> listener);

  /// Optional: the greeting for a configuration the caller HANDS OVER
  /// (`{"name": ".."}`). The greeter owns the map from the call on,
  /// whatever it answers, and may keep it; nothing is copied to pass
  /// it. A greeter leaving it out greets the name
  /// through `greet`.
  @appended
  @fallible
  MapAnswer greetFrom(Taken<MapRef> config);

  /// Optional: completes a configuration for `greet_from` that the
  /// caller LENDS for this call, in the caller's own map: fills in what
  /// it leaves out and adds what follows from it. Answers whether it can
  /// be greeted. A greeter leaving it out changes
  /// nothing.
  @appended
  @fallible
  bool check(Lent<MapRef> config);
}

/// What a conversation tells whoever is listening. Implemented by the
/// HOST and handed to the library, which owns it from then on.
abstract interface class Listener implements ObjectKind {
  /// Something was said.
  void heard(String what);
}

/// One conversation a greeter started: state the caller drives.
abstract interface class Conversation implements ObjectKind {
  /// Say something; the listener hears it.
  @fallible
  void say(String what);

  /// Copies the transcript so far into `dst` and answers how many bytes
  /// it wrote — an out-buffer, the caller's own memory.
  I64 read(BytesMut dst);

  /// How many things were said.
  I64 turns();
}

/// Counts how often it was asked.
abstract interface class Counter {
  /// The count so far, after this call.
  I64 count();
}

/// Answers the 64-bit integer it is given, unchanged: what a host in any
/// language crosses its 64-bit numbers with, both ways and at their
/// limits.
abstract interface class Wide {
  /// `x`, as it arrived.
  @fallible
  I64 echoI64(I64 x);

  /// `x`, as it arrived.
  @fallible
  U64 echoU64(U64 x);
}
