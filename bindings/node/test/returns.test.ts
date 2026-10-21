// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A provider written here builds what it answers in the allocator its
// caller carried in, grows a lent tree in the allocator that tree
// records, and frees what it was handed: proved with an allocator that
// counts its blocks. And a slot called from a thread Node did not start
// is answered on Node's thread.

import path from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { Registry, Value } from '../src/index.js';
import { OWNED, PROVIDER_ERROR, SIZE, Status, Tag, VALUE } from '../src/abi.js';
import { kind, liveObjects, t } from '../src/kinds.js';
import * as n from '../src/native.js';
import { Provider, library, libraryOf } from '../src/provide.js';
import { Greeter } from '../../../examples/greeter_kind/node/greeter_kind.ts';
import { Counting, REPO } from './helpers.js';
import { buildC, scratch } from './native.js';

const NODE_GREETER = path.join(REPO, 'examples', 'node_provider', 'node_greeter', 'greeters.ts');

let reg: Registry;
let table: bigint;
let ctx: bigint;

beforeAll(async () => {
  reg = new Registry('node-returns', '0.1');
  reg.registerLibrary(await libraryOf(NODE_GREETER));
  ({ table, ctx } = reg.providerTable('node_greeter_hello', 'greeter'));
});

afterAll(() => reg.close());

/** The slot at `index` of the greeter table, as koffi calls it. */
function slot(index: number, args: string[]): (...a: unknown[]) => number {
  const k = n.koffi();
  return k.decode(n.ptrAt(table, 8 + index * 8), k.proto('uint32_t', ['void *', ...args])) as (...a: unknown[]) => number;
}

describe('a provider written here', () => {
  it('answers in the allocator the carrier names', () => {
    using counting = new Counting();
    const greet = slot(0, ['guatiao_str', 'void *', 'void *']);
    const carrier = n.alloc(SIZE.map);
    n.setPtr(carrier, OWNED.alloc, counting.alloc);
    const err = n.alloc(SIZE.providerError);
    expect(greet(ctx, n.str('ana'), carrier, err)).toBe(Status.OK);
    expect(n.ptrAt(carrier, OWNED.alloc)).toBe(counting.alloc);
    expect(counting.live).toBeGreaterThan(0);
    const node = n.alloc(SIZE.value);
    n.setU32(node, VALUE.tag, Tag.MAP);
    n.copy(node + BigInt(VALUE.payload), carrier, SIZE.map);
    n.freeNode(node);
    expect(counting.live).toBe(0);
    for (const block of [node, carrier, err]) n.release(block);
  });

  it('answers in its own allocator when the carrier names none, and words a refusal', () => {
    const greet = slot(0, ['guatiao_str', 'void *', 'void *']);
    const carrier = n.alloc(SIZE.map);
    const err = n.alloc(SIZE.providerError);
    expect(greet(ctx, n.str(''), carrier, err)).toBe(Status.ERR_BAD_VALUE);
    expect(n.u32(err, PROVIDER_ERROR.status)).toBe(Status.ERR_BAD_VALUE);
    expect(n.stringText(err + BigInt(PROVIDER_ERROR.message))).toBe('nobody to greet');
    expect(n.size(carrier, OWNED.cap)).toBe(0);
    n.check(Status.OK, err);
    expect(greet(ctx, n.str('bo'), carrier, err)).toBe(Status.OK);
    expect(n.ptrAt(carrier, OWNED.alloc)).toBe(n.core().alloc);
    const node = n.alloc(SIZE.value);
    n.setU32(node, VALUE.tag, Tag.MAP);
    n.copy(node + BigInt(VALUE.payload), carrier, SIZE.map);
    n.freeNode(node);
    for (const block of [node, carrier, err]) n.release(block);
  });

  it('grows a lent map in the allocator it records', () => {
    using counting = new Counting();
    const config = new Value({ name: 'bo' }, { alloc: counting.alloc });
    const before = counting.live;
    expect(reg.bind('node_greeter_hello', Greeter).check(config)).toBe(true);
    expect(counting.live).toBeGreaterThan(before);
    expect(config.toJS()).toEqual({ name: 'bo', greeting: 'hello, bo' });
    config.close();
    expect(counting.live).toBe(0);
  });

  it('frees a map handed over, whatever it answers', () => {
    using counting = new Counting();
    const greeter = reg.bind('node_greeter_hello', Greeter);
    const config = new Value({ name: 'ana' }, { alloc: counting.alloc });
    greeter.greetFrom(config).close();
    expect(counting.live).toBe(0);
    const refused = new Value({ name: '' }, { alloc: counting.alloc });
    expect(() => greeter.greetFrom(refused)).toThrow(/nobody to greet/);
    expect(counting.live).toBe(0);
    config.close();
    refused.close();
  });

  it('lets go of every object it served once its holder is done', () => {
    const before = liveObjects();
    const greeter = reg.bind('node_greeter_hello', Greeter);
    const conversation = greeter.start({ heard() {} });
    expect(liveObjects()).toBe(before + 2);
    conversation.close();
    expect(liveObjects()).toBe(before);
  });
});

