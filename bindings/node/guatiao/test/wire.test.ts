// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The wire encoding and channels against the shared vectors every
// implementation reads: each value both ways, each malformed input
// refused, the frames the builders write, and a conversation with every
// refusal.

import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { BadValue, NotFound, Tag, Value, WrongKind, type Ref } from 'guatiao';
import { MAX_DEPTH, jsToWire, wireToJS } from 'guatiao/ffi';
import * as wire from 'guatiao/wire';

import { SIZE, WIRE_EVENT } from '../src/abi.js';
import { HEADER, REPO, readText } from './helpers.js';

type Tree = [string, ...unknown[]];

interface Vectors {
  values: { name: string; tree: Tree; hex: string }[];
  malformed: { name: string; hex: string; at: number }[];
  frames: { build: string; tree?: Tree; hex: string }[];
  conversation: {
    frame: string;
    event?: string;
    refusal?: string;
    channel?: number;
    kind?: number;
    at?: number;
    key?: string;
    tree?: Tree;
  }[];
}

const VECTORS = JSON.parse(readText(path.join(REPO, 'crates', 'guatiao', 'tests', 'vectors', 'wire.json'))) as Vectors;

const fromHex = (text: string): Uint8Array => Uint8Array.from(Buffer.from(text, 'hex'));
const toHex = (bytes: Uint8Array): string => Buffer.from(bytes).toString('hex');

/** The channel a frame names, read from its bytes: JSON numbers lose 64-bit channels. */
function channelOf(frame: Uint8Array): bigint {
  let n = 0n;
  let shift = 0n;
  for (let i = 1; i < frame.length; i++) {
    n |= BigInt(frame[i]! & 0x7f) << shift;
    if ((frame[i]! & 0x80) === 0) break;
    shift += 7n;
  }
  return n;
}

const NAMES: Record<number, string> = {
  [Tag.ABSENT]: 'absent',
  [Tag.NULL]: 'null',
  [Tag.BOOL]: 'bool',
  [Tag.NUMBER]: 'number',
  [Tag.STRING]: 'string',
  [Tag.BYTES]: 'bytes',
  [Tag.LIST]: 'list',
  [Tag.MAP]: 'map',
};

/** A node as the vector file writes a tree. */
function describeNode(ref: Ref): unknown[] {
  const tag = NAMES[ref.tag]!;
  switch (tag) {
    case 'absent':
    case 'null':
      return [tag];
    case 'bool':
      return [tag, ref.boolOr()];
    case 'number':
      return [tag, ref.toJS({ numbers: 'text' })];
    case 'string':
      return [tag, ref.stringOr()];
    case 'bytes':
      return [tag, toHex(ref.toJS<Uint8Array>())];
    case 'list':
      return [tag, [...ref.asList()].map(describeNode)];
    default:
      return [tag, ref.asMap().entries().map(([k, v]) => [k, describeNode(v)])];
  }
}

/** The value a vector tree names. */
function build(tree: Tree): Value {
  const [tag, arg] = tree;
  switch (tag) {
    case 'absent':
      return Value.absent();
    case 'null':
      return Value.null();
    case 'bool':
      return Value.bool(arg as boolean);
    case 'number':
      return Value.number(arg as string);
    case 'string':
      return Value.string(arg as string);
    case 'bytes':
      return Value.bytes(fromHex(arg as string));
    case 'list': {
      const value = Value.list();
      for (const item of arg as Tree[]) {
        using child = build(item);
        value.asList().push(child);
      }
      return value;
    }
    default: {
      const value = Value.map();
      for (const [key, item] of arg as [string, Tree][]) {
        using child = build(item);
        value.asMap().set(key, child);
      }
      return value;
    }
  }
}

