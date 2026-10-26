// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The wire encoding read into plain JavaScript data and written from it in
 * JavaScript, with no library: one crossing carries a whole document as
 * bytes, where reading or building a tree node by node costs a crossing
 * per node.
 *
 * Bytes a library wrote were checked as it wrote them; reading here
 * refuses a body cut short, bytes after the value, an unknown tag, text
 * that is not UTF-8 and nesting past {@link MAX_DEPTH}, and checks nothing
 * else.
 *
 * @module
 */

import { Status } from './abi.js';
import { BadValue } from './errors.js';
import { Ref, type ReadOptions, asBytes, isPlainObject, numberTextFor, readNumber } from './value.js';

const ABSENT = 0;
const NULL = 1;
const BOOL = 2;
const NUMBER = 3;
const STRING = 4;
const BYTES = 5;
const LIST = 6;
const MAP = 7;

/** How deep containers nest: what every walk in the library follows. */
export const MAX_DEPTH = 128;

const UTF8 = new TextDecoder('utf-8', { fatal: true });
const ENCODER = new TextEncoder();

const cutShort = (): BadValue => new BadValue(Status.ERR_BAD_VALUE, 'the bytes end inside a node');
const tooDeep = (): BadValue => new BadValue(Status.ERR_BAD_VALUE, `nested deeper than ${MAX_DEPTH}`);

/** `n` bytes from `at` as text; ASCII is read without a decoder. */
function textOf(data: Uint8Array, at: number, n: number): string {
  if (n < 32) {
    let s = '';
    for (let i = at; i < at + n; i++) {
      const c = data[i]!;
      if (c >= 0x80) return utf8(data, at, n);
      s += String.fromCharCode(c);
    }
    return s;
  }
  return utf8(data, at, n);
}

function utf8(data: Uint8Array, at: number, n: number): string {
  try {
    return UTF8.decode(data.subarray(at, at + n));
  } catch {
    throw new BadValue(Status.ERR_BAD_VALUE, 'text that is not UTF-8');
  }
}

/** An open container while reading: what it collects, and how many are left. */
interface Open {
  readonly list: unknown[] | null;
  readonly object: Record<string, unknown> | null;
  readonly map: Map<string, unknown> | null;
  left: number;
  /** The key it goes under in its parent. */
  readonly key: string;
}

function place(top: Open, key: string, value: unknown): void {
  if (top.list !== null) top.list.push(value);
  else if (top.map !== null) top.map.set(key, value);
  else if (key === '__proto__') {
    // Defined, not assigned: a key named `__proto__` is data here.
    Object.defineProperty(top.object, key, { value, enumerable: true, writable: true, configurable: true });
  } else top.object![key] = value;
}

/**
 * The plain data wire bytes hold, as `Value.fromWire(data).toJS(options)`
 * reads it, with no tree built. `BadValue` for bytes that are not one
 * value (see the module's note on what is checked).
 */
export function wireToJS<T = unknown>(data: Uint8Array, options: ReadOptions = {}): T {
  const numbers = options.numbers;
  if (numbers !== undefined && numbers !== 'auto' && numbers !== 'bigint' && numbers !== 'text') {
    throw new TypeError(`unknown numbers mode: ${String(numbers)}`);
  }
  const asMap = options.maps === 'map';
  const end = data.length;
  const stack: Open[] = [];
  let at = 0;
  let key = '';
  // One LEB128 count; `at` moves past it.
  const count = (): number => {
    if (at >= end) throw cutShort();
    let b = data[at++]!;
    if (b < 0x80) return b;
    let n = b & 0x7f;
    let scale = 0x80;
    for (;;) {
      if (at >= end) throw cutShort();
      b = data[at++]!;
      n += (b & 0x7f) * scale;
      if (b < 0x80) return n;
      scale *= 0x80;
      if (scale > 2 ** 56) throw new BadValue(Status.ERR_BAD_VALUE, 'a length past what this reader holds');
    }
  };
  for (;;) {
    const top = stack.length ? stack[stack.length - 1]! : null;
    if (top !== null && top.list === null) {
      const n = count();
      if (at + n > end) throw cutShort();
      key = textOf(data, at, n);
      at += n;
    }
    if (at >= end) throw cutShort();
    const tag = data[at++]!;
    let value: unknown;
    switch (tag) {
      case NUMBER:
      case STRING:
      case BYTES: {
        const n = count();
        if (at + n > end) throw cutShort();
        if (tag === STRING) value = textOf(data, at, n);
        else if (tag === NUMBER) value = readNumber(textOf(data, at, n), numbers);
        else value = new Uint8Array(data.subarray(at, at + n));
        at += n;
        break;
      }
      case LIST:
      case MAP: {
        const n = count();
        if (n > 0) {
          if (stack.length >= MAX_DEPTH) throw tooDeep();
          stack.push({
            list: tag === LIST ? [] : null,
            object: tag === MAP && !asMap ? {} : null,
            map: tag === MAP && asMap ? new Map() : null,
            left: n,
            key,
          });
          continue;
        }
        value = tag === LIST ? [] : asMap ? new Map() : {};
        break;
      }
      case NULL:
        value = null;
        break;
      case BOOL:
        if (at >= end) throw cutShort();
        value = data[at++] === 1;
        break;
      case ABSENT:
        value = undefined;
        break;
      default:
        throw new BadValue(Status.ERR_BAD_VALUE, `unknown tag ${tag} at ${at - 1}`);
    }
    // Place the value, closing every container it completes.
    for (;;) {
      const open = stack.length ? stack[stack.length - 1]! : null;
      if (open === null) {
        if (at !== end) throw new BadValue(Status.ERR_BAD_VALUE, `bytes after the value, from ${at}`);
        return value as T;
      }
      place(open, key, value);
      if (--open.left > 0) break;
      stack.pop();
      value = open.list ?? open.map ?? open.object;
      key = open.key;
    }
  }
}

