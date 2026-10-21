// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The kinds of `include/greeter_kind.h`, described for Node.
//
// Written by hand from the header: each method in the order its table
// declares it, its shapes from `t`, each floor hash copied from the
// `#define` beside the table. A test compares the literals with the
// header, so a kind that changes shape fails there rather than in a call.
// A host calls a greeter through `Greeter`; a provider implements it.

import { type Implementation, kind, t } from 'guatiao/kinds';

/** `listener_vtable`: what a conversation tells, implemented by the host. */
export const Listener = kind('listener', {
  floorHash: 1867921573,
  object: true,
  methods: {
    heard: { args: [t.str], required: true },
  },
});
export type Listener = Implementation<typeof Listener>;

/** `conversation_vtable`: one conversation a greeter started. */
export const Conversation = kind('conversation', {
  floorHash: 2769624503,
  object: true,
  methods: {
    say: { args: [t.str], required: true, fallible: true },
    read: { args: [t.bytesMut], answer: t.i64, required: true },
    turns: { answer: t.i64, required: true },
  },
});
export type Conversation = Implementation<typeof Conversation>;

/**
 * `greeter_vtable`: greets by name. `greet` is the one slot every table
 * has; the rest were appended, and an older table may lack them.
 */
export const Greeter = kind('greeter', {
  floorHash: 2695856491,
  methods: {
    greet: { args: [t.str], answer: t.map, required: true, fallible: true },
    shout: { args: [t.str], answer: t.text },
    start: { args: [t.taken(Listener)], answer: Conversation, fallible: true },
    greetInto: { args: [t.str, t.lent(Listener)], fallible: true },
    greetFrom: { args: [t.taken(t.map)], answer: t.map, fallible: true },
    check: { args: [t.lent(t.map)], answer: t.bool, fallible: true },
  },
});
export type Greeter = Implementation<typeof Greeter>;

/** `counter_vtable`: counts how often it was asked. */
export const Counter = kind('counter', {
  floorHash: 1227528937,
  methods: {
    count: { answer: t.i64, required: true },
  },
});
export type Counter = Implementation<typeof Counter>;
