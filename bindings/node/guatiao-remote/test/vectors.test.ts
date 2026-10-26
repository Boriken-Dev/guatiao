// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The remote protocol's shared vectors: every frame both ways, every
// argument list and answer typed by the greeter kinds' descriptions,
// every malformed body refused where the file says, every proof.

import { describe, expect, test } from 'vitest';

import { ObjectRef, decodeAnswer, decodeArgs, encodeAnswer, encodeArgs, isTree } from '../src/args.js';
import {
  Call,
  Cancel,
  Close,
  Frame,
  FrameReader,
  FrameWriter,
  Goodbye,
  Hello,
  ListCatalog,
  MalformedFrame,
  Open,
  Ping,
  Pong,
  Proof,
  Release,
  Reply,
  Target,
  Welcome,
  bigOf,
} from '../src/codec.js';
import { BadValue } from '../src/errors.js';
import { CLIENT_LABEL, SERVER_LABEL, proofOf, proves } from '../src/proof.js';
import { fromWire, toWire } from '../src/wire.js';
import { hex, kinds, toHex, vectors, wireVectors } from './support.js';

type Case = Record<string, unknown>;
const list = (key: string): Case[] => vectors[key] as Case[];

/** An integer the file writes, quoted when wide. */
function int(v: unknown): number | bigint {
  if (typeof v === 'string') {
    const big = BigInt(v);
    return big >= BigInt(Number.MIN_SAFE_INTEGER) && big <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(big) : big;
  }
  return v as number;
}

/** The plain data a vector tree names. */
function tree(t: unknown[]): unknown {
  switch (t[0]) {
    case 'absent':
      return undefined;
    case 'null':
      return null;
    case 'bool':
      return t[1] as boolean;
    case 'number': {
      const text = t[1] as string;
      if (/[.eE]/.test(text)) return Number(text);
      return int(text);
    }
    case 'string':
      return t[1] as string;
    case 'bytes':
      return hex(t[1] as string);
    case 'list':
      return (t[1] as unknown[][]).map(tree);
    default: {
      const out: Record<string, unknown> = {};
      for (const [k, v] of t[1] as [string, unknown[]][]) out[k] = tree(v);
      return out;
    }
  }
}

const TRUST: Record<string, number> = { local: 0, trusted: 1, secure: 2 };

function frameOf(f: Case): Frame {
  switch (f['type']) {
    case 'hello':
      return new Hello(TRUST[f['trust'] as string]!, hex(f['nonce'] as string), tree(f['options'] as unknown[]));
    case 'welcome':
      return new Welcome(hex(f['nonce'] as string), hex(f['proof'] as string), tree(f['options'] as unknown[]));
    case 'proof':
      return new Proof(hex(f['proof'] as string));
    case 'list':
      return new ListCatalog(f['call'] as number);
    case 'open':
      return new Open(f['call'] as number, f['library'] as string, f['stamp'] as string);
    case 'close':
      return new Close(f['library'] as string, f['stamp'] as string);
    case 'call': {
      const [kind, id] = f['target'] as [string, number];
      return new Call(
        f['call'] as number,
        f['parent'] as number,
        kind === 'table' ? Target.table(id) : Target.object(id),
        f['slot'] as number,
        hex(f['args'] as string),
      );
    }
    case 'reply':
      return new Reply(f['call'] as number, f['status'] as number, hex(f['body'] as string));
    case 'cancel':
      return new Cancel(f['call'] as number);
    case 'release':
      return new Release(f['object'] as number);
    case 'ping':
      return new Ping(f['n'] as number);
    case 'pong':
      return new Pong(f['n'] as number);
    case 'goodbye':
      return new Goodbye(f['status'] as number, f['message'] as string);
  }
  throw new Error(`a frame type the vectors do not define: ${String(f['type'])}`);
}

/** A vector's argument or answer as the codec takes it. */
function valueOf(a: unknown[]): unknown {
  switch (a[0]) {
    case 'str':
    case 'text':
      return a[1] as string;
    case 'int':
    case 'uint':
      return int(a[1]);
    case 'bool':
      return a[1] as boolean;
    case 'buffer':
      return typeof a[1] === 'string' ? hex(a[1]) : new Uint8Array(a[1] as number);
    case 'map':
      return tree(a[1] as unknown[]);
    case 'object':
      return new ObjectRef(a[2] as number, a[1] === 'receivers');
    case 'nothing':
      return undefined;
  }
  throw new Error(`an argument form the vectors do not define: ${String(a[0])}`);
}