/** How many bytes `n` takes as LEB128. */
function lebSize(n: number): number {
  let size = 1;
  while (n >= 0x80) {
    n = Math.floor(n / 0x80);
    size++;
  }
  return size;
}

/** A growing byte buffer. */
class Out {
  buf = new Uint8Array(256);
  len = 0;

  room(n: number): void {
    if (this.len + n <= this.buf.length) return;
    let size = this.buf.length * 2;
    while (size < this.len + n) size *= 2;
    const grown = new Uint8Array(size);
    grown.set(this.buf.subarray(0, this.len));
    this.buf = grown;
  }

  byte(b: number): void {
    this.room(1);
    this.buf[this.len++] = b;
  }

  leb(n: number): void {
    this.room(10);
    while (n >= 0x80) {
      this.buf[this.len++] = (n % 0x80) | 0x80;
      n = Math.floor(n / 0x80);
    }
    this.buf[this.len++] = n;
  }

  /** `text` as its LEB128 length and UTF-8 bytes; a lone surrogate is refused. */
  text(text: string): void {
    const n = text.length;
    const mark = this.len;
    if (n < 64) {
      // Short ASCII, the common case, is copied here: a native encoder
      // costs more to call than these few bytes.
      this.room(n + 1);
      this.buf[this.len++] = n;
      let i = 0;
      for (; i < n; i++) {
        const c = text.charCodeAt(i);
        if (c >= 0x80) break;
        this.buf[this.len++] = c;
      }
      if (i === n) return;
      this.len = mark;
    }
    if (!text.isWellFormed()) throw new TypeError('the text holds a lone surrogate, which UTF-8 cannot carry');
    // UTF-8 takes at least one byte and at most three per UTF-16 unit:
    // encode after room for the shortest length's prefix, and move the
    // bytes on in the rare case the real length needs a longer one.
    const guess = lebSize(n);
    this.room(10 + 3 * n);
    const { written } = ENCODER.encodeInto(text, this.buf.subarray(mark + guess, mark + guess + 3 * n));
    const need = lebSize(written);
    if (need !== guess) this.buf.copyWithin(mark + need, mark + guess, mark + guess + written);
    this.len = mark;
    this.leb(written);
    this.len += written;
  }

  bytes(data: Uint8Array): void {
    this.leb(data.length);
    this.room(data.length);
    this.buf.set(data, this.len);
    this.len += data.length;
  }
}

function write(out: Out, value: unknown, depth: number, seen: Set<object>): void {
  switch (typeof value) {
    case 'string':
      out.byte(STRING);
      out.text(value);
      return;
    case 'number':
    case 'bigint':
      out.byte(NUMBER);
      out.text(numberTextFor(value));
      return;
    case 'boolean':
      out.byte(BOOL);
      out.byte(value ? 1 : 0);
      return;
    case 'undefined':
      out.byte(ABSENT);
      return;
    case 'object':
      break;
    default:
      throw new TypeError(`cannot convert a ${typeof value} to a guatiao value`);
  }
  if (value === null) {
    out.byte(NULL);
    return;
  }
  if (value instanceof Ref) {
    const wire = value.toWire();
    out.room(wire.length);
    out.buf.set(wire, out.len);
    out.len += wire.length;
    return;
  }
  const bytes = asBytes(value);
  if (bytes !== undefined) {
    out.byte(BYTES);
    out.bytes(bytes);
    return;
  }
  if (seen.has(value)) throw new TypeError('the value refers to itself');
  if (depth >= MAX_DEPTH) throw tooDeep();
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      out.byte(LIST);
      out.leb(value.length);
      for (const item of value as unknown[]) write(out, item, depth + 1, seen);
      return;
    }
    if (value instanceof Map) {
      out.byte(MAP);
      out.leb(value.size);
      for (const [k, item] of value as Map<unknown, unknown>) {
        if (typeof k !== 'string') throw new TypeError('a map key is a string');
        out.text(k);
        write(out, item, depth + 1, seen);
      }
      return;
    }
    if (!isPlainObject(value)) {
      throw new TypeError(`cannot convert a ${value.constructor?.name ?? 'object'} to a guatiao value`);
    }
    const keys = Object.keys(value);
    out.byte(MAP);
    out.leb(keys.length);
    for (const k of keys) {
      out.text(k);
      write(out, value[k], depth + 1, seen);
    }
  } finally {
    seen.delete(value);
  }
}

/**
 * The wire bytes of what `new Value(value)` builds, written here with no
 * tree built: the same data accepted (a `Ref` is written as its node), the
 * same errors thrown, and `BadValue` past {@link MAX_DEPTH}.
 */
export function jsToWire(value: unknown): Uint8Array {
  const out = new Out();
  write(out, value, 0, new Set());
  return out.buf.subarray(0, out.len);
}