const Pair = kind('pair', {
  floorHash: 7,
  methods: { both: { args: [t.taken(t.map), t.taken(t.map)], answer: t.i64, required: true } },
});

class Sizes extends Provider {
  static override readonly kinds = [Pair];
  both(a: Value, b: Value): bigint {
    const size = BigInt(a.asMap().size + b.asMap().size);
    a.close();
    b.close();
    return size;
  }
}

const Mirror = kind('mirror', {
  floorHash: 8,
  methods: { show: { answer: t.map, required: true } },
});

class Built extends Provider {
  static override readonly kinds = [Mirror];
  show(): Value {
    return new Value({ built: 'here' });
  }
}

describe('a provider answering a value it built elsewhere', () => {
  it('copies it into the allocator the carrier names', () => {
    using counting = new Counting();
    reg.registerLibrary(library('node_mirror', '0.1.0', { providers: [Built] }));
    const found = reg.providerTable('node_mirror_built', 'mirror');
    const show = n.koffi().decode(n.ptrAt(found.table, 8), n.koffi().proto('uint32_t', ['void *', 'void *'])) as (
      ctx: bigint,
      out: bigint,
    ) => number;
    const carrier = n.alloc(SIZE.map);
    n.setPtr(carrier, OWNED.alloc, counting.alloc);
    expect(show(found.ctx, carrier)).toBe(Status.OK);
    expect(n.ptrAt(carrier, OWNED.alloc)).toBe(counting.alloc);
    const node = n.alloc(SIZE.value);
    n.setU32(node, VALUE.tag, Tag.MAP);
    n.copy(node + BigInt(VALUE.payload), carrier, SIZE.map);
    expect(counting.live).toBeGreaterThan(0);
    n.freeNode(node);
    expect(counting.live).toBe(0);
    n.release(node);
    n.release(carrier);
  });
});

describe('a caller handing trees over', () => {
  it('frees what it handed over when a later argument cannot be converted', () => {
    using counting = new Counting();
    reg.registerLibrary(library('node_pair', '0.1.0', { providers: [Sizes] }));
    const pair = reg.bind('node_pair_sizes', Pair);
    const first = new Value({ a: 1 }, { alloc: counting.alloc });
    expect(() => pair.both(first, [1] as never)).toThrow(TypeError);
    expect(first.isAbsent()).toBe(true);
    expect(counting.live).toBe(0);
    const again = new Value({ a: 1 }, { alloc: counting.alloc });
    expect(pair.both(again, { b: 2, c: 3 })).toBe(3n);
    expect(counting.live).toBe(0);
    first.close();
    again.close();
  });
});

describe('a call from a thread Node did not start', () => {
  it('runs on Node thread when the event loop turns, the caller waiting', async (ctx_) => {
    const built = buildC(path.join(REPO, 'bindings', 'node', 'test', 'fixtures', 'foreign_thread.c'), scratch('foreign'), 'foreign_thread');
    if (built === null) return ctx_.skip('no C compiler: the fixture is not built');
    const k = n.koffi();
    const lib = k.load(built);
    const start = lib.func('int foreign_greet(void *table, void *ctx, const char *name, void *free_value)');
    const done = lib.func('int foreign_done(void)');
    const status = lib.func('uint32_t foreign_status(void)');
    const greeting = lib.func('const char *foreign_greeting(void)');
    const free = k.load(n.core().path).symbol('guatiao_value_free') as bigint;
    expect(start(table, ctx, 'ana', free)).toBe(1);
    // Nothing has run yet: the call waits for this thread's event loop.
    expect(done()).toBe(0);
    for (let i = 0; i < 200 && !done(); i++) await new Promise((r) => setTimeout(r, 10));
    expect(done()).toBe(1);
    expect(status()).toBe(Status.OK);
    expect(greeting()).toBe('hello, ana');
  });
});
