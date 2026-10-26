// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A call's arguments and a reply's answer, typed by the slot's
 * description: nothing on the wire repeats what both sides know.
 *
 * ```text
 * bool            1 byte, 0 or 1
 * i8..i64, isize  int (zigzag LEB128)    u8..u64, usize  uint
 * f32, f64        4 or 8 bytes, little-endian IEEE 754
 * str, text       text                   bytes  bytes
 * bytes_mut       uint: its length; the reply writes back bytes
 * value/map/list  value (a nullable value that is absent: ABSENT)
 * object          uint: id << 1, | 1 when the receiver's own
 *
 * reply body := answer? writeback*   one per lent tree and per bytes_mut
 * ```
 *
 * @module
 */

import { FrameReader, FrameWriter, MalformedFrame, bigOf } from './codec.js';
import type { RemoteSlot } from './described.js';
import { isPlainObject } from './wire.js';

/** An object as it crosses: `id` in the numbering of the side that holds it, `receivers` when that side is the receiver. */
export class ObjectRef {
  constructor(
    readonly id: number,
    readonly receivers = false,
  ) {}

  get code(): number {
    return this.receivers ? this.id * 2 + 1 : this.id * 2;
  }

  static of(code: number): ObjectRef {
    return code % 2 === 1 ? new ObjectRef((code - 1) / 2, true) : new ObjectRef(code / 2, false);
  }

  toString(): string {
    return this.receivers ? `the receiver's object ${this.id}` : `object ${this.id}`;
  }
}

/** The bounds of each integer type. */
const RANGES: ReadonlyMap<string, readonly [bigint, bigint]> = (() => {
  const out = new Map<string, readonly [bigint, bigint]>();
  for (const bits of [8n, 16n, 32n, 64n]) {
    out.set(`i${bits}`, [-(1n << (bits - 1n)), (1n << (bits - 1n)) - 1n]);
    out.set(`u${bits}`, [0n, (1n << bits) - 1n]);
  }
  out.set('isize', out.get('i64')!);
  out.set('usize', out.get('u64')!);
  return out;
})();

const signed = (type: string): boolean => type.startsWith('i');

/** Whether `type` is a tree: `value`, `map` or `list`. */
export const isTree = (type: string): boolean => type === 'value' || type === 'map' || type === 'list';

/** A plain map: an object literal or a `Map`. */
export function isMap(v: unknown): v is Record<string, unknown> | Map<string, unknown> {
  return v instanceof Map || (typeof v === 'object' && v !== null && !Array.isArray(v) && isPlainObject(v));
}

function mismatch(slot: RemoteSlot, i: number, what: string): TypeError {
  const arg = slot.args[i]!;
  const name = arg.name === '' ? '' : ` (\`${arg.name}\`)`;
  return new TypeError(`argument ${i}${name} of \`${slot.name}\` is not ${what}, which the slot takes`);
}

function putScalar(out: FrameWriter, type: string, v: unknown, wrong: (what: string) => TypeError): void {
  if (type === 'bool') {
    if (typeof v !== 'boolean') throw wrong('a boolean');
    out.byte(v ? 1 : 0);
  } else if (type === 'f32' || type === 'f64') {
    if (typeof v !== 'number') throw wrong('a number');
    const b = new DataView(new ArrayBuffer(8));
    if (type === 'f32') {
      b.setFloat32(0, v, true);
      out.raw(new Uint8Array(b.buffer, 0, 4));
    } else {
      b.setFloat64(0, v, true);
      out.raw(new Uint8Array(b.buffer));
    }
  } else {
    const range = RANGES.get(type);
    if (range === undefined) throw wrong('of a type these tables carry');
    const big = bigOf(v);
    if (big === undefined) throw wrong('an integer');
    if (big < range[0] || big > range[1]) throw wrong(`within ${type}`);
    const n = v as number | bigint;
    if (signed(type)) out.int(n);
    else out.uint(n);
  }
}

function getScalar(r: FrameReader, type: string): unknown {
  switch (type) {
    case 'bool': {
      const at = r.at;
      const b = r.byte();
      if (b > 1) throw new MalformedFrame('field', `the field at ${at} is not a bool`, at);
      return b === 1;
    }
    case 'f32':
      return new DataView(r.fixed(4).buffer).getFloat32(0, true);
    case 'f64':
      return new DataView(r.fixed(8).buffer).getFloat64(0, true);
  }
  if (!RANGES.has(type)) {
    throw new MalformedFrame('field', `the field at ${r.at} is not a scalar these tables carry`, r.at);
  }
  return signed(type) ? r.int() : r.uint();
}

function checkTree(type: string, v: unknown, wrong: (what: string) => TypeError): void {
  if (type === 'map' && !isMap(v)) throw wrong('a map');
  if (type === 'list' && !Array.isArray(v)) throw wrong('a list');
}

function tree(r: FrameReader, type: string): unknown {
  const at = r.at;
  const v = r.value();
  if (type === 'map' && !isMap(v)) throw new MalformedFrame('field', `the field at ${at} is not a map`, at);
  if (type === 'list' && !Array.isArray(v)) throw new MalformedFrame('field', `the field at ${at} is not a list`, at);
  return v;
}

