// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The structs and constants of `guatiao.h`, as byte layouts: sizes and
 * offsets in the header's own field order, natural alignment, 64-bit
 * pointers. Every struct the binding reads or writes in memory is laid out
 * here and nowhere else; `test/abi.test.ts` checks the constants against
 * the committed header.
 *
 * @module
 */

/** `guatiao_tag`: which arm of a value's payload is live. */
export const Tag = {
  ABSENT: 0,
  NULL: 1,
  BOOL: 2,
  NUMBER: 3,
  STRING: 4,
  BYTES: 5,
  LIST: 6,
  MAP: 7,
} as const;
export type Tag = (typeof Tag)[keyof typeof Tag];

/** `guatiao_status`: what an entry point reports. 0 is success. */
export const Status = {
  OK: 0,
  ERR_BAD_VALUE: 7,
  ERR_ALLOC: 8,
  ERR_WRONG_KIND: 9,
  ERR_NOT_FOUND: 10,
  ERR_NULL: 11,
  ERR_GONE: 12,
  ERR_UNSUPPORTED: 13,
  ERR_BUSY: 14,
  ERR_INTERNAL: 100,
} as const;
export type Status = (typeof Status)[keyof typeof Status];

/** `GUATIAO_ABI_VERSION`: the envelope version this binding speaks. */
export const ABI_VERSION = 1;

/** The width of a pointer and of `size_t`: this binding is 64-bit only. */
export const WORD = 8;

/** Sizes, in bytes. */
export const SIZE = {
  str: 16,
  bytes: 16,
  string: 32,
  buffer: 32,
  list: 32,
  map: 32,
  payload: 32,
  value: 40,
  entry: 72,
  alloc: 40,
  providerError: 40,
  textError: 48,
  date: 8,
  time: 8,
  datetime: 16,
  timestamp: 16,
  span: 64,
  kindHeader: 8,
  object: 24,
  kinds: 16,
  kindTable: 32,
  kindTables: 24,
  providerInfo: 152,
  providers: 24,
  libraryInfo: 80,
  hostInfo: 64,
} as const;

/**
 * `guatiao_string`, `guatiao_buffer`, `guatiao_list` and `guatiao_map`
 * share one layout: `ptr`, `len`, `cap`, `alloc`.
 */
export const OWNED = { ptr: 0, len: 8, cap: 16, alloc: 24 } as const;

/** `guatiao_value`: the tag, padding, then the 32-byte payload. */
export const VALUE = { tag: 0, payload: 8 } as const;

/** `guatiao_entry`: an owned key, then its value. */
export const ENTRY = { key: 0, value: 32 } as const;

/** `guatiao_alloc`. */
export const ALLOC = { structSize: 0, ctx: 8, alloc: 16, free: 24, release: 32 } as const;

/** `guatiao_provider_error`: a status, then an owned message. */
export const PROVIDER_ERROR = { status: 0, message: 8 } as const;

/** `guatiao_text_error`: the byte offset, then an owned string value. */
export const TEXT_ERROR = { at: 0, expected: 8 } as const;

/** `guatiao_date`. */
export const DATE = { year: 0, month: 4, day: 5 } as const;
/** `guatiao_time`. */
export const TIME = { hour: 0, minute: 1, second: 2, nanosecond: 4 } as const;
/** `guatiao_datetime`: a date, then a time. */
export const DATETIME = { date: 0, time: 8 } as const;
/** `guatiao_timestamp`. */
export const TIMESTAMP = { seconds: 0, nanoseconds: 8 } as const;
/** `guatiao_span`: seven `int64_t` units, then `int32_t` nanoseconds. */
export const SPAN = {
  years: 0,
  months: 8,
  weeks: 16,
  days: 24,
  hours: 32,
  minutes: 40,
  seconds: 48,
  nanoseconds: 56,
} as const;

/** `guatiao_kind_header`: the first eight bytes of every kind table. */
export const KIND_HEADER = { structSize: 0, floorHash: 4 } as const;

/** `guatiao_object`: an object crossing the boundary. */
export const OBJECT = { table: 0, size: 8, ctx: 16 } as const;

/** `guatiao_kind_table`. */
export const KIND_TABLE = { structSize: 0, vtableSize: 4, kind: 8, vtable: 24 } as const;

/** `guatiao_provider_info`. */
export const PROVIDER_INFO = {
  structSize: 0,
  vtableSize: 4,
  kinds: 8,
  id: 24,
  displayName: 40,
  config: 56,
  vtable: 64,
  ctx: 72,
  meta: 80,
  version: 88,
  available: 104,
  tables: 112,
  create: 136,
  destroy: 144,
} as const;

/** `guatiao_library_info`. */
export const LIBRARY_INFO = {
  structSize: 0,
  abiVersion: 4,
  id: 8,
  version: 24,
  providers: 40,
  meta: 64,
  unload: 72,
} as const;

/** `guatiao_host_info`. */
export const HOST_INFO = {
  structSize: 0,
  abiVersion: 4,
  hostId: 8,
  hostVersion: 24,
  alloc: 40,
  meta: 48,
  services: 56,
} as const;
