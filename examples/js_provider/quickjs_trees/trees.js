// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A greeter that takes trees, written for the guatiao-source-js source only: a
// tree argument is plain data here (an object or an array), and one the
// caller lends is changed in place and read back. The Node binding hands
// a `Value` or a `MapRef` for the same slots, so this file does not run
// under Node; see `../quickjs_greeter` for one that does.

import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';

import { Greeter } from '../../greeter_kind/node/greeter_kind.js';

export class Trees extends Provider {
  static kinds = [Greeter];
  static displayName = 'Trees, in JavaScript';

  /** @param {string} name */
  greet(name) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello, ${name}` };
  }

  /** @param {{ name?: string }} config handed over: this provider's to keep */
  greetFrom(config) {
    return this.greet(config.name ?? '');
  }

  /** @param {{ name?: string, checkedBy?: string }} config lent for this call */
  check(config) {
    config.name ??= 'ana';
    config.checkedBy = 'JavaScript';
    return true;
  }
}
