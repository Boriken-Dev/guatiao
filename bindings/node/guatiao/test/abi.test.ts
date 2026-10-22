// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The constants are the ABI: every tag and status is read out of the
// committed header, so the binding cannot drift from it unnoticed.

import { describe, expect, it } from 'vitest';

import { SIZE, Status, Tag } from '../src/abi.js';
import * as n from '../src/native.js';
import { HEADER, readText } from './helpers.js';

function headerConstants(): Map<string, number> {
  const found = new Map<string, number>();
  for (const m of readText(HEADER).matchAll(/^\s*GUATIAO_([A-Z_0-9]+) = (\d+),/gm)) {
    found.set(m[1]!, Number(m[2]));
  }
  return found;
}

describe('the header', () => {
  const constants = headerConstants();

  it('names every tag as the binding does', () => {
    for (const [name, value] of Object.entries(Tag)) expect(constants.get(name), name).toBe(value);
  });

  it('names every status as the binding does', () => {
    for (const [name, value] of Object.entries(Status)) expect(constants.get(name), name).toBe(value);
  });

  it('declares no status or tag the binding is missing', () => {
    const tags = new Set(Object.keys(Tag));
    const statuses = new Set(Object.keys(Status));
    for (const name of constants.keys()) {
      if (name === 'OK' || name.startsWith('ERR_')) expect(statuses.has(name), name).toBe(true);
      if (['ABSENT', 'NULL', 'BOOL', 'NUMBER', 'STRING', 'BYTES', 'LIST', 'MAP'].includes(name)) {
        expect(tags.has(name), name).toBe(true);
      }
    }
  });

  it('passes the by-value structs at their C sizes', () => {
    const k = n.koffi();
    expect(k.sizeof('guatiao_str')).toBe(SIZE.str);
    expect(k.sizeof('guatiao_bytes')).toBe(SIZE.bytes);
    expect(k.sizeof('guatiao_object')).toBe(SIZE.object);
  });
});
