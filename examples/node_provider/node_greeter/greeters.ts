// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Greeters written in TypeScript: the same kinds the Rust, C and Python
// greeters serve, offered by a library any host in this process can call.
//
// The module is the library: its id and version are its package's
// (`package.json` beside it), its providers the `Provider` classes it
// exports. A host registers it with
// `registry.registerLibrary(await libraryOf(url))`.

import { BadValue, type MapRef, type Value } from 'guatiao';
import type { ObjectHandle, Remote } from 'guatiao/kinds';
import { Provider, s } from 'guatiao/provide';

import { type Conversation, Counter, Greeter, Listener } from '../../greeter_kind/node/greeter_kind.ts';

/** Changed by the examples to show a different build. */
export const GREETING = 'hello';

/**
 * One conversation `start` hands back, holding the listener it was handed.
 * Disposed when its holder destroys it, which lets the listener go.
 */
class Chat implements Conversation, Disposable {
  readonly #listener: ObjectHandle<typeof Listener>;
  readonly #transcript: string[] = [];

  constructor(listener: ObjectHandle<typeof Listener>) {
    this.#listener = listener;
  }

  [Symbol.dispose](): void {
    this.#listener.close();
  }

  say(what: string): void {
    if (!what) throw new BadValue('nothing to say');
    this.#transcript.push(what);
    this.#listener.heard(what);
  }

  read(dst: Uint8Array): bigint {
    const text = Buffer.from(this.#transcript.join('\n'), 'utf8');
    const n = Math.min(text.length, dst.length);
    dst.set(text.subarray(0, n));
    return BigInt(n);
  }

  turns(): bigint {
    return BigInt(this.#transcript.length);
  }
}

/** One provider serving two kinds, its own one instance. */
export class Hello extends Provider implements Greeter, Counter {
  static override readonly kinds = [Greeter, Counter];
  static override readonly displayName = 'Hello, in Node';
  #asked = 0n;

  greet(name: string): { greeting: string } {
    if (name === 'panic') throw new Error('a provider bug, on request');
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `${GREETING}, ${name}` };
  }

  shout(name: string): string {
    return this.greet(name).greeting.toUpperCase();
  }

  start(listener: ObjectHandle<typeof Listener>): Chat {
    return new Chat(listener);
  }

  greetInto(name: string, listener: Remote<typeof Listener>): void {
    listener.heard(this.greet(name).greeting);
  }

  greetFrom(config: Value): { greeting: string } {
    try {
      return this.greet(config.asMap().get('name')?.stringOr('') ?? '');
    } finally {
      config.close();
    }
  }

  check(config: MapRef): boolean {
    const name = config.get('name')?.stringOr('world') ?? 'world';
    config.set('name', name);
    config.set('greeting', `${GREETING}, ${name}`);
    return name !== '';
  }

  count(): bigint {
    this.#asked += 1n;
    return this.#asked;
  }
}

/** What a shouter is built from; its schema is written by these builders. */
export const ShouterConfig = s.object({ prefix: s.string() });
export type ShouterConfig = s.Infer<typeof ShouterConfig>;

/** Built from a configuration: one instance per configuration. */
export class Shouter extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  static override readonly config = ShouterConfig;
  readonly #prefix: string;

  constructor(config: ShouterConfig) {
    super();
    this.#prefix = config.prefix;
  }

  greet(name: string): { greeting: string } {
    return { greeting: `${this.#prefix} ${name.toUpperCase()}` };
  }
}
