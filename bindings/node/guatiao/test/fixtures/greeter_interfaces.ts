// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The greeter example's kinds, written as TypeScript interfaces: what
// `describeInterfaces` reads, and what the test compares with the
// description `#[guatiao::kind]` writes for the same kinds.

import type { MapRef } from 'guatiao';
import type { BytesMut, Lent, MapAnswer, ObjectKind, Taken, i64, u64 } from 'guatiao/kinds';

/** Greets by name. */
export interface Greeter {
  /**
   * `{"greeting": "..."}`, or why not.
   *
   * @throws
   */
  greet(name: string): MapAnswer;
  /**
   * Optional: a provider that leaves it out has the proxy run this
   * body instead.
   */
  shout?(name: string): string;
  /**
   * Optional: a conversation the caller then drives, told to the
   * listener the caller handed in. A greeter that leaves it out, or
   * one that holds no conversations, answers the default.
   *
   * @throws
   */
  start?(listener: Taken<Listener>): Conversation;
  /**
   * Optional: the greeting, told to a listener the caller LENDS for
   * this call alone. It stays on the caller's side: nothing is
   * allocated to pass it and nothing is destroyed after. A greeter
   * leaving it out greets through `greet`.
   *
   * @throws
   */
  greetInto?(name: string, listener: Lent<Listener>): void;
  /**
   * Optional: the greeting for a configuration the caller HANDS OVER
   * (`{"name": ".."}`). The greeter owns the map from the call on,
   * whatever it answers, and may keep it; nothing is copied to pass
   * it. A greeter leaving it out greets the name
   * through `greet`.
   *
   * @throws
   */
  greetFrom?(config: Taken<MapRef>): MapAnswer;
  /**
   * Optional: completes a configuration for `greet_from` that the
   * caller LENDS for this call, in the caller's own map: fills in what
   * it leaves out and adds what follows from it. Answers whether it can
   * be greeted. A greeter leaving it out changes
   * nothing.
   *
   * @throws
   */
  check?(config: Lent<MapRef>): boolean;
}

/**
 * What a conversation tells whoever is listening. Implemented by the
 * HOST and handed to the library, which owns it from then on.
 */
export interface Listener extends ObjectKind {
  /** Something was said. */
  heard(what: string): void;
}

/** One conversation a greeter started: state the caller drives. */
export interface Conversation extends ObjectKind {
  /**
   * Say something; the listener hears it.
   *
   * @throws
   */
  say(what: string): void;
  /**
   * Copies the transcript so far into `dst` and answers how many bytes
   * it wrote — an out-buffer, the caller's own memory.
   */
  read(dst: BytesMut): i64;
  /** How many things were said. */
  turns(): i64;
}

/** Counts how often it was asked. */
export interface Counter {
  /** The count so far, after this call. */
  count(): i64;
}

/**
 * Answers the 64-bit integer it is given, unchanged: what a host in any
 * language crosses its 64-bit numbers with, both ways and at their
 * limits.
 */
export interface Wide {
  /**
   * `x`, as it arrived.
   *
   * @throws
   */
  echoI64(x: i64): i64;
  /**
   * `x`, as it arrived.
   *
   * @throws
   */
  echoU64(x: u64): u64;
}

