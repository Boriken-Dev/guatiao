// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Every value kind round-trips through a real build of guatiao.

import { describe, expect, it } from 'vitest';

import { ListRef, MapRef, Ref, Tag, Value } from '../src/index.js';

describe('Value', () => {
  it('round-trips every kind', () => {
    expect(Value.null().toJS()).toBe(null);
    expect(Value.bool(true).toJS()).toBe(true);
    expect(Value.bool(false).toJS()).toBe(false);
    expect(Value.string('hola').toJS()).toBe('hola');
    expect(Value.bytes(new Uint8Array([0, 1, 255])).toJS()).toEqual(new Uint8Array([0, 1, 255]));
    expect(Value.absent().toJS()).toBe(undefined);
    const nested = new Value({ a: [1, 2, { b: 'c' }], d: null });
    expect(nested.toJS()).toEqual({ a: [1, 2, { b: 'c' }], d: null });
    nested.close();
  });

  it('constructs from what plain data holds', () => {
    for (const plain of [1, true, false, 1.5, 'x', null, [1, 2], { host: 'h', tags: ['a'] }]) {
      using v = new Value(plain);
      expect(v.toJS()).toEqual(plain);
    }
    using absent = new Value();
    expect(absent.isAbsent()).toBe(true);
    using fromMap = new Value(new Map([['a', 1]]));
    expect(fromMap.toJS()).toEqual({ a: 1 });
    using bytes = new Value(new Uint16Array([1]).buffer);
    expect(bytes.tag).toBe(Tag.BYTES);
  });

  it('refuses what has no value form', () => {
    expect(() => new Value(Number.NaN)).toThrow(RangeError);
    expect(() => new Value(Number.POSITIVE_INFINITY)).toThrow(RangeError);
    expect(() => new Value(new Date())).toThrow(TypeError);
    expect(() => new Value(() => 1)).toThrow(TypeError);
    expect(() => new Value(Symbol('x'))).toThrow(TypeError);
    expect(() => new Value(new Map([[1, 'one']]))).toThrow(TypeError);
    expect(() => new Value('\ud800')).toThrow(TypeError);
    const cycle: Record<string, unknown> = {};
    cycle.self = cycle;
    expect(() => new Value(cycle)).toThrow(TypeError);
  });

  it('keeps a number exact, and reads it as asked', () => {
    using v = Value.number('1.10');
    expect(v.toJS({ numbers: 'text' })).toBe('1.10');
    expect(v.toJS()).toBe(1.1);
    using big = new Value(2n ** 64n - 1n);
    expect(big.toJS()).toBe(2n ** 64n - 1n);
    expect(big.toJS({ numbers: 'text' })).toBe('18446744073709551615');
    using small = new Value(5900);
    expect(small.toJS()).toBe(5900);
    expect(small.toJS({ numbers: 'bigint' })).toBe(5900n);
    using negativeZero = new Value(-0);
    expect(negativeZero.toJS({ numbers: 'text' })).toBe('-0');
    const digits = 7n * (10n ** 200n - 1n) / 9n;
    using huge = new Value(digits);
    expect(huge.toJS()).toBe(digits);
  });

  it('reads one field with the header rules', () => {
    using root = new Value({ n: 5, s: 'hi', b: true, f: 1.5, big: 2n ** 63n });
    const m = root.asMap();
    expect(m.get('n')!.intOr(-1n)).toBe(5n);
    expect(m.get('s')!.intOr(-1n)).toBe(-1n);
    expect(m.get('f')!.intOr(-1n)).toBe(-1n);
    expect(m.get('big')!.intOr(-1n)).toBe(-1n);
    expect(m.get('f')!.numberOr(0)).toBe(1.5);
    expect(m.get('s')!.stringOr('')).toBe('hi');
    expect(m.get('b')!.boolOr(false)).toBe(true);
    expect(m.get('missing')).toBe(undefined);
    const missing = new Ref(root, ['missing']);
    expect(missing.isAbsent()).toBe(true);
    expect(missing.intOr(-1n)).toBe(-1n);
  });

  it('keeps a ref valid while the map grows past a reallocation', () => {
    using root = Value.map();
    const m = root.asMap();
    m.set('a', 'first');
    const ref = m.get('a')!;
    for (let i = 0; i < 64; i++) m.set(`k${i}`, i);
    expect(ref.toJS()).toBe('first');
    expect(m.size).toBe(65);
  });

  it('refuses use after close, and closes twice quietly', () => {
    const v = Value.string('x');
    v.close();
    expect(() => v.toJS()).toThrow(TypeError);
    v.close();
    expect(v.closed).toBe(true);
  });

  it('speaks the map protocol', () => {
    using root = Value.map();
    const m: MapRef = root.asMap();
    m.set('one', 1).set('two', 2);
    expect(m.keys()).toEqual(['one', 'two']);
    expect(m.get('one')!.toJS()).toBe(1);
    expect(m.has('one')).toBe(true);
    expect(m.delete('one')).toBe(true);
    expect(m.delete('one')).toBe(false);
    expect([...m].map(([k]) => k)).toEqual(['two']);
    m.clear();
    expect(m.size).toBe(0);
  });

  it('copies one map into another', () => {
    using src = new Value({ a: 1, b: 2 });
    using dst = new Value({ a: 0, c: 3 });
    dst.asMap().copyFrom(src);
    expect(dst.toJS()).toEqual({ a: 1, c: 3, b: 2 });
  });

  it('speaks the list protocol', () => {
    using root = Value.list();
    const list: ListRef = root.asList();
    expect(list.push(1, 2, 3)).toBe(3);
    expect([...list].map((r) => r.toJS())).toEqual([1, 2, 3]);
    list.removeAt(1);
    expect(root.toJS()).toEqual([1, 3]);
    list.insert(1, 'mid');
    expect(root.toJS()).toEqual([1, 'mid', 3]);
    list.set(0, 'first');
    list.set(-1, 'last');
    expect(root.toJS()).toEqual(['first', 'mid', 'last']);
    expect(list.at(-1)!.toJS()).toBe('last');
    expect(list.at(9)).toBe(undefined);
    expect(() => list.removeAt(9)).toThrow(RangeError);
    list.clear();
    expect(list.length).toBe(0);
  });

  it('clones into an independent value', () => {
    using src = new Value([1, 2, 3]);
    using copy = src.clone();
    copy.asList().push(4);
    expect(src.toJS()).toEqual([1, 2, 3]);
    expect(copy.toJS()).toEqual([1, 2, 3, 4]);
  });

  it('copies a nested value in', () => {
    const inner = new Value({ a: 'b' });
    using v = new Value({ host: 'h', test: inner, more: [new Value(1)] });
    inner.asMap().set('a', 'changed');
    inner.close();
    expect(v.toJS()).toEqual({ host: 'h', test: { a: 'b' }, more: [1] });
  });

  it('reads a key named __proto__ as data, and a map as a Map when asked', () => {
    using v = new Value(new Map([['__proto__', 1], ['b', 2]]));
    const plain = v.toJS<Record<string, unknown>>();
    expect(Object.getPrototypeOf(plain)).toBe(Object.prototype);
    expect(Object.keys(plain)).toEqual(['__proto__', 'b']);
    expect(v.toJS({ maps: 'map' })).toEqual(new Map([['__proto__', 1], ['b', 2]]));
  });

  it('refuses the wrong container', () => {
    using v = new Value([1]);
    expect(() => v.asMap()).toThrow(TypeError);
    expect(v.asList().length).toBe(1);
  });
});
