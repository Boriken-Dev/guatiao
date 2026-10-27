// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A whole document as wire bytes, read and written in JavaScript through
 * the tape (`tape.ts`), as this package gives it: a `Ref` written as its
 * node, and a refusal thrown as `BadValue`.
 *
 * @module
 */

import { Status } from './abi.js';
import { BadValue } from './errors.js';
import { type CopyAscii, type ReadOptions, TapeError, type WriteOptions, jsToWire as write, wireToJS as read } from './tape.js';
import { Ref } from './value.js';

/**
 * Node 22's `encodeInto` encodes a unit at a time: on long ASCII it is
 * about three times slower than measuring the text's UTF-8 length and
 * copying it as Latin-1. From Node 24 it is the faster of the two.
 */
const latin1: CopyAscii | undefined =
  Number(process.versions.node.split('.')[0]) < 24
    ? (text, into, at) => {
        // Every unit is ASCII exactly when the UTF-8 length is the unit
        // count (a lone surrogate counts three bytes).
        if (Buffer.byteLength(text, 'utf8') !== text.length) return false;
        Buffer.from(into.buffer, into.byteOffset).write(text, at, text.length, 'latin1');
        return true;
      }
    : undefined;

const WRITING: WriteOptions = {
  embed: (value) => (value instanceof Ref ? value.toWire() : undefined),
  copyAscii: latin1,
};

function badValue(e: unknown): unknown {
  return e instanceof TapeError ? new BadValue(Status.ERR_BAD_VALUE, e.message) : e;
}

/**
 * The plain data wire bytes hold, as `Value.fromWire(data).toJS(options)`
 * reads it, with no tree built. `BadValue` for a body cut short, bytes
 * after the value, an unknown tag, text that is not UTF-8 and nesting past
 * `MAX_DEPTH`; nothing else is checked: it is for bytes a library wrote.
 */
export function wireToJS<T = unknown>(data: Uint8Array, options: ReadOptions = {}): T {
  try {
    return read(data, { numbers: options.numbers, maps: options.maps }) as T;
  } catch (e) {
    throw badValue(e);
  }
}

/**
 * The wire bytes of what `new Value(value)` builds, written here with no
 * tree built: the same data accepted (a `Ref` is written as its node), the
 * same errors thrown, and `BadValue` past `MAX_DEPTH`.
 */
export function jsToWire(value: unknown): Uint8Array {
  try {
    return write(value, WRITING);
  } catch (e) {
    throw badValue(e);
  }
}
