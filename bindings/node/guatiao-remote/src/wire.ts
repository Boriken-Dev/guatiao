// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * One value in the wire encoding (guatiao's encodings, "Values on the
 * wire"), read into plain data and written from it, as the `guatiao`
 * package maps a value: `undefined` is absent, then `null`, booleans,
 * numbers (a `number` when it is a safe integer or has a fraction, else a
 * `bigint`), strings, `Uint8Array`, arrays and plain objects.
 *
 * @module
 */

import { BadValue } from './errors.js';

/** How deep containers nest: what every walk in guatiao follows. */
const MAX_DEPTH = 128;

const Tag = {
  ABSENT: 0,
  NULL: 1,
  BOOL: 2,
  NUMBER: 3,
  STRING: 4,
  BYTES: 5,
  LIST: 6,
  MAP: 7,
} as const;

const utf8 = new TextEncoder();
/** Refuses what is not UTF-8, and keeps a leading U+FEFF: it is text. */
const strict = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/** The JSON number grammar, which a number's text follows. */
const NUMBER = /^-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?$/;

/**
 * A number's text as the wire carries it, back as plain data: a `number`
 * when it has a fraction or an exponent (an infinity past a double's
 * range), or is a safe integer; else a `bigint`. Throws `BadValue` for
 * text outside the JSON number grammar.
 */
export function readNumber(text: string): number | bigint {
  if (!NUMBER.test(text)) throw new BadValue('a number outside the JSON number grammar');
  if (/[.eE]/.test(text)) return Number(text);
  const asNumber = Number(text);
  return Number.isSafeInteger(asNumber) ? asNumber : BigInt(text);
}

/** A number's text: a finite number or a bigint. */
export function numberText(value: number | bigint): string {
  if (typeof value === 'bigint') return value.toString();
  if (!Number.isFinite(value)) throw new RangeError(`${value} is not a finite number`);
  return Object.is(value, -0) ? '-0' : String(value);
}

/** Whether `value` is an object literal, not an instance of a class. */
export function isPlainObject(value: object): value is Record<string, unknown> {
  const proto = Object.getPrototypeOf(value) as unknown;
  return proto === Object.prototype || proto === null;
}

/** Defined, not assigned: a key named `__proto__` is data here. */
export function define(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, { value, enumerable: true, writable: true, configurable: true });
}

function tooDeep(): BadValue {
  return new BadValue(`nested deeper than ${MAX_DEPTH}`);
}

/** Bytes that are not a value: why, and the byte it went wrong at. */
function refused(why: string, at: number): BadValue {
  return new BadValue(`${why} at ${at}`);
}

class Reader {
  at = 0;
  constructor(readonly data: Uint8Array) {}

  byte(): number {
    if (this.at >= this.data.length) throw refused('the bytes end inside a node', this.at);
    return this.data[this.at++]!;
  }

  /** A length or a count: LEB128 in its shortest form, at most what is left. */
  count(): number {
    const start = this.at;
    let n = 0;
    for (let shift = 0; ; shift += 7) {
      if (shift >= 64) throw refused('a length past 64 bits', start);
      const b = this.byte();
      if (shift === 63 && (b & 0x7f) > 1) throw refused('a length past 64 bits', start);
      n += (b & 0x7f) * 2 ** shift;
      if (b < 0x80) {
        if (b === 0 && shift > 0) throw refused('a length not in its shortest form', start);
        if (n > this.data.length - this.at) throw refused('a length past what remains', this.at);
        return n;
      }
    }
  }

  body(): Uint8Array {
    const n = this.count();
    const out = this.data.subarray(this.at, this.at + n);
    this.at += n;
    return out;
  }

  text(): string {
    const start = this.at;
    try {
      return strict.decode(this.body());
    } catch (e) {
      if (e instanceof TypeError) throw refused('text that is not UTF-8', start);
      throw e;
    }
  }

