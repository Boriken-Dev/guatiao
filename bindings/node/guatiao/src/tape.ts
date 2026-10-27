// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The wire encoding read into plain JavaScript data and written from it,
 * with no library: one crossing carries a whole document as bytes, where
 * reading or building a tree node by node costs a crossing per node.
 *
 * This file imports nothing and names nothing of Node: the guatiao-remote
 * package, which a browser loads, compiles it in as well. Each package
 * throws a {@link TapeError} as its own `BadValue`.
 *
 * Reading refuses a body cut short, bytes after the value, an unknown tag,
 * text that is not UTF-8 and nesting past {@link MAX_DEPTH}, which is all
 * that bytes a library wrote need. Bytes from anywhere else are read with
 * `strict`, which also refuses, as the wire encoding states, a length not
 * in its shortest form or past what remains, a number outside the JSON
 * number grammar, a bool byte other than 0 or 1 and a key repeated.
 *
 * @module
 */

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

/** How `wireToJS` and `toJS` read numbers and maps. */
export interface ReadOptions {
  /**
   * `'auto'` (the default): a `number` for a fraction or an exponent, and
   * for an integer a `number` when it is safe and a `bigint` when it is
   * not. `'bigint'`: a `bigint` for every integer. `'text'`: the exact
   * text, always.
   */
  numbers?: 'auto' | 'bigint' | 'text';
  /** `'object'` (the default) reads a map as a plain object, `'map'` as a `Map`. */
  maps?: 'object' | 'map';
}

/** {@link ReadOptions}, and whether to read as strictly as bytes from a peer need. */
export interface TapeReadOptions extends ReadOptions {
  strict?: boolean;
}

/**
 * Writes `text` into `into` at `at` a byte per UTF-16 unit and answers
 * true when every unit is ASCII; answers false, having written nothing,
 * otherwise. Room for `text.length` bytes is there.
 */
export type CopyAscii = (text: string, into: Uint8Array, at: number) => boolean;

/** What a writer is told beyond plain data. */
export interface WriteOptions {
  /** The wire bytes of an object plain data does not cover, or `undefined`. */
  embed?: (value: object) => Uint8Array | undefined;
  /** A faster copy of long ASCII text than `TextEncoder.encodeInto`, where there is one. */
  copyAscii?: CopyAscii;
}

/**
 * Bytes that are not one value, or data nested past {@link MAX_DEPTH}. A
 * refusal of bytes names the byte it went wrong at (`... at N`).
 */
export class TapeError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'TapeError';
  }
}

/** Refuses what is not UTF-8, and keeps a leading U+FEFF: it is text. */
const UTF8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
const ENCODER = new TextEncoder();

/** The JSON number grammar, which a number's text follows. */
const GRAMMAR = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$/;

const refused = (why: string, at: number): TapeError => new TapeError(`${why} at ${at}`);
const tooDeep = (): TapeError => new TapeError(`nested deeper than ${MAX_DEPTH}`);

/** A number's text read as `ReadOptions.numbers` says. */
export function readNumber(text: string, numbers: ReadOptions['numbers']): number | bigint | string {
  if (numbers === 'text') return text;
  const fractional = /[.eE]/.test(text);
  if (fractional) return Number(text);
  if (numbers === 'bigint') return BigInt(text);
  if (numbers !== undefined && numbers !== 'auto') throw new TypeError(`unknown numbers mode: ${String(numbers)}`);
  const asNumber = Number(text);
  return Number.isSafeInteger(asNumber) ? asNumber : BigInt(text);
}

/** The text a JS number or bigint is written as. */
export function numberTextFor(value: number | bigint): string {
  if (typeof value === 'bigint') return value.toString();
  if (!Number.isFinite(value)) throw new RangeError(`${value} is not a finite number`);
  return Object.is(value, -0) ? '-0' : String(value);
}

/** Whether `value` is an object literal, not an instance of a class. */
export function isPlainObject(value: object): value is Record<string, unknown> {
  const proto = Object.getPrototypeOf(value) as unknown;
  return proto === Object.prototype || proto === null;
}

/** A byte view's bytes, without copying; `undefined` for anything else. */
export function asBytes(value: unknown): Uint8Array | undefined {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  return undefined;
}

/** Defined, not assigned: a key named `__proto__` is data here. */
export function define(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, { value, enumerable: true, writable: true, configurable: true });
}

