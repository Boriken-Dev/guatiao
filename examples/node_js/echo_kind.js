// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Two kinds defined here, in JavaScript, with no Rust trait or C header
// behind them: each is a class whose static fields describe it. A kind
// that exists elsewhere is never declared again like this; it is loaded
// from its description (see greeters.js). `describe.js` writes these two
// as a description, `echo_kind.kinds.json`, which the other languages'
// generators read.

import { Kind, slot, t } from 'guatiao/kinds';

/** A page that is written on: an object kind, one holder owns it. */
export class Page extends Kind {
  static kind = 'page';
  static floorHash = 0x70616765;
  static object = true;
  static doc = 'A page that is written on.';
  static slots = {
    write: slot(t.str).named('line').doc('Adds a line.').required(),
    lines: slot().returns(t.u32).doc('How many lines it holds.').required(),
  };
}

/** Repeats what it is told, and writes it on a page lent for the call. */
export class Echo extends Kind {
  static kind = 'echo';
  static floorHash = 0x6563686f;
  static doc = 'Repeats what it is told.';
  static slots = {
    echo: slot(t.str).named('text').returns(t.text).doc('`text`, twice, or why not.').required().fallible(),
    copyOnto: slot(t.str, t.lent(Page)).named('text', 'page').doc('Writes `text` on `page`, then the echo of it.').fallible(),
  };
}
