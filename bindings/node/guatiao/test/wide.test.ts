// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// 64-bit integers, which a JavaScript number holds exactly only to 2^53,
// cross a slot as a bigint, exactly: through a provider written in Rust,
// C or TypeScript, and from Rust into the TypeScript one.

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { Internal, Registry } from '../src/index.js';
import * as n from '../src/native.js';
import { Provider, library } from '../src/provide.js';
import { adopt } from '../src/value.js';
import { Wide } from '../../../../examples/greeter_kind/node/greeter_kind.js';
import { cLibraries, workspaceLibrary } from './native.js';
import { libraryDir } from './setup.js';

const TWO53_1 = (1n << 53n) + 1n;
const I64_MIN = -(1n << 63n);
const I64_MAX = (1n << 63n) - 1n;
const U64_TOP = 1n << 63n;
const U64_MAX = (1n << 64n) - 1n;
const SIGNED = [TWO53_1, I64_MIN, I64_MAX];
const UNSIGNED = [TWO53_1, U64_TOP, U64_MAX];

/** Answers what it is handed. */
class Echo extends Provider implements Wide {
  static override readonly kinds = [Wide];
  echoI64(x: bigint): bigint {
    return x;
  }
  echoU64(x: bigint): bigint {
    return x;
  }
}

/** Answers one past what it is handed, and a number a bigint would not be. */
class Overshoot extends Provider implements Wide {
  static override readonly kinds = [Wide];
  echoI64(x: bigint): bigint {
    return x + 1n;
  }
  echoU64(x: bigint): number {
    return x === 0n ? -1 : 1.5;
  }
}

const PROVIDERS = {
  rust: 'derived_greeter_hello',
  c: 'c_greeter',
  node: 'node_wide_echo',
} as const;
type Which = keyof typeof PROVIDERS;

let reg: Registry;
const present = new Set<Which>(['rust', 'node']);

beforeAll(() => {
  reg = new Registry('node-wide', '0.1');
  reg.scanDir(libraryDir(), { rules: 'kind=greeter' });
  const cDir = cLibraries()?.greeterDir;
  if (cDir !== undefined) {
    reg.scanDir(cDir, { rules: 'kind=greeter' });
    present.add('c');
  }
  reg.registerLibrary(library('node_wide', '0.1.0', { providers: [Echo] }));
  reg.registerLibrary(library('node_overshoot', '0.1.0', { providers: [Overshoot] }));
});

afterAll(() => {
  reg.close();
});

const each = it.for(Object.keys(PROVIDERS) as Which[]);

function wide(which: Which, skip: (note: string) => void) {
  if (!present.has(which)) skip('no C compiler: the greeter written in C is not built');
  return reg.bind(PROVIDERS[which], Wide);
}

describe('a slot of a provider carries 64-bit integers exactly', () => {
  each('%s, as a bigint', (which, ctx) => {
    const remote = wide(which, (note) => ctx.skip(note));
    for (const x of SIGNED) expect(remote.echoI64(x)).toBe(x);
    for (const x of UNSIGNED) expect(remote.echoU64(x)).toBe(x);
  });

  each('%s, as a number that is a whole double', (which, ctx) => {
    const remote = wide(which, (note) => ctx.skip(note));
    // A number is the double it is: 2^53 + 1 written as one is 2^53.
    expect(remote.echoI64(2 ** 53)).toBe(1n << 53n);
    expect(remote.echoI64(-5)).toBe(-5n);
    expect(remote.echoU64(2 ** 53)).toBe(1n << 53n);
    expect(remote.echoU64(2 ** 63)).toBe(U64_TOP);
    expect(remote.echoU64(2 ** 64 - 2048)).toBe(U64_MAX - 2047n);
  });

  each('%s refuses what does not fit, and what is no integer', (which, ctx) => {
    const remote = wide(which, (note) => ctx.skip(note));
    for (const bad of [I64_MAX + 1n, I64_MIN - 1n, U64_MAX, 2 ** 63, -(2 ** 63) - 2048]) {
      expect(() => remote.echoI64(bad)).toThrow(RangeError);
      expect(() => remote.echoI64(bad)).toThrow(/does not fit t\.i64/);
    }
    for (const bad of [-1n, -1, U64_MAX + 1n, 2 ** 64]) {
      expect(() => remote.echoU64(bad)).toThrow(RangeError);
      expect(() => remote.echoU64(bad)).toThrow(/does not fit t\.u64/);
    }
    for (const bad of [1.5, NaN, Infinity, '5', true, null, undefined]) {
      expect(() => remote.echoI64(bad as never)).toThrow(TypeError);
      expect(() => remote.echoU64(bad as never)).toThrow(TypeError);
    }
  });
});

describe('a provider in TypeScript is handed and answers bigints', () => {
  it('fails the call whose answer does not fit', () => {
    const remote = reg.bind('node_overshoot_overshoot', Wide);
    expect(remote.echoI64(5n)).toBe(6n);
    expect(() => remote.echoI64(I64_MAX)).toThrow(Internal);
    expect(() => remote.echoU64(0n)).toThrow(Internal);
    expect(() => remote.echoU64(1n)).toThrow(Internal);
  });
});

describe('a Rust caller calls a wide provider', () => {
  let caller: (host: bigint, key: n.StrArg, out: bigint) => number;
  beforeAll(() => {
    const lib = n.koffi().load(workspaceLibrary('greeter_caller'));
    caller = lib.func('uint32_t greeter_caller_wide(void *host, guatiao_str key, void *out)') as typeof caller;
  });

  each('%s, exactly', (which, ctx) => {
    wide(which, (note) => ctx.skip(note));
    const out = n.alloc(40);
    expect(caller(reg.host(), n.str(PROVIDERS[which]), out)).toBe(0);
    using report = adopt(out);
    const got = report.toJS<{ i64: bigint[]; u64: bigint[] }>({ numbers: 'bigint' });
    expect(got.i64).toEqual(SIGNED);
    expect(got.u64).toEqual(UNSIGNED);
  });
});
