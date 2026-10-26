// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind that exists only here, written as TypeScript interfaces: no Rust
// trait and no C header stand behind it. `guatiao-kind` reads this file
// into `include/tally_kind.kinds.json` and writes the Node module from it;
// the Dart package in `dart/` is written from the same document.

import type { MapRef } from 'guatiao';
import type { Lent, MapAnswer, ObjectKind, Taken, i64, u32 } from 'guatiao/kinds';

/** What a tally tells whoever is watching it. Implemented by the host. */
export interface Watcher extends ObjectKind {
  /** `name` now stands at `total`. */
  changed(name: string, total: i64): void;
}

/** Counts by name. */
export interface Tally {
  /**
   * Adds `by` to the count under `name`: the new count, or why not.
   *
   * @throws
   */
  add(name: string, by: u32): i64;
  /** The count under `name`: 0 when there is none. */
  total(name: string): i64;
  /** Optional: every count, as `{name: count}`. */
  all?(): MapAnswer;
  /** Optional: whether `name` has a count. */
  known?(name: string): boolean;
  /** Optional: the sum of the bytes, modulo 2^32. */
  checksum?(data: Uint8Array): u32;
  /**
   * Optional: tells `watcher` each count, for this call alone.
   *
   * @throws
   */
  report?(watcher: Lent<Watcher>): void;
  /**
   * Optional: adds the counts of `counts` (`{name: count}`), which the
   * tally owns from the call on; answers how many names it held.
   *
   * @throws
   */
  load?(counts: Taken<MapRef>): u32;
}
