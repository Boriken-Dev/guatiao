/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// Must disagree with greeter_kind.kinds.json: a kind declared under a
// name of its own, after a block comment holding a `//`, answers nothing
// where `count` answers an integer.

/** Counts. */
GUATIAO_KIND_NAMED(tally, "counter", counter_vtable,
                   (required, count, void()));
