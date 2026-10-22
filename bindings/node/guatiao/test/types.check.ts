// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What `tsc --strict` refuses: each `@ts-expect-error` below must meet an
// error, or the type check fails. Compiled by `npm run typecheck`, never
// run.

import type { Value } from '../src/index.js';
import type { Remote } from '../src/kinds.js';
import { Provider } from '../src/provide.js';
import { Counter, Greeter } from '../../../../examples/greeter_kind/node/greeter_kind.ts';

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
