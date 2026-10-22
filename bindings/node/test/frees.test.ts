// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The binding frees what it allocates on every path, proved with an
// allocator that counts the blocks it handed out and has not taken back.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { Value } from '../src/index.js';
import { Counting } from './helpers.js';

class Unconvertible {}

describe('frees', () => {
  let counting: Counting;
  beforeEach(() => {
    counting = new Counting();
  });
  afterEach(() => {
    counting[Symbol.dispose]();
  });
  const alloc = (): { alloc: bigint } => ({ alloc: counting.alloc });

  it('leaves nothing after a clean build and close', () => {
    new Value([[1, 'x'], { k: [2, 3] }], alloc()).close();
    expect(counting.live).toBe(0);
  });

  it.each([
    [[[1, 'x', new Unconvertible()]]],
    [{ a: [1, 2], b: { c: 'd', e: new Unconvertible() } }],
    [[{ a: 1 }, { b: [true, null, new Unconvertible()] }]],
    [new Map<unknown, unknown>([['a', 1], [2, 'x']])],
    [[Number.NaN]],
  ])('frees what a conversion built before it failed: %#', (bad) => {
    expect(() => new Value(bad, alloc())).toThrow();
    expect(counting.live).toBe(0);
  });

  it('frees the partial value of a failed map write', () => {
    const v = new Value({ a: 1 }, alloc());
    const before = counting.live;
    expect(() => v.asMap().set('b', [1, 'two', new Unconvertible()])).toThrow(TypeError);
    expect(counting.live).toBe(before);
    expect(v.toJS()).toEqual({ a: 1 });
    v.close();
    expect(counting.live).toBe(0);
  });

  it('frees the partial value of a failed push', () => {
    const v = new Value(['a'], alloc());
    const before = counting.live;
    expect(() => v.asList().push({ k: ['x', new Unconvertible()] })).toThrow(TypeError);
    expect(counting.live).toBe(before);
    v.close();
    expect(counting.live).toBe(0);
  });

  it('frees the cloned siblings of a failed list write or insert', () => {
    const v = new Value(['a', ['b', 'c'], { d: 'e' }], alloc());
    const before = counting.live;
    expect(() => v.asList().set(0, new Unconvertible())).toThrow(TypeError);
    expect(() => v.asList().set(1, ['x', new Unconvertible()])).toThrow(TypeError);
    expect(() => v.asList().set(9, 'x')).toThrow(RangeError);
    expect(() => v.asList().insert(1, [new Unconvertible()])).toThrow(TypeError);
    expect(counting.live).toBe(before);
    expect(v.toJS()).toEqual(['a', ['b', 'c'], { d: 'e' }]);
    v.close();
    expect(counting.live).toBe(0);
  });

  it('leaves nothing after a list write and insert that succeed', () => {
    const v = new Value(['a', ['b']], alloc());
    v.asList().set(0, { k: [1] });
    v.asList().insert(1, 'mid');
    expect(v.toJS()).toEqual([{ k: [1] }, 'mid', ['b']]);
    v.close();
    expect(counting.live).toBe(0);
  });

  it('leaves nothing after a clone that succeeds', () => {
    const v = new Value({ a: [1, 'two'] }, alloc());
    v.clone().close();
    v.close();
    expect(counting.live).toBe(0);
  });
});
