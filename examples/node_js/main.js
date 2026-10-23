// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A host in plain JavaScript: registers the library greeters.js is, adds
// the greeters it finds in the directories it is given (written in any
// language), and calls each through the greeter kind; then calls the
// echo kind, defined in JavaScript, lending it a page.
//
//     node main.js [DIR ...]

import { GuatiaoError, Registry } from 'guatiao';
import { libraryOf } from 'guatiao/provide';

import { Greeter } from '../greeter_kind/node/greeter_kind.js';
import { Echo } from './echo_kind.js';

/** A page: what the echo kind writes on. */
class Page {
  /** @type {string[]} */
  written = [];
  /** @param {string} line */
  write(line) {
    this.written.push(line);
  }
  lines() {
    return this.written.length;
  }
}

const library = await libraryOf(new URL('./greeters.js', import.meta.url));
const reg = new Registry('js-host', '1.0');
try {
  reg.registerLibrary(library);
  console.log(`registered ${library.id} ${library.version}`);
  for (const dir of process.argv.slice(2)) reg.scanDir(dir, { rules: 'kind=greeter' });

  for (const provider of reg.providers('greeter')) {
    const key = String(provider.key);
    if (provider.has_config) continue;
    let greeter;
    try {
      greeter = reg.bind(key, Greeter);
    } catch (e) {
      console.log(`${key}: not this greeter kind`);
      continue;
    }
    const answer = greeter.greet('ana');
    console.log(`${key}: ${JSON.stringify(answer.toJS())}`);
    answer.close();
    try {
      greeter.greet('').close();
    } catch (e) {
      if (!(e instanceof GuatiaoError)) throw e;
      console.log(`${key}: refused, ${e.name}: ${e.message}`);
    }
  }

  const echo = reg.bind('js_greeter_parrot', Echo);
  console.log(`echo: ${echo.echo('hi')}`);
  const page = new Page();
  echo.copyOnto('ho', page);
  console.log(`the lent page: ${JSON.stringify(page.written)}`);
} finally {
  reg.close();
  library.close();
}
