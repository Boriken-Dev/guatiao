// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What `tsc --strict` refuses: each `@ts-expect-error` below must meet an
// error, or the type check fails. Compiled by `npm run typecheck`, never
// run.

import type { Value } from '../src/index.js';
import { type Implementation, Kind, type Remote, slot, t } from '../src/kinds.js';
import { Provider } from '../src/provide.js';
import { Conversation, Counter, Greeter, Listener } from '../../../../examples/greeter_kind/node/greeter_kind.js';

// A provider that misspells a required method does not implement its kind.
// @ts-expect-error -- `greet` is required, and `greeet` is not it
export class Misspelled extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  greeet(name: string): { greeting: string } {
    return { greeting: name };
  }
}

export class WrongArgument extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  // @ts-expect-error -- `greet` takes the name as a string
  greet(name: number): { greeting: string } {
    return { greeting: String(name) };
  }
}

export class WrongAnswer extends Provider implements Greeter, Counter {
  static override readonly kinds = [Greeter, Counter];
  greet(name: string): { greeting: string } {
    return { greeting: name };
  }
  // @ts-expect-error -- `shout` answers text
  shout(name: string): number {
    return name.length;
  }
  // @ts-expect-error -- `count` answers an integer, not text
  count(): string {
    return '1';
  }
}

export function calls(greeter: Remote<typeof Greeter>): Value {
  // @ts-expect-error -- `greet` takes a string
  greeter.greet(5);
  // @ts-expect-error -- the kind has no `greeet`
  greeter.greeet('ana');
  // @ts-expect-error -- a listener has `heard`
  greeter.greetInto('ana', { said(): void {} });
  greeter.greetInto('ana', { heard(): void {} });
  const shouted: string = greeter.shout('ana');
  void shouted;
  return greeter.greet('ana');
}

// An object the provider answers implements the object kind's interface.
export class BadStart extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  greet(name: string): { greeting: string } {
    return { greeting: name };
  }
  // @ts-expect-error -- a conversation has `say`, `read` and `turns`
  start(): { say(): void } {
    return { say(): void {} };
  }
}

export class GoodStart extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  greet(name: string): { greeting: string } {
    return { greeting: name };
  }
  start(): Conversation {
    return { say(): void {}, read: (): bigint => 0n, turns: (): number => 0 };
  }
}

// A kind object only passes where its kind is asked for.
// @ts-expect-error -- a listener is an object kind, not one a provider serves
export const notServed: readonly (typeof Greeter)[] = [Listener];

// ---- a kind defined as a class -------------------------------------------------------

class Note extends Kind {
  static readonly floorHash = 7;
  static readonly object = true;
  static readonly slots = { add: slot(t.str).required() };
}

class Echo extends Kind {
  static readonly floorHash = 8;
  static readonly slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
    tell: slot(t.str, t.lent(Note)),
  };
}

export class Parrot extends Provider implements Implementation<typeof Echo> {
  static override readonly kinds = [Echo];
  echo(what: string): string {
    return what;
  }
  tell(what: string, note: Remote<typeof Note>): void {
    note.add(what);
  }
}

// @ts-expect-error -- `echo` is required
export class Mute extends Provider implements Implementation<typeof Echo> {
  static override readonly kinds = [Echo];
}

export class Garbled extends Provider implements Implementation<typeof Echo> {
  static override readonly kinds = [Echo];
  // @ts-expect-error -- `echo` answers text
  echo(what: string): number {
    return what.length;
  }
}

export function callsEcho(echo: Remote<typeof Echo>): string {
  // @ts-expect-error -- `echo` takes a string
  echo.echo(1);
  // @ts-expect-error -- a note has `add`
  echo.tell('x', { remove(): void {} });
  echo.tell('x', { add(): void {} });
  return echo.echo('x');
}

// @ts-expect-error -- `t.lent` takes an object kind, and Echo is a provider kind
export const lentProvider = t.lent(Echo);