/** `n` bytes from `at` as text; ASCII is read without a decoder. `lenAt` is where its length starts. */
function textOf(data: Uint8Array, at: number, n: number, lenAt: number): string {
  if (n < 32) {
    let s = '';
    for (let i = at; i < at + n; i++) {
      const c = data[i]!;
      if (c >= 0x80) return utf8(data, at, n, lenAt);
      s += String.fromCharCode(c);
    }
    return s;
  }
  return utf8(data, at, n, lenAt);
}

function utf8(data: Uint8Array, at: number, n: number, lenAt: number): string {
  try {
    return UTF8.decode(data.subarray(at, at + n));
  } catch {
    throw refused('text that is not UTF-8', lenAt);
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
  else if (key === '__proto__') define(top.object!, key, value);
  else top.object![key] = value;
}

/**
 * The plain data wire bytes hold: absent is `undefined`, numbers as
 * `options.numbers` says, bytes a `Uint8Array` copy, maps as
 * `options.maps` says (a key named `__proto__` is data). Throws
 * {@link TapeError} for bytes that are not one value (see the module's
 * note on what is checked, and what `strict` adds).
 */
export function wireToJS(data: Uint8Array, options: TapeReadOptions = {}): unknown {
  const numbers = options.numbers;
  if (numbers !== undefined && numbers !== 'auto' && numbers !== 'bigint' && numbers !== 'text') {
    throw new TypeError(`unknown numbers mode: ${String(numbers)}`);
  }
  const asMap = options.maps === 'map';
  const strict = options.strict === true;
  const end = data.length;
  const stack: Open[] = [];
  let at = 0;
  let key = '';
  // One LEB128 count; `at` moves past it. Strictly: the shortest form, at
  // most 64 bits, and at most what remains. One closure for both modes, the
  // one-byte count first: a closure per mode reads numbers about a tenth
  // slower.
  const count = (): number => {
    if (at >= end) throw refused('the bytes end inside a node', at);
    const b = data[at++]!;
    if (b < 0x80) {
      if (strict && b > end - at) throw refused('a length past what remains', at);
      return b;
    }
    return longCount(at - 1);
  };
  // A count of more than one byte, from `start`.
  const longCount = (start: number): number => {
    at = start;
    let n = 0;
    for (let shift = 0; ; shift += 7) {
      if (shift >= 64) throw refused('a length past 64 bits', start);
      if (!strict && shift > 56) throw refused('a length past what this reader holds', start);
      if (at >= end) throw refused('the bytes end inside a node', at);
      const b = data[at++]!;
      if (shift === 63 && (b & 0x7f) > 1) throw refused('a length past 64 bits', start);
      n += (b & 0x7f) * 2 ** shift;
      if (b < 0x80) {
        if (strict && b === 0) throw refused('a length not in its shortest form', start);
        if (strict && n > end - at) throw refused('a length past what remains', at);
        return n;
      }
    }
  };
  for (;;) {
    const top = stack.length ? stack[stack.length - 1]! : null;
    if (top !== null && top.list === null) {
      const keyAt = at;
      const n = count();
      if (at + n > end) throw refused('the bytes end inside a node', at);
      key = textOf(data, at, n, keyAt);
      at += n;
      if (strict && (top.map !== null ? top.map.has(key) : Object.hasOwn(top.object!, key))) {
        throw refused('a key repeated', keyAt);
      }
    }
    if (at >= end) throw refused('the bytes end inside a node', at);
    const tag = data[at++]!;
    let value: unknown;
    switch (tag) {
      case NUMBER:
      case STRING:
      case BYTES: {
        const lenAt = at;
        const n = count();
        if (at + n > end) throw refused('the bytes end inside a node', at);
        if (tag === STRING) value = textOf(data, at, n, lenAt);
        else if (tag === NUMBER) {
          const text = textOf(data, at, n, lenAt);
          if (strict && !GRAMMAR.test(text)) throw refused('a number outside the JSON number grammar', lenAt);
          value = readNumber(text, numbers);
        } else value = new Uint8Array(data.subarray(at, at + n));
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
      case BOOL: {
        if (at >= end) throw refused('the bytes end inside a node', at);
        const b = data[at++]!;
        if (strict && b > 1) throw refused('a bool byte other than 0 or 1', at - 1);
        value = b === 1;
        break;
      }
      case ABSENT:
        value = undefined;
        break;
      default:
        throw refused(`an unknown tag ${tag}`, at - 1);
    }
    // Place the value, closing every container it completes.
    for (;;) {
      const open = stack.length ? stack[stack.length - 1]! : null;
      if (open === null) {
        if (at !== end) throw refused('bytes after the value', at);
        return value;
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

/** Wire bytes being written, in a buffer that grows as needed. */
export class TapeWriter {
  private buf = new Uint8Array(256);
  private len = 0;
  private readonly embed: WriteOptions['embed'];
  private readonly copyAscii: CopyAscii | undefined;

  constructor(options: WriteOptions = {}) {
    this.embed = options.embed;
    this.copyAscii = options.copyAscii;
  }

  private room(n: number): void {
    if (this.len + n <= this.buf.length) return;
    // Doubled, or what this write needs when that is more: one large
    // string is not rounded up to a power of two.
    const grown = new Uint8Array(Math.max(this.buf.length * 2, this.len + n));
    grown.set(this.buf.subarray(0, this.len));
    this.buf = grown;
  }

  byte(b: number): void {
    this.room(1);
    this.buf[this.len++] = b;
  }

  /** An unsigned LEB128 of a count: a safe, non-negative integer. */
  leb(n: number): void {
    this.room(10);
    while (n >= 0x80) {
      this.buf[this.len++] = (n % 0x80) | 0x80;
      n = Math.floor(n / 0x80);
    }
    this.buf[this.len++] = n;
  }

  /** `data` as it is, with no length in front. */
  raw(data: Uint8Array): void {
    this.room(data.length);
    this.buf.set(data, this.len);
    this.len += data.length;
  }

  /** `data` behind its LEB128 length. */
  bytes(data: Uint8Array): void {
    this.leb(data.length);
    this.raw(data);
  }

  /** `text` as its LEB128 length and UTF-8 bytes; a lone surrogate is a `TypeError`. */
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
    } else if (this.copyAscii !== undefined && text.charCodeAt(0) < 0x80) {
      this.room(10 + n);
      this.leb(n);
      if (this.copyAscii(text, this.buf, this.len)) {
        this.len += n;
        return;
      }
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

  /**
   * `value` as plain data: `undefined` is absent, then `null`, booleans,
   * finite numbers and bigints (as their text), strings, byte views,
   * arrays, `Map`s with string keys and plain objects, else what `embed`
   * answers. `TypeError` for anything else (a cycle included), `RangeError`
   * for a number that is not finite, {@link TapeError} past
   * {@link MAX_DEPTH}.
   */
  value(value: unknown): void {
    this.write(value, 0, new Set());
  }

  /** What was written; a view of the buffer. */
  done(): Uint8Array {
    return this.buf.subarray(0, this.len);
  }

  private write(value: unknown, depth: number, seen: Set<object>): void {
    switch (typeof value) {
      case 'string':
        this.byte(STRING);
        this.text(value);
        return;
      case 'number':
      case 'bigint':
        this.byte(NUMBER);
        this.text(numberTextFor(value));
        return;
      case 'boolean':
        this.byte(BOOL);
        this.byte(value ? 1 : 0);
        return;
      case 'undefined':
        this.byte(ABSENT);
        return;
      case 'object':
        break;
      default:
        throw new TypeError(`cannot convert a ${typeof value} to a guatiao value`);
    }
    if (value === null) {
      this.byte(NULL);
      return;
    }
    const embedded = this.embed?.(value);
    if (embedded !== undefined) {
      this.raw(embedded);
      return;
    }
    const bytes = asBytes(value);
    if (bytes !== undefined) {
      this.byte(BYTES);
      this.bytes(bytes);
      return;
    }
    if (seen.has(value)) throw new TypeError('the value refers to itself');
    if (depth >= MAX_DEPTH) throw tooDeep();
    seen.add(value);
    try {
      if (Array.isArray(value)) {
        this.byte(LIST);
        this.leb(value.length);
        for (const item of value as unknown[]) this.write(item, depth + 1, seen);
        return;
      }
      if (value instanceof Map) {
        this.byte(MAP);
        this.leb(value.size);
        for (const [k, item] of value as Map<unknown, unknown>) {
          if (typeof k !== 'string') throw new TypeError('a map key is a string');
          this.text(k);
          this.write(item, depth + 1, seen);
        }
        return;
      }
      if (!isPlainObject(value)) {
        throw new TypeError(`cannot convert a ${value.constructor?.name ?? 'object'} to a guatiao value`);
      }
      const keys = Object.keys(value);
      this.byte(MAP);
      this.leb(keys.length);
      for (const k of keys) {
        this.text(k);
        this.write(value[k], depth + 1, seen);
      }
    } finally {
      seen.delete(value);
    }
  }
}

/** The wire bytes of `value` (see {@link TapeWriter.value}). */
export function jsToWire(value: unknown, options: WriteOptions = {}): Uint8Array {
  const out = new TapeWriter(options);
  out.value(value);
  return out.done();
}
