// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Two kinds defined here, in JavaScript, with no Rust trait or C header
// behind them: each is a class whose static fields describe it. A kind
// that exists elsewhere is never declared again like this; it is loaded
// from its description (see greeters.js).

import { Kind, slot, t } from 'guatiao/kinds';

/** A page that is written on: an object kind, one holder owns it. */
export class Page extends Kind {
  static kind = 'page';
  static floorHash = 0x70616765;
  static object = true;
  static slots = {
    write: slot(t.str).required(),
    lines: slot().returns(t.u32).required(),
  };
}

/** Repeats what it is told, and writes it on a page lent for the call. */
export class Echo extends Kind {
  static kind = 'echo';
  static floorHash = 0x6563686f;
  static slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
    copyOnto: slot(t.str, t.lent(Page)).fallible(),
  };
}