describe('values on the wire', () => {
  it.each(VECTORS.values.map((v) => [v.name, v] as const))('%s round-trips', (_name, v) => {
    const data = fromHex(v.hex);
    using value = Value.fromWire(data);
    expect(describeNode(value)).toEqual(v.tree);
    expect(toHex(value.toWire())).toBe(v.hex);
    using built = build(v.tree);
    expect(toHex(built.toWire())).toBe(v.hex);
  });

  it.each(VECTORS.malformed.map((v) => [v.name, v] as const))('%s is refused', (_name, v) => {
    expect(() => Value.fromWire(fromHex(v.hex))).toThrow(BadValue);
  });

  it('encodes a child alone', () => {
    using value = new Value({ outer: { inner: [1, 'two'] } });
    using back = Value.fromWire(value.asMap().get('outer')!.toWire());
    expect(back.toJS()).toEqual({ inner: [1, 'two'] });
  });
});

describe('wire bytes read and written in JavaScript', () => {
  const modes = (['auto', 'bigint', 'text'] as const).flatMap((numbers) =>
    (['object', 'map'] as const).map((maps) => ({ numbers, maps })),
  );

  it.each(VECTORS.values.map((v) => [v.name, v] as const))('%s reads as the tree does', (_name, v) => {
    const data = fromHex(v.hex);
    for (const options of modes) {
      using value = Value.fromWire(data);
      const expected = value.toJS(options);
      const got = wireToJS(data, options);
      expect(got, JSON.stringify(options)).toStrictEqual(expected);
      using back = new Value(got);
      expect(toHex(jsToWire(got))).toBe(toHex(back.toWire()));
    }
  });

  it('refuses bytes cut short, and bytes after the value', () => {
    using value = new Value({ a: [1, 'two', new Uint8Array([0])], 'ключ': 'значение' });
    const data = value.toWire();
    for (let end = 0; end < data.length; end++) {
      expect(() => wireToJS(data.subarray(0, end)), `cut at ${end}`).toThrow(BadValue);
    }
    expect(() => wireToJS(Uint8Array.from([...data, 1]))).toThrow(BadValue);
    expect(() => wireToJS(Uint8Array.of(9))).toThrow(BadValue);
    expect(() => wireToJS(Uint8Array.of(4, 1, 0xff))).toThrow(BadValue);
  });

  it('writes text of every length as the tree does', () => {
    // 43 three-byte characters are 129 bytes: a two-byte length after a
    // guess of one.
    for (const text of ['', 'a', 'é', '€'.repeat(42), '€'.repeat(43), 'x'.repeat(63), 'x'.repeat(64), 'ü'.repeat(70_000), '😀'.repeat(40)]) {
      using value = new Value({ [text]: text });
      const data = jsToWire({ [text]: text });
      expect(toHex(data)).toBe(toHex(value.toWire()));
      expect(wireToJS(data)).toStrictEqual({ [text]: text });
    }
  });

  it('refuses what new Value refuses, and writes a Ref as its node', () => {
    class Point {
      x = 1;
    }
    const cyclic: unknown[] = [];
    cyclic.push(cyclic);
    for (const [bad, error] of [
      [Number.NaN, RangeError],
      [new Map([[1, 2]]), TypeError],
      [Symbol('s'), TypeError],
      [() => 1, TypeError],
      [new Point(), TypeError],
      [cyclic, TypeError],
      ['\ud800', TypeError],
      [{ '\udc00': 1 }, TypeError],
    ] as const) {
      expect(() => new Value(bad)).toThrow(error);
      expect(() => jsToWire(bad)).toThrow(error);
    }
    using inner = new Value({ x: [true, null] });
    using whole = new Value({ ref: { x: [true, null] } });
    expect(toHex(jsToWire({ ref: inner }))).toBe(toHex(whole.toWire()));
    // A key named `__proto__` is data both ways.
    const odd: unknown = JSON.parse('{"__proto__": 1, "constructor": 2}');
    const read = wireToJS<Record<string, unknown>>(jsToWire(odd));
    expect(read).toStrictEqual(odd);
    expect(Object.getPrototypeOf(read)).toBe(Object.prototype);
  });

  it(`nests ${MAX_DEPTH} deep and no deeper`, () => {
    const nest = (depth: number): unknown => {
      let v: unknown = [];
      for (let i = 1; i < depth; i++) v = [v];
      return v;
    };
    // A non-empty innermost list, so the reader opens every level.
    const deepest = (depth: number): unknown => {
      let v: unknown = [1];
      for (let i = 1; i < depth; i++) v = [v];
      return v;
    };
    const ok = jsToWire(deepest(MAX_DEPTH));
    using value = Value.fromWire(ok);
    expect(wireToJS(ok)).toStrictEqual(value.toJS());
    expect(() => jsToWire(nest(MAX_DEPTH + 1))).toThrow(BadValue);
    const tooDeep = Uint8Array.from([...Array<number>(MAX_DEPTH + 1).fill(0).flatMap(() => [6, 1]), 1]);
    expect(() => wireToJS(tooDeep)).toThrow(BadValue);
  });
});