  value(depth: number): unknown {
    const at = this.at;
    const tag = this.byte();
    switch (tag) {
      case Tag.STRING:
        return this.text();
      case Tag.NUMBER: {
        const start = this.at;
        try {
          return readNumber(String.fromCharCode(...this.body()));
        } catch (e) {
          if (e instanceof BadValue) throw refused(e.message, start);
          throw e;
        }
      }
      case Tag.MAP: {
        const n = this.count();
        if (n > 0 && depth >= MAX_DEPTH) throw tooDeep();
        const map: Record<string, unknown> = {};
        const seen = new Set<string>();
        for (let i = 0; i < n; i++) {
          const keyAt = this.at;
          const key = this.text();
          if (seen.has(key)) throw refused('a key repeated', keyAt);
          seen.add(key);
          define(map, key, this.value(depth + 1));
        }
        return map;
      }
      case Tag.LIST: {
        const n = this.count();
        if (n > 0 && depth >= MAX_DEPTH) throw tooDeep();
        const list: unknown[] = [];
        for (let i = 0; i < n; i++) list.push(this.value(depth + 1));
        return list;
      }
      case Tag.NULL:
        return null;
      case Tag.BOOL: {
        const b = this.byte();
        if (b > 1) throw refused('a bool byte other than 0 or 1', this.at - 1);
        return b === 1;
      }
      case Tag.BYTES:
        return this.body().slice();
      case Tag.ABSENT:
        return undefined;
    }
    throw refused(`an unknown tag ${tag}`, at);
  }
}

/**
 * The plain data `bytes` hold: one whole value, nothing after it.
 * Throws `BadValue` for bytes that are not one.
 */
export function fromWire(bytes: Uint8Array): unknown {
  const reader = new Reader(bytes);
  const value = reader.value(0);
  if (reader.at !== bytes.length) throw refused('bytes after the value', reader.at);
  return value;
}

/** Bytes being written, growing as needed. */
export class Bytes {
  private buf = new Uint8Array(64);
  private len = 0;

  private room(n: number): void {
    if (this.len + n <= this.buf.length) return;
    let size = this.buf.length * 2;
    while (size < this.len + n) size *= 2;
    const next = new Uint8Array(size);
    next.set(this.buf.subarray(0, this.len));
    this.buf = next;
  }

  byte(b: number): void {
    this.room(1);
    this.buf[this.len++] = b;
  }

  raw(bytes: Uint8Array): void {
    this.room(bytes.length);
    this.buf.set(bytes, this.len);
    this.len += bytes.length;
  }

  /** An unsigned LEB128 of a count: a safe, non-negative integer. */
  count(n: number): void {
    this.room(8);
    while (n >= 0x80) {
      this.buf[this.len++] = (n % 128) | 0x80;
      n = Math.floor(n / 128);
    }
    this.buf[this.len++] = n;
  }

  done(): Uint8Array {
    return this.buf.slice(0, this.len);
  }
}

function asBytes(value: unknown): Uint8Array | undefined {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  return undefined;
}

function write(out: Bytes, value: unknown, depth: number): void {
  if (typeof value === 'string') {
    out.byte(Tag.STRING);
    const b = utf8.encode(value);
    out.count(b.length);
    out.raw(b);
  } else if (value === undefined) {
    out.byte(Tag.ABSENT);
  } else if (value === null) {
    out.byte(Tag.NULL);
  } else if (typeof value === 'boolean') {
    out.byte(Tag.BOOL);
    out.byte(value ? 1 : 0);
  } else if (typeof value === 'number' || typeof value === 'bigint') {
    out.byte(Tag.NUMBER);
    const text = numberText(value);
    out.count(text.length);
    for (let i = 0; i < text.length; i++) out.byte(text.charCodeAt(i));
  } else if (typeof value === 'object') {
    const bytes = asBytes(value);
    if (bytes !== undefined) {
      out.byte(Tag.BYTES);
      out.count(bytes.length);
      out.raw(bytes);
      return;
    }
    if (depth >= MAX_DEPTH) throw tooDeep();
    if (Array.isArray(value)) {
      out.byte(Tag.LIST);
      out.count(value.length);
      for (const item of value) write(out, item, depth + 1);
    } else if (value instanceof Map) {
      out.byte(Tag.MAP);
      out.count(value.size);
      for (const [key, item] of value as Map<unknown, unknown>) {
        if (typeof key !== 'string') throw new TypeError('a map key must be a string');
        const b = utf8.encode(key);
        out.count(b.length);
        out.raw(b);
        write(out, item, depth + 1);
      }
    } else if (isPlainObject(value)) {
      const keys = Object.keys(value);
      out.byte(Tag.MAP);
      out.count(keys.length);
      for (const key of keys) {
        const b = utf8.encode(key);
        out.count(b.length);
        out.raw(b);
        write(out, value[key], depth + 1);
      }
    } else {
      throw new TypeError(`a ${value.constructor?.name ?? 'object'} cannot be converted to a guatiao value`);
    }
  } else {
    throw new TypeError(`a ${typeof value} cannot be converted to a guatiao value`);
  }
}

/** The wire bytes of plain data. Throws `TypeError` for what is not plain data. */
export function toWire(value: unknown): Uint8Array {
  const out = new Bytes();
  write(out, value, 0);
  return out.done();
}
