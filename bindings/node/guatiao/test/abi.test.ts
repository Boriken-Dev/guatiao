// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The constants are the ABI: every tag and status is read out of the
// committed header, and every struct's size and offsets are compared with
// the layout this platform's C compiler gives it, so the binding cannot
// drift from it unnoticed.

import { describe, expect, it } from 'vitest';

import * as abi from '../src/abi.js';
import { SIZE, Status, Tag, WORD } from '../src/abi.js';
import * as n from '../src/native.js';
import { HEADER, readText } from './helpers.js';
import { cLibraries } from './native.js';

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

  it('lays every struct out in words', () => {
    expect(WORD).toBe(n.koffi().sizeof('void *'));
    expect(SIZE.value).toBe(8 + 4 * WORD);
    expect(SIZE.entry).toBe(8 + 8 * WORD);
    expect(SIZE.str).toBe(2 * WORD);
  });

  it('lays every struct out as the C compiler lays out the header', (context) => {
    const built = cLibraries();
    if (built === null) return context.skip('no C compiler to lay the header out with');
    const k = n.koffi();
    const lib = k.load(built.layout);
    const table = lib.func('void *guatiao_layout(_Out_ size_t *count)');
    const count = [0];
    const entries = table(count) as bigint;
    const layout = new Map<string, number>();
    // Each entry: a `const char *` name, then a `size_t`.
    for (let i = 0; i < Number(count[0]); i++) {
      const at = entries + BigInt(i * 2 * WORD);
      layout.set(k.decode(n.ptrAt(at), 'char', -1) as string, n.size(at, WORD));
    }
    const sizes: Record<string, number> = {
      guatiao_str: SIZE.str,
      guatiao_bytes: SIZE.bytes,
      guatiao_string: SIZE.string,
      guatiao_buffer: SIZE.buffer,
      guatiao_list: SIZE.list,
      guatiao_map: SIZE.map,
      guatiao_payload: SIZE.payload,
      guatiao_value: SIZE.value,
      guatiao_entry: SIZE.entry,
      guatiao_alloc: SIZE.alloc,
      guatiao_provider_error: SIZE.providerError,
      guatiao_text_error: SIZE.textError,
      guatiao_date: SIZE.date,
      guatiao_time: SIZE.time,
      guatiao_datetime: SIZE.datetime,
      guatiao_timestamp: SIZE.timestamp,
      guatiao_span: SIZE.span,
      guatiao_kind_header: SIZE.kindHeader,
      guatiao_object: SIZE.object,
      guatiao_kinds: SIZE.kinds,
      guatiao_kind_table: SIZE.kindTable,
      guatiao_kind_tables: SIZE.kindTables,
      guatiao_provider_info: SIZE.providerInfo,
      guatiao_providers: SIZE.providers,
      guatiao_library_info: SIZE.libraryInfo,
      guatiao_host_info: SIZE.hostInfo,
      guatiao_wire_event: SIZE.wireEvent,
    };
    for (const [name, size] of Object.entries(sizes)) expect(size, name).toBe(layout.get(name));
    const pair = { ptr: abi.VIEW.ptr, len: abi.VIEW.len };
    const offsets: Record<string, Readonly<Record<string, number>>> = {
      guatiao_string: abi.OWNED,
      guatiao_buffer: abi.OWNED,
      guatiao_list: abi.OWNED,
      guatiao_map: abi.OWNED,
      guatiao_str: pair,
      guatiao_bytes: pair,
      guatiao_kinds: pair,
      guatiao_kind_tables: abi.VIEW,
      guatiao_providers: abi.VIEW,
      guatiao_value: abi.VALUE,
      guatiao_entry: abi.ENTRY,
      guatiao_alloc: abi.ALLOC,
      guatiao_provider_error: abi.PROVIDER_ERROR,
      guatiao_text_error: abi.TEXT_ERROR,
      guatiao_wire_event: abi.WIRE_EVENT,
      guatiao_date: abi.DATE,
      guatiao_time: abi.TIME,
      guatiao_datetime: abi.DATETIME,
      guatiao_timestamp: abi.TIMESTAMP,
      guatiao_span: abi.SPAN,
      guatiao_kind_header: abi.KIND_HEADER,
      guatiao_object: abi.OBJECT,
      guatiao_kind_table: abi.KIND_TABLE,
      guatiao_provider_info: abi.PROVIDER_INFO,
      guatiao_library_info: abi.LIBRARY_INFO,
      guatiao_host_info: abi.HOST_INFO,
    };
    const camel = (field: string): string => field.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
    for (const [name, fields] of Object.entries(offsets)) {
      for (const [field, at] of Object.entries(fields)) {
        const found = [...layout.keys()].find((key) => key.startsWith(`${name}.`) && camel(key.slice(name.length + 1)) === field);
        expect(found, `${name}.${field}`).toBeDefined();
        expect(at, `${name}.${field}`).toBe(layout.get(found!));
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
