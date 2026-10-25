// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What `tsc --strict` refuses: each `@ts-expect-error` below must meet an
// error, or the type check fails. Compiled by `npm run typecheck`, never
// run.

import type { ListRef, MapRef, Value } from '../src/index.js';
import { type Implementation, Kind, type Lent, type Remote, type Taken, type i32, type i64, type u32, type u64, slot, t } from '../src/kinds.js';
import type { Tally as TallyInterface, Watcher as WatcherInterface } from '../../../../examples/tally_kind/tally_kind.ts';
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

// ---- a kind written as an interface ----------------------------------------------------

declare const aNumber: number;
declare const aBigint: bigint;

// A scalar type is its plain type with a mark: a plain value fits, another scalar does not.
export const fitsU32: u32 = aNumber;
export const fitsI64: i64 = aBigint;
export const readsBack: number = fitsU32;
// @ts-expect-error -- an i64 is a bigint, not a number
export const numberIsNotI64: i64 = aNumber;
// @ts-expect-error -- a u32 is not an i32
export const u32IsNotI32: i32 = fitsU32;
// @ts-expect-error -- an i64 is not a u64
export const i64IsNotU64: u64 = fitsI64;

// A taken tree is the `Value` that owns it, a lent one the view the caller keeps.
declare const taken: Taken<MapRef>;
export const ownedValue: Value = taken;
// @ts-expect-error -- a handed-over list is not a handed-over map
export const wrongTree: Taken<ListRef> = taken;
export const lentView: MapRef = {} as Lent<MapRef>;

// A class implementing the interface a kind is written as is checked against it.
export class Counts implements TallyInterface {
  add(_name: string, by: number): bigint {
    return BigInt(by);
  }
  total(_name: string): bigint {
    return 0n;
  }
  report(watcher: WatcherInterface): void {
    watcher.changed('a', 1n);
  }
}

// @ts-expect-error -- `total` is required
export class NoTotal implements TallyInterface {
  add(_name: string, by: number): bigint {
    return BigInt(by);
  }
}

export class WrongTotal implements TallyInterface {
  add(_name: string, by: number): bigint {
    return BigInt(by);
  }
  // @ts-expect-error -- `total` answers an i64, not text
  total(name: string): string {
    return name;
  }
}
