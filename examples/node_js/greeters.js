// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A library written in plain JavaScript, run by Node as it is: its id and
// version are its package's (package.json beside it), its providers the
// Provider classes it exports.
//
// The greeter kinds come from the module guatiao-kind wrote from their
// description, so nothing here says what their slots take; `@implements`
// lets an editor with `// @ts-check` check each method against them. A
// provider missing a required method is refused, naming it, when the
// library is built.

import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';

import { Counter, Greeter } from '../greeter_kind/node/greeter_kind.js';
import { Echo } from './echo_kind.js';

/**
 * Greets, and counts how often it was asked.
 * @implements {Greeter}
 * @implements {Counter}
 */
export class Hello extends Provider {
  static kinds = [Greeter, Counter];
  static displayName = 'Hello, in JavaScript';
  #asked = 0n;

  /** @param {string} name */
  greet(name) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello from JS, ${name}` };
  }

  /** @param {string} name */
  shout(name) {
    return this.greet(name).greeting.toUpperCase();
  }

  count() {
    this.#asked += 1n;
    return this.#asked;
  }
}

/** Serves the echo kind, defined in JavaScript beside it. */
export class Parrot extends Provider {
  static kinds = [Echo];

  /** @param {string} what */
  echo(what) {
    if (!what) throw new BadValue('nothing to repeat');
    return `${what}, ${what}`;
  }

  /**
   * @param {string} what
   * @param {import('guatiao/kinds').Remote<typeof import('./echo_kind.js').Page>} page
   */
  copyOnto(what, page) {
    page.write(what);
    page.write(this.echo(what));
  }
}
