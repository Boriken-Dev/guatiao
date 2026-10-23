// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What `// @ts-check` refuses in plain JavaScript, typed through JSDoc
// and the generated declarations: each `@ts-expect-error` below must meet
// an error. Compiled by `npm run typecheck`, never run. This package's
// tsconfig sets noImplicitOverride, hence the `@override` tags.

import { Provider } from '../src/provide.js';
import { Greeter } from '../../../../examples/greeter_kind/node/greeter_kind.js';

/** @implements {Greeter} */
export class Hello extends Provider {
  /** @override */
  static kinds = [Greeter];
  /** @param {string} name */
  greet(name) {
    return { greeting: `hello, ${name}` };
  }
}

/** @implements {Greeter} */
// @ts-expect-error -- `greet` is required
export class Silent extends Provider {
  /** @override */
  static kinds = [Greeter];
}

/** @implements {Greeter} */
export class Loud extends Provider {
  /** @override */
  static kinds = [Greeter];
  /** @param {string} name */
  greet(name) {
    return { greeting: name };
  }
  /** @param {string} name */
  // @ts-expect-error -- `shout` answers text
  shout(name) {
    return name.length;
  }
}

/** @param {import('../src/kinds.js').Remote<typeof Greeter>} greeter */
export function calls(greeter) {
  // @ts-expect-error -- `greet` takes a string
  greeter.greet(5);
  return greeter.greet('ana');
}