/** The bytes of `args` as `slot` types them; an object is its `ObjectRef`. */
export function encodeArgs(slot: RemoteSlot, args: readonly unknown[]): Uint8Array {
  if (args.length !== slot.args.length) {
    throw new TypeError(`${args.length} arguments for \`${slot.name}\`, which takes ${slot.args.length}`);
  }
  const out = new FrameWriter();
  args.forEach((v, i) => {
    const type = slot.args[i]!.type;
    const wrong = (what: string): TypeError => mismatch(slot, i, what);
    switch (type) {
      case 'str':
        if (typeof v !== 'string') throw wrong('text');
        out.text(v);
        break;
      case 'bytes':
        if (!(v instanceof Uint8Array)) throw wrong('a Uint8Array');
        out.bytes(v);
        break;
      case 'bytes_mut':
        if (!(v instanceof Uint8Array)) throw wrong('a Uint8Array to write into');
        out.uint(v.length);
        break;
      case 'value':
      case 'map':
      case 'list':
        checkTree(type, v, wrong);
        out.value(v);
        break;
      case 'object':
        if (!(v instanceof ObjectRef)) throw wrong('an object');
        out.uint(v.code);
        break;
      default:
        putScalar(out, type, v, wrong);
    }
  });
  return out.done();
}

/** The largest buffer a caller may ask a callee to fill: 64 MiB. */
export const MAX_BUFFER = 64 << 20;

/**
 * A CALL's arguments as `slot` types them: a buffer as a zeroed
 * `Uint8Array` of its length, an object as its `ObjectRef`, an absent
 * nullable value as `undefined`.
 */
export function decodeArgs(slot: RemoteSlot, bytes: Uint8Array): unknown[] {
  const r = new FrameReader(bytes);
  const out: unknown[] = [];
  for (const arg of slot.args) {
    switch (arg.type) {
      case 'str':
        out.push(r.text());
        break;
      case 'bytes':
        out.push(r.bytes());
        break;
      case 'bytes_mut': {
        const at = r.at;
        const n = r.uint();
        if (typeof n !== 'number' || n > MAX_BUFFER) {
          throw new MalformedFrame('field', `the field at ${at} is not a buffer length within the link's limit`, at);
        }
        out.push(new Uint8Array(n));
        break;
      }
      case 'value':
      case 'map':
      case 'list':
        out.push(tree(r, arg.type));
        break;
      case 'object':
        out.push(ObjectRef.of(r.small('an object')));
        break;
      default:
        out.push(getScalar(r, arg.type));
    }
  }
  r.end();
  return out;
}

/**
 * A reply's body: `answer` as `slot` types it (an object as its
 * `ObjectRef`), then what the call wrote into each lent tree and buffer
 * of `args`, in order.
 */
export function encodeAnswer(slot: RemoteSlot, answer: unknown, args: readonly unknown[]): Uint8Array {
  const out = new FrameWriter();
  const type = slot.answer?.type;
  const wrong = (what: string): TypeError => new TypeError(`\`${slot.name}\` answered something that is not ${what}`);
  switch (type) {
    case undefined:
      break;
    case 'text':
      if (typeof answer !== 'string') throw wrong('text');
      out.text(answer);
      break;
    case 'value':
    case 'map':
    case 'list':
      checkTree(type, answer, wrong);
      out.value(answer);
      break;
    case 'object':
      if (!(answer instanceof ObjectRef)) throw wrong('an object');
      out.uint(answer.code);
      break;
    default:
      putScalar(out, type, answer, wrong);
  }
  slot.args.forEach((arg, i) => {
    if (isTree(arg.type) && arg.pass === 'lent') {
      out.value(args[i]);
    } else if (arg.type === 'bytes_mut') {
      const buffer = args[i];
      if (!(buffer instanceof Uint8Array)) throw wrong(`a buffer in argument ${i}`);
      out.bytes(buffer);
    }
  });
  return out.done();
}

/**
 * A reply's body read as `slot` types it: the answer (`undefined` for a
 * slot that answers nothing, an `ObjectRef` for an object), then one
 * write-back per lent tree and per buffer, in the arguments' order.
 */
export function decodeAnswer(slot: RemoteSlot, body: Uint8Array): { answer: unknown; writebacks: unknown[] } {
  const r = new FrameReader(body);
  const type = slot.answer?.type;
  let answer: unknown;
  switch (type) {
    case undefined:
      answer = undefined;
      break;
    case 'text':
      answer = r.text();
      break;
    case 'value':
    case 'map':
    case 'list':
      answer = tree(r, type);
      break;
    case 'object':
      answer = ObjectRef.of(r.small('an object'));
      break;
    default:
      answer = getScalar(r, type);
  }
  const writebacks: unknown[] = [];
  for (const arg of slot.args) {
    if (isTree(arg.type) && arg.pass === 'lent') writebacks.push(tree(r, arg.type));
    else if (arg.type === 'bytes_mut') writebacks.push(r.bytes());
  }
  r.end();
  return { answer, writebacks };
}
