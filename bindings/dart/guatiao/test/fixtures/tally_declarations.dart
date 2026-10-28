// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The tally kinds declared in Dart: describe_test.dart checks they
// describe byte for byte as the TypeScript declaration of the same kinds
// (examples/tally_kind/tally_kind.ts) does.

import 'dart:typed_data';

import 'package:guatiao/describe.dart';
import 'package:guatiao/guatiao.dart' show MapRef;

/// What a tally tells whoever is watching it. Implemented by the host.
abstract interface class Watcher implements ObjectKind {
  /// `name` now stands at `total`.
  void changed(String name, I64 total);
}

/// Counts by name.
abstract interface class Tally {
  /// Adds `by` to the count under `name`: the new count, or why not.
  @fallible
  I64 add(String name, U32 by);

  /// The count under `name`: 0 when there is none.
  I64 total(String name);

  /// Optional: every count, as `{name: count}`.
  @optional
  MapAnswer all();

  /// Optional: whether `name` has a count.
  @optional
  bool known(String name);

  /// Optional: the sum of the bytes, modulo 2^32.
  @optional
  U32 checksum(Uint8List data);

  /// Optional: tells `watcher` each count, for this call alone.
  @optional
  @fallible
  void report(Lent<Watcher> watcher);

  /// Optional: adds the counts of `counts` (`{name: count}`), which the
  /// tally owns from the call on; answers how many names it held.
  @optional
  @fallible
  U32 load(Taken<MapRef> counts);
}