/** Trees compared by their wire bytes, everything else as it is. */
function same(got: unknown, want: unknown, name: string): void {
  if (typeof want === 'object' && want !== null && !(want instanceof Uint8Array) && !(want instanceof ObjectRef)) {
    expect(toHex(toWire(got)), name).toBe(toHex(toWire(want)));
  } else {
    expect(got, name).toEqual(want);
  }
}

describe('the shared vectors', () => {
  test('every frame decodes to its fields and encodes to its bytes', () => {
    const cases = list('frames');
    expect(cases).toHaveLength(17);
    for (const c of cases) {
      const name = c['name'] as string;
      const whole = hex(c['hex'] as string);
      const length = new DataView(whole.buffer).getUint32(0, true);
      const body = whole.subarray(4);
      expect(length, name).toBe(body.length);
      const expected = frameOf(c['frame'] as Case);
      expect(toHex(expected.encode()), name).toBe(toHex(body));
      const decoded = Frame.decode(body);
      expect(decoded.constructor, name).toBe(expected.constructor);
      expect(toHex(decoded.encode()), name).toBe(toHex(body));
      if (decoded instanceof Hello && expected instanceof Hello) {
        expect(decoded.trust, name).toBe(expected.trust);
        same(decoded.options, expected.options, name);
      } else if (decoded instanceof Welcome && expected instanceof Welcome) {
        expect(decoded.proof, name).toEqual(expected.proof);
        same(decoded.options, expected.options, name);
      } else if (decoded instanceof Call && expected instanceof Call) {
        expect([decoded.call, decoded.parent, decoded.target.code, decoded.slot], name).toEqual([
          expected.call,
          expected.parent,
          expected.target.code,
          expected.slot,
        ]);
      } else if (decoded instanceof Open && expected instanceof Open) {
        expect([decoded.call, decoded.library, decoded.stamp]).toEqual([expected.call, expected.library, expected.stamp]);
      } else if (decoded instanceof Goodbye && expected instanceof Goodbye) {
        expect([decoded.status, decoded.message], name).toEqual([expected.status, expected.message]);
      }
    }
  });

  test('every malformed body is refused where the file says', () => {
    const cases = list('malformed');
    expect(cases).toHaveLength(8);
    for (const c of cases) {
      const name = c['name'] as string;
      let refused: unknown;
      try {
        Frame.decode(hex(c['body'] as string));
      } catch (e) {
        refused = e;
      }
      expect(refused, `accepted a body the vectors refuse: ${name}`).toBeInstanceOf(MalformedFrame);
      const e = refused as MalformedFrame;
      expect(e.fault, name).toBe(c['error']);
      expect(e, name).toBeInstanceOf(BadValue);
      if ('at' in c) expect(e.at, name).toBe(c['at']);
    }
  });

  const slotOf = (c: Case) => kinds.get(c['kind'] as string)!.slot(c['slot'] as string);

  test('every argument list encodes to its bytes and decodes to its values', () => {
    const cases = list('args');
    expect(cases).toHaveLength(11);
    for (const c of cases) {
      const name = c['name'] as string;
      const slot = slotOf(c);
      const args = (c['args'] as unknown[][]).map(valueOf);
      expect(toHex(encodeArgs(slot, args)), name).toBe(c['hex']);
      const back = decodeArgs(slot, hex(c['hex'] as string));
      expect(back, name).toHaveLength(args.length);
      args.forEach((want, i) => {
        if (want instanceof Uint8Array) expect((back[i] as Uint8Array).length, name).toBe(want.length);
        else same(back[i], want, name);
      });
    }
  });

  test('every answer encodes to its bytes and decodes with its write-backs', () => {
    const cases = list('answers');
    expect(cases).toHaveLength(7);
    for (const c of cases) {
      const name = c['name'] as string;
      const slot = slotOf(c);
      const answer = valueOf(c['answer'] as unknown[]);
      const writebacks = (c['writebacks'] as unknown[][]).map(valueOf);
      // What the callee holds after the call: its write-backs in the places
      // of the lent trees and buffers.
      const args: unknown[] = slot.args.map(() => undefined);
      let w = 0;
      slot.args.forEach((a, i) => {
        if ((isTree(a.type) && a.pass === 'lent') || a.type === 'bytes_mut') args[i] = writebacks[w++];
      });
      expect(toHex(encodeAnswer(slot, answer, args)), name).toBe(c['hex']);
      const back = decodeAnswer(slot, hex(c['hex'] as string));
      same(back.answer, answer, name);
      expect(back.writebacks, name).toHaveLength(writebacks.length);
      writebacks.forEach((want, i) => same(back.writebacks[i], want, name));
    }
  });

  test('every proof is the HMAC-SHA256 the file holds', () => {
    const cases = list('proofs');
    expect(cases).toHaveLength(3);
    for (const c of cases) {
      const name = c['name'] as string;
      const key = hex(c['key'] as string);
      const client = hex(c['client'] as string);
      const server = hex(c['server'] as string);
      const label = c['label'] as string;
      const proof = proofOf(key, label, client, server);
      expect(toHex(proof), name).toBe(c['hex']);
      expect(proves(key, label, client, server, proof), name).toBe(true);
      const wrong = proof.slice();
      wrong[0]! ^= 1;
      expect(proves(key, label, client, server, wrong), name).toBe(false);
      expect(proves(key, label, server, client, proof), name).toBe(false);
    }
    expect(SERVER_LABEL).toBe('guatiao-remote server');
    expect(CLIENT_LABEL).toBe('guatiao-remote client');
  });
});