describe('channels', () => {
  it('writes the documented frames', () => {
    for (const f of VECTORS.frames) {
      const expected = fromHex(f.hex);
      const channel = channelOf(expected);
      if (f.build === 'close') {
        expect(toHex(wire.close(channel))).toBe(f.hex);
        continue;
      }
      using tree = build(f.tree!);
      const built = f.build === 'announce' ? wire.announce(channel, tree) : wire.send(channel, tree);
      expect(toHex(built)).toBe(f.hex);
    }
    expect(toHex(wire.send(7, { greeting: 'hello' }))).toBe(VECTORS.frames[1]!.hex);
  });

  it('runs the conversation through one receiver', () => {
    const classes = {
      malformed: [wire.Malformed, BadValue],
      no_schema: [wire.NoSchema, NotFound],
      not_a_schema: [wire.NotASchema, WrongKind],
      invalid: [wire.Invalid, BadValue],
    } as const;
    using receiver = new wire.Receiver();
    for (const step of VECTORS.conversation) {
      const frame = fromHex(step.frame);
      const channel = BigInt(step.channel ?? 0);
      if (step.event !== undefined) {
        const event = receiver.accept(frame);
        expect(Object.entries(wire.Frame).find(([, v]) => v === event.kind)?.[0].toLowerCase()).toBe(step.event);
        expect(event.channel).toBe(channel);
        if (step.tree) {
          using value = event.value!;
          expect(describeNode(value)).toEqual(step.tree);
        } else {
          expect(event.value).toBeNull();
        }
        continue;
      }
      let caught: unknown;
      try {
        receiver.accept(frame);
      } catch (e) {
        caught = e;
      }
      const [refusal, status] = classes[step.refusal as keyof typeof classes];
      expect(caught, JSON.stringify(step)).toBeInstanceOf(refusal);
      expect(caught).toBeInstanceOf(status);
      const error = caught as wire.Refused;
      expect(error.channel).toBe(channel);
      expect(error.message).not.toBe('');
      expect(String(error)).not.toContain('hunter2');
      if (step.kind !== undefined) expect(error.kind).toBe(step.kind);
      if (step.at !== undefined) expect((error as wire.Malformed).at).toBe(step.at);
      if (step.key !== undefined) expect((error as wire.Invalid).key).toBe(step.key);
    }
    receiver.close();
    receiver.close();
    expect(() => receiver.accept(wire.close(1))).toThrow(TypeError);
  });

  it('lays out the event and the codes as the header does', () => {
    const header = readText(HEADER);
    for (const [name, code] of Object.entries(wire.Frame)) {
      expect(header).toContain(`#define GUATIAO_FRAME_${name} ${code}\n`);
    }
    for (const [name, code] of Object.entries(wire.Refusal)) {
      expect(header).toContain(`#define GUATIAO_WIRE_${name} ${code}\n`);
    }
    expect(WIRE_EVENT.message - WIRE_EVENT.value).toBe(SIZE.value);
    expect(WIRE_EVENT.detail + SIZE.value).toBe(SIZE.wireEvent);
  });
});
