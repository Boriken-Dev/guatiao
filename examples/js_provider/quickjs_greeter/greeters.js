// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Greeters written in JavaScript, in the shape the Node binding gives
// providers: the same file runs under Node (`libraryOf`) and under the
// guatiao-source-js source (QuickJS), serving the same kinds as the greeters in
// Rust, C, Python and Lua.
//
// The module is the library: its id and version are its package's
// (`package.json` beside it), its providers the `Provider` classes it
// exports. `quickjs_greeter.guatiao-js.toml` beside it says which kinds it
// serves and where its code is, which the source reads before running
// anything. The kinds come from the module guatiao-kind wrote from their
// description, so nothing here says what a slot takes.

import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';

import { Counter, Greeter } from '../../greeter_kind/node/greeter_kind.js';

/** Changed by the examples to show a refresh serving new code. */
export const GREETING = 'hello';

/**
 * One conversation `start` hands back: the listener it was given, and what
 * was said. Disposed when its holder destroys it, which lets the listener
 * go.
 */
class Chat {
  #listener;
  /** @type {string[]} */
  #said = [];

  /** @param {{ heard(what: string): void, close(): void }} listener */
  constructor(listener) {
    this.#listener = listener;
  }

  [Symbol.dispose]() {
    this.#listener.close();
  }

  /** @param {string} what */
  say(what) {
    if (!what) throw new BadValue('nothing to say');
    this.#said.push(what);
    this.#listener.heard(what);
  }

  /** @param {Uint8Array} dst */
  read(dst) {
    const text = new TextEncoder().encode(this.#said.join(' '));
    const n = Math.min(text.length, dst.length);
    dst.set(text.subarray(0, n));
    return BigInt(n);
  }

  turns() {
    return BigInt(this.#said.length);
  }
}

/**
 * One provider serving two kinds, keeping its count in itself.
 * @implements {Greeter}
 * @implements {Counter}
 */
export class Hello extends Provider {
  static kinds = [Greeter, Counter];
  static displayName = 'Hello, in JavaScript';
  #asked = 0n;

  /** @param {string} name */
  greet(name) {
    if (name === 'panic') throw new Error('a provider bug, on request');
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `${GREETING}, ${name}` };
  }

  // `shout` is not written: a caller in Rust runs the kind's default.

  /** @param {{ heard(what: string): void, close(): void }} listener */
  start(listener) {
    return new Chat(listener);
  }

  /**
   * @param {string} name
   * @param {{ heard(what: string): void }} listener
   */
  greetInto(name, listener) {
    listener.heard(this.greet(name).greeting);
  }

  count() {
    this.#asked += 1n;
    return this.#asked;
  }
}

/**
 * A provider of one kind.
 * @implements {Greeter}
 */
export class Polite extends Provider {
  static kinds = [Greeter];
  static displayName = 'Polite, in JavaScript';

  /** @param {string} name */
  greet(name) {
    if (!name) throw new BadValue('nobody to greet, politely');
    return { greeting: `good day, ${name}` };
  }
}