test('integers keep every bit at the 64-bit limits', () => {
  const edges: (number | bigint)[] = [0, 127, 128, 2 ** 33, 9007199254740991, 9007199254740993n, 18446744073709551615n];
  for (const n of edges) {
    const r = new FrameReader(new FrameWriter().uint(n).done());
    expect(bigOf(r.uint()), String(n)).toBe(bigOf(n));
    r.end();
  }
  const signed: (number | bigint)[] = [-1, -9007199254740991, -9223372036854775808n, 9223372036854775807n, -(2n ** 62n)];
  for (const n of signed) {
    expect(bigOf(new FrameReader(new FrameWriter().int(n).done()).int()), String(n)).toBe(bigOf(n));
  }
  // Past 2^53 a number is no longer exact, though 2^60 is one: it reads back a bigint.
  const read = new FrameReader(new FrameWriter().uint(2n ** 60n).done()).uint();
  expect(read).toBe(2n ** 60n);
  expect(new FrameReader(new FrameWriter().uint(2 ** 53 - 1).done()).uint()).toBe(2 ** 53 - 1);
  expect(() => new FrameWriter().uint(-1)).toThrow(RangeError);
  expect(() => new FrameWriter().uint(2n ** 64n)).toThrow(RangeError);
  expect(() => new FrameWriter().int(2n ** 63n)).toThrow(RangeError);
  expect(() => new FrameWriter().uint(1.5)).toThrow(RangeError);
});

describe("the wire encoding's shared vectors", () => {
  type WireCase = { name: string; hex: string; tree?: unknown[]; at?: number };
  const wire = wireVectors as { values: WireCase[]; malformed: WireCase[] };

  test('every value decodes to its tree, and encodes back to its bytes', () => {
    expect(wire.values).toHaveLength(21);
    for (const c of wire.values) {
      const got = fromWire(hex(c.hex));
      same(got, tree(c.tree!), c.name);
      // A number's text with a fraction comes back as a `number`, which
      // keeps its value, not its spelling (`1.10` is 1.1).
      if (!['a number keeps its text', 'a negative exponent'].includes(c.name)) {
        expect(toHex(toWire(got)), c.name).toBe(c.hex);
      }
    }
  });

  test('every malformed value is refused, naming the byte', () => {
    expect(wire.malformed).toHaveLength(12);
    for (const c of wire.malformed) {
      let refused: unknown;
      try {
        fromWire(hex(c.hex));
      } catch (e) {
        refused = e;
      }
      expect(refused, `accepted bytes the vectors refuse: ${c.name}`).toBeInstanceOf(BadValue);
      expect((refused as Error).message, c.name).toMatch(new RegExp(` at ${c.at}$`));
    }
  });

  test('a leading U+FEFF is text, kept as text', () => {
    expect(fromWire(toWire('﻿abc'))).toBe('﻿abc');
    expect(Object.keys(fromWire(toWire({ '﻿k': 1 })) as object)).toEqual(['﻿k']);
    expect(new FrameReader(new FrameWriter().text('﻿x').done()).text()).toBe('﻿x');
  });
});

test('the source names nothing of Node', async () => {
  // A page imports this package: no `node:` module, no `process`, no `Buffer`.
  const sources = import.meta.glob('../src/*.ts', { query: '?raw', import: 'default', eager: true });
  expect(Object.keys(sources).length).toBeGreaterThan(5);
  for (const [file, text] of Object.entries(sources)) {
    expect(text as string, file).not.toMatch(/from 'node:|\bprocess\.|\bBuffer\b|require\(/);
  }
});
