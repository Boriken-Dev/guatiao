// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The structs and constants of `guatiao.h`, as byte layouts: sizes and
 * offsets in the header's own field order, natural alignment, written in
 * words (a pointer and a `size_t`), as the header states them: a view is
 * two words, an owned container four, a value 8 bytes and four words. A
 * 64-bit integer is 8-aligned, as on every platform the library builds for
 * (64-bit, and 32-bit ARM). Every struct the binding reads or writes in
 * memory is laid out here and nowhere else; `test/abi.test.ts` checks the
 * constants against the committed header and against the layout this
 * platform's C compiler gives it.
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

/** The 32-bit platforms Node runs on: every other one has 8-byte pointers. */
const THIRTY_TWO_BIT: ReadonlySet<string> = new Set(['arm', 'ia32', 'mips', 'mipsel', 'ppc', 's390']);

/** The width of a pointer and of `size_t`, in bytes: 8 on a 64-bit platform, 4 on a 32-bit one. */
export const WORD: 4 | 8 = THIRTY_TWO_BIT.has(process.arch) ? 4 : 8;

const W = WORD;
/** `guatiao_value`: a 4-byte tag, 4 of padding, a four-word payload. */
const VALUE_SIZE = 8 + 4 * W;
/** `guatiao_wire_event`: a `uint64_t`, two `uint32_t`, a `size_t`, then three values. */
const WIRE_EVENT_VALUE = 16 + W;

/** Sizes, in bytes. */
export const SIZE = {
  str: 2 * W,
  bytes: 2 * W,
  string: 4 * W,
  buffer: 4 * W,
  list: 4 * W,
  map: 4 * W,
  payload: 4 * W,
  value: VALUE_SIZE,
  entry: 4 * W + VALUE_SIZE,
  alloc: 5 * W,
  providerError: 5 * W,
  textError: W + VALUE_SIZE,
  date: 8,
  time: 8,
  datetime: 16,
  timestamp: 16,
  span: 64,
  kindHeader: 8,
  object: 3 * W,
  kinds: 2 * W,
  kindTable: 8 + 3 * W,
  kindTables: 3 * W,
  providerInfo: 8 + 20 * W,
  providers: 3 * W,
  libraryInfo: 8 + 9 * W,
  hostInfo: 8 + 7 * W,
  wireEvent: Math.ceil((WIRE_EVENT_VALUE + 3 * VALUE_SIZE) / 8) * 8,
} as const;

/**
 * `guatiao_string`, `guatiao_buffer`, `guatiao_list` and `guatiao_map`
 * share one layout: `ptr`, `len`, `cap`, `alloc`.
 */
export const OWNED = { ptr: 0, len: W, cap: 2 * W, alloc: 3 * W } as const;

/**
 * A borrowed array or view (`guatiao_str`, `guatiao_bytes`, `guatiao_kinds`,
 * `guatiao_kind_tables`, `guatiao_providers`): `ptr`, `len`, and for an
 * array of structs its `stride`.
 */
export const VIEW = { ptr: 0, len: W, stride: 2 * W } as const;

/** `guatiao_value`: the tag, padding, then the payload. */
export const VALUE = { tag: 0, payload: 8 } as const;

/** `guatiao_entry`: an owned key, then its value. */
export const ENTRY = { key: 0, value: 4 * W } as const;

/** `guatiao_alloc`. */
export const ALLOC = { structSize: 0, ctx: W, alloc: 2 * W, free: 3 * W, release: 4 * W } as const;

/** `guatiao_provider_error`: a status, then an owned message. */
export const PROVIDER_ERROR = { status: 0, message: W } as const;

/** `guatiao_text_error`: the byte offset, then an owned string value. */
export const TEXT_ERROR = { at: 0, expected: W } as const;

/**
 * `guatiao_wire_event`: the channel, the kind and refusal codes, the byte
 * offset, then three owned values.
 */
export const WIRE_EVENT = {
  channel: 0,
  kind: 8,
  refusal: 12,
  at: 16,
  value: WIRE_EVENT_VALUE,
  message: WIRE_EVENT_VALUE + VALUE_SIZE,
  detail: WIRE_EVENT_VALUE + 2 * VALUE_SIZE,
} as const;

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
export const OBJECT = { table: 0, size: W, ctx: 2 * W } as const;

/** `guatiao_kind_table`. */
export const KIND_TABLE = { structSize: 0, vtableSize: 4, kind: 8, vtable: 8 + 2 * W } as const;

/** `guatiao_provider_info`. */
export const PROVIDER_INFO = {
  structSize: 0,
  vtableSize: 4,
  kinds: 8,
  id: 8 + 2 * W,
  displayName: 8 + 4 * W,
  config: 8 + 6 * W,
  vtable: 8 + 7 * W,
  ctx: 8 + 8 * W,
  meta: 8 + 9 * W,
  version: 8 + 10 * W,
  available: 8 + 12 * W,
  tables: 8 + 13 * W,
  create: 8 + 16 * W,
  destroy: 8 + 17 * W,
  kindDescriptions: 8 + 18 * W,
} as const;

/** `guatiao_library_info`. */
export const LIBRARY_INFO = {
  structSize: 0,
  abiVersion: 4,
  id: 8,
  version: 8 + 2 * W,
  providers: 8 + 4 * W,
  meta: 8 + 7 * W,
  unload: 8 + 8 * W,
} as const;

/** `guatiao_host_info`. */
export const HOST_INFO = {
  structSize: 0,
  abiVersion: 4,
  hostId: 8,
  hostVersion: 8 + 2 * W,
  alloc: 8 + 4 * W,
  meta: 8 + 5 * W,
  services: 8 + 6 * W,
} as const;
