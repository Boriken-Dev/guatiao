// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Finding and loading `guatiao`, `guatiao_serde` and `guatiao_intake`, the
 * declaration of every export this binding calls, and reading and writing
 * native memory.
 *
 * Nothing native is touched until the first call that needs it: koffi
 * itself is required lazily, so importing the package loads no addon and
 * no library.
 *
 * @module
 */

import { existsSync, statSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import type KoffiModule from 'koffi';

import { OWNED, PROVIDER_ERROR, SIZE, Status, Tag, VALUE } from './abi.js';
import { LibraryNotFound, MissingSymbol, raiseForStatus } from './errors.js';

/** The koffi module, as its CommonJS entry exports it. */
export type Koffi = typeof KoffiModule;

let koffiModule: Koffi | undefined;

/** koffi, loaded on first use, with the by-value structs declared. */
export function koffi(): Koffi {
  if (koffiModule === undefined) {
    const loaded = createRequire(import.meta.url)('koffi') as Koffi;
    loaded.struct('guatiao_str', { ptr: 'void *', len: 'size_t' });
    loaded.struct('guatiao_bytes', { ptr: 'void *', len: 'size_t' });
    loaded.struct('guatiao_bytes_mut', { ptr: 'void *', len: 'size_t' });
    loaded.struct('guatiao_object', { table: 'void *', size: 'size_t', ctx: 'void *' });
    koffiModule = loaded;
  }
  return koffiModule;
}

// ---- memory -------------------------------------------------------------

/** A byte view of `len` bytes at `ptr`. */
export function view(ptr: bigint, len: number): DataView {
  return new DataView(koffi().view(ptr, len));
}

export function u8(ptr: bigint, at = 0): number {
  return view(ptr + BigInt(at), 1).getUint8(0);
}
export function u32(ptr: bigint, at = 0): number {
  return view(ptr + BigInt(at), 4).getUint32(0, true);
}
export function i32(ptr: bigint, at = 0): number {
  return view(ptr + BigInt(at), 4).getInt32(0, true);
}
export function u64(ptr: bigint, at = 0): bigint {
  return view(ptr + BigInt(at), 8).getBigUint64(0, true);
}
export function i64(ptr: bigint, at = 0): bigint {
  return view(ptr + BigInt(at), 8).getBigInt64(0, true);
}
/** A pointer field; `0n` for null. */
export function ptrAt(ptr: bigint, at = 0): bigint {
  return u64(ptr, at);
}
/** A `size_t` field, which this binding reads as a number. */
export function size(ptr: bigint, at = 0): number {
  return Number(u64(ptr, at));
}
export function setU8(ptr: bigint, at: number, value: number): void {
  view(ptr + BigInt(at), 1).setUint8(0, value);
}
export function setU32(ptr: bigint, at: number, value: number): void {
  view(ptr + BigInt(at), 4).setUint32(0, value, true);
}
export function setI32(ptr: bigint, at: number, value: number): void {
  view(ptr + BigInt(at), 4).setInt32(0, value, true);
}
export function setU64(ptr: bigint, at: number, value: bigint): void {
  view(ptr + BigInt(at), 8).setBigUint64(0, value, true);
}
export function setI64(ptr: bigint, at: number, value: bigint): void {
  view(ptr + BigInt(at), 8).setBigInt64(0, value, true);
}
export function setPtr(ptr: bigint, at: number, value: bigint | null): void {
  setU64(ptr, at, value ?? 0n);
}

/** `len` bytes at `ptr`, copied out. */
export function bytesAt(ptr: bigint, len: number): Uint8Array {
  if (len === 0 || ptr === 0n) return new Uint8Array(0);
  return new Uint8Array(koffi().view(ptr, len)).slice();
}

/** Copies `data` to `ptr`. */
export function write(ptr: bigint, data: Uint8Array): void {
  if (data.length === 0) return;
  new Uint8Array(koffi().view(ptr, data.length)).set(data);
}

/** Copies `len` bytes from `src` to `dst`; the two must not overlap. */
export function copy(dst: bigint, src: bigint, len: number): void {
  if (len === 0) return;
  const k = koffi();
  new Uint8Array(k.view(dst, len)).set(new Uint8Array(k.view(src, len)));
}

export function zero(ptr: bigint, len: number): void {
  new Uint8Array(koffi().view(ptr, len)).fill(0);
}

/** `len` zeroed bytes that stay where they are until `release`. */
export function alloc(len: number): bigint {
  const ptr = koffi().alloc('uint8_t', len) as bigint;
  zero(ptr, len);
  return ptr;
}

export function release(ptr: bigint): void {
  koffi().free(ptr);
}

const UTF8 = new TextDecoder('utf-8', { fatal: true });

/** UTF-8 text at `ptr`; `TypeError` for bytes that are not UTF-8. */
export function textAt(ptr: bigint, len: number): string {
  if (len === 0 || ptr === 0n) return '';
  return UTF8.decode(new Uint8Array(koffi().view(ptr, len)));
}

/** `text` as UTF-8, refusing a lone surrogate rather than replacing it. */
export function encode(text: string): Buffer {
  if (typeof text !== 'string') throw new TypeError(`expected a string, got ${typeof text}`);
  if (!text.isWellFormed()) throw new TypeError('the text holds a lone surrogate, which UTF-8 cannot carry');
  return Buffer.from(text, 'utf8');
}

/** A `guatiao_str` argument, by value. */
export interface StrArg {
  ptr: Buffer | bigint | null;
  len: number;
}

/** `text` as a `guatiao_str` argument; koffi pins the bytes for the call. */
export function str(text: string): StrArg {
  const bytes = encode(text);
  return { ptr: bytes.length ? bytes : null, len: bytes.length };
}

/** `data` as a `guatiao_bytes` argument. */
export function bytesArg(data: Uint8Array): StrArg {
  const copied = Buffer.from(data.buffer, data.byteOffset, data.byteLength);
  return { ptr: copied.length ? copied : null, len: copied.length };
}

// ---- finding the libraries -----------------------------------------------

/** The platform's file name for a library of this base name. */
export function libraryFilename(basename: string): string {
  if (process.platform === 'win32') return `${basename}.dll`;
  if (process.platform === 'darwin') return `lib${basename}.dylib`;
  return `lib${basename}.so`;
}

let chosen: string | undefined;

/**
 * Names the guatiao library to load instead of looking for one: a file
 * exporting the ABI, whatever it is called, or a directory holding
 * `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib`. `guatiao_serde` and
 * `guatiao_intake` are then looked for beside it. Call it before the first
 * native call; naming another file once one is loaded throws, naming the
 * same one does nothing.
 */
export function useLibrary(where: string | URL): void {
  const named = where instanceof URL ? fileURLToPath(where) : where;
  if (cores !== undefined) {
    const same = isFile(named) ? named : path.join(named, libraryFilename('guatiao'));
    if (path.resolve(same) !== path.resolve(cores.path)) {
      throw new Error(`the guatiao library is already loaded from ${cores.path}`);
    }
    return;
  }
  chosen = named;
}

function isFile(p: string): boolean {
  try {
    return statSync(p).isFile();
  } catch {
    return false;
  }
}

function isDir(p: string): boolean {
  try {
    return statSync(p).isDirectory();
  } catch {
    return false;
  }
}

/** Where `basename`'s library is, by each route in order. */
function candidates(basename: string): { tries: string[]; looked: string[] } {
  const filename = libraryFilename(basename);
  const tries: string[] = [];
  const looked: string[] = [];
  const named = chosen ?? process.env.GUATIAO_LIBRARY;
  const route = chosen !== undefined ? 'useLibrary()' : 'GUATIAO_LIBRARY';
  if (named) {
    if (isDir(named)) {
      const candidate = path.join(named, filename);
      if (existsSync(candidate)) tries.push(candidate);
    } else if (isFile(named)) {
      // The core library is the file as it is named; the others are
      // looked for beside it, under their own platform names.
      const candidate = basename === 'guatiao' ? named : path.join(path.dirname(named), filename);
      if (existsSync(candidate)) tries.push(candidate);
    }
    looked.push(`${route} (${named})`);
  } else {
    looked.push('GUATIAO_LIBRARY (unset)');
  }
  tries.push(filename);
  looked.push(`the platform's search for ${filename}`);
  return { tries, looked };
}

// ---- the exports -----------------------------------------------------------

type Declared = readonly [prototype: string, feature?: string];

const TEXT_FORMS = ['date', 'time', 'datetime', 'timestamp', 'span'] as const;

const CORE: readonly Declared[] = [
  ['uint32_t guatiao_alloc_default(void *out)'],
  ['uint32_t guatiao_value_free(void *value)'],
  ['uint32_t guatiao_value_clone(void *alloc, void *src, void *out)'],
  ['uint32_t guatiao_value_null(void *out)'],
  ['uint32_t guatiao_value_absent(void *out)'],
  ['uint32_t guatiao_value_bool(bool b, void *out)'],
  ['uint32_t guatiao_value_map(void *alloc, void *out)'],
  ['uint32_t guatiao_value_list(void *alloc, void *out)'],
  ['uint32_t guatiao_value_string(void *alloc, guatiao_str text, void *out)'],
  ['uint32_t guatiao_value_number(void *alloc, guatiao_str text, void *out)'],
  ['uint32_t guatiao_value_bytes(void *alloc, guatiao_bytes data, void *out)'],
  ['uint32_t guatiao_map_set(void *alloc, void *node, guatiao_str key, void *value)'],
  ['uint32_t guatiao_map_discard(void *node, guatiao_str key)'],
  ['uint32_t guatiao_map_clear(void *node)'],
  ['uint32_t guatiao_map_copy_from(void *alloc, void *dst, void *src)'],
  ['uint32_t guatiao_list_push(void *alloc, void *node, void *value)'],
  ['uint32_t guatiao_list_discard(void *node, size_t index)'],
  ['uint32_t guatiao_list_clear(void *node)'],
  ['uint32_t guatiao_schema_redact(void *schema, void *value, guatiao_str placeholder, void *alloc, void *out)'],
  ['uint32_t guatiao_date_add_span(void *date, void *span, void *out)'],
  ['uint32_t guatiao_datetime_add_span(void *at, void *span, void *out)'],
  ['uint32_t guatiao_duration_read(void *value, void *seconds, void *nanoseconds)'],
  ...TEXT_FORMS.flatMap((form): Declared[] => [
    [`uint32_t guatiao_${form}_parse(void *alloc, guatiao_str text, void *out, void *err)`],
    [`uint32_t guatiao_${form}_format(void *alloc, void *fields, void *out)`],
    [`uint32_t guatiao_${form}_normalize(void *alloc, guatiao_str text, void *out, void *err)`],
  ]),
  ['void *guatiao_registry_new(guatiao_str id, guatiao_str version, void *alloc)', 'load'],
  ['void guatiao_registry_free(void *reg)', 'load'],
  ['void *guatiao_registry_host(void *reg)', 'load'],
  ['uint32_t guatiao_registry_keyed_by(void *reg, guatiao_str template)', 'load'],
  ['uint32_t guatiao_registry_libraries_keyed_by(void *reg, guatiao_str template)', 'load'],
  ['uint32_t guatiao_registry_load_file(void *reg, guatiao_str path, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_register_entry(void *reg, guatiao_str name, void *entry, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_scan_dir(void *reg, guatiao_str dir, bool descending, void *alloc, void *out)', 'load'],
  [
    'uint32_t guatiao_registry_scan_dir_rules(void *reg, guatiao_str dir, bool descending, guatiao_str rules, void *alloc, void *out)',
    'load',
  ],
  [
    'uint32_t guatiao_registry_scan_path(void *reg, guatiao_str spec, bool descending, guatiao_str rules, void *alloc, void *out)',
    'load',
  ],
  ['uint32_t guatiao_registry_libraries(void *reg, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_providers(void *reg, guatiao_str kind, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_provider(void *reg, guatiao_str key, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_available(void *reg, guatiao_str kind, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_why_not(void *reg, guatiao_str kind, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_best(void *reg, guatiao_str kind, void *alloc, void *out)', 'load'],
  ['bool guatiao_registry_provider_available(void *reg, guatiao_str key, void *reason)', 'load'],
  ['uint32_t guatiao_registry_set_priority(void *reg, guatiao_str id, int32_t priority)', 'load'],
  ['int32_t guatiao_registry_priority(void *reg, guatiao_str id)', 'load'],
  ['void *guatiao_registry_provider_table(void *reg, guatiao_str key, guatiao_str kind, void *size)', 'load'],
  ['void *guatiao_registry_provider_vtable(void *reg, guatiao_str key, void *size)', 'load'],
  ['void *guatiao_registry_provider_ctx(void *reg, guatiao_str key)', 'load'],
  ['void *guatiao_registry_provider_config(void *reg, guatiao_str key)', 'load'],
  ['uint32_t guatiao_registry_provider_create(void *reg, guatiao_str key, void *config, void *out, void *err)', 'load'],
  ['void guatiao_registry_provider_destroy(void *reg, guatiao_str key, void *instance)', 'load'],
  ['uint32_t guatiao_registry_retire(void *reg, guatiao_str key)', 'load'],
  ['uint32_t guatiao_registry_unload(void *reg, guatiao_str key)', 'load'],
  ['uint32_t guatiao_registry_unload_unchecked(void *reg, guatiao_str key)', 'load'],
  ['uint32_t guatiao_registry_collect(void *reg, void *alloc, void *out)', 'load'],
  ['uint32_t guatiao_registry_add_source_file(void *reg, guatiao_str path, void *config, void *err)', 'load'],
  ['uint32_t guatiao_registry_watch_dir(void *reg, guatiao_str dir, guatiao_str rules)', 'load'],
  ['uint32_t guatiao_registry_refresh(void *reg, void *alloc, void *out)', 'load'],
];

const SERDE: readonly Declared[] = [
  ['uint32_t guatiao_alloc_default(void *out)'],
  ['uint32_t guatiao_json_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)'],
  ['uint32_t guatiao_json_emit(void *value, uint32_t how, void *alloc, void *out)'],
  ['uint32_t guatiao_json_emit_pretty(void *value, uint32_t how, void *alloc, void *out)'],
  ['uint32_t guatiao_toml_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)', 'toml'],
  ['uint32_t guatiao_toml_emit(void *value, uint32_t how, void *alloc, void *out)', 'toml'],
  ['uint32_t guatiao_yaml_parse_ex(guatiao_str text, uint32_t how, void *alloc, void *out, void *err)', 'yaml'],
  ['uint32_t guatiao_yaml_emit(void *value, uint32_t how, void *alloc, void *out)', 'yaml'],
];

const INTAKE: readonly Declared[] = [
  ['uint32_t guatiao_alloc_default(void *out)'],
  ['uint32_t guatiao_intake_check(void *schema, void *form, void *alloc, void *out)'],
  ['uint32_t guatiao_intake_layout(void *schema, void *form, void *alloc, void *out)'],
  ['uint32_t guatiao_intake_is_visible(void *schema, void *form, guatiao_str key, void *values, void *out)'],
  ['uint32_t guatiao_intake_for_schema(void *schema, void *alloc, void *out)'],
  ['uint32_t guatiao_intake_form_for(void *form, void *schema, guatiao_str path, void *alloc, void *out)'],
  ['void *guatiao_intake_path_get(void *value, guatiao_str path)'],
  ['void *guatiao_intake_resolve(void *schema, guatiao_str key)'],
  ['uint32_t guatiao_intake_flat_keys(void *schema, guatiao_str key, void *alloc, void *out)'],
  ['uint32_t guatiao_intake_flatten(void *schema, guatiao_str key, void *value, void *alloc, void *out)'],
  ['uint32_t guatiao_intake_unflatten(void *schema, guatiao_str key, void *store, void *alloc, void *out)'],
];

// koffi's functions take and answer what their prototypes say; the
// callers here know which.
type NativeFn = (...args: any[]) => any;

/** One loaded library: its declared exports and its default allocator. */
export class Surface {
  readonly path: string;
  readonly fn: Readonly<Record<string, NativeFn>>;
  readonly missing: ReadonlySet<string>;
  private allocPtr: bigint | undefined;

  constructor(file: string, exports: readonly Declared[]) {
    const k = koffi();
    const handle = k.load(file);
    const fns: Record<string, NativeFn> = {};
    const missing = new Set<string>();
    for (const [prototype, feature] of exports) {
      const name = /(\w+)\s*\(/.exec(prototype)?.[1] ?? prototype;
      try {
        fns[name] = handle.func(prototype) as NativeFn;
      } catch {
        missing.add(name);
        fns[name] = () => {
          throw new MissingSymbol(file, name, feature);
        };
      }
    }
    this.path = file;
    this.fn = fns;
    this.missing = missing;
  }

  /**
   * The address of this library's default allocator, filled by
   * `guatiao_alloc_default` once and kept for the process: every tree
   * built in it records it.
   */
  get alloc(): bigint {
    if (this.allocPtr === undefined) {
      const ptr = alloc(SIZE.alloc);
      raiseForStatus(this.fn.guatiao_alloc_default!(ptr) as number, 'guatiao_alloc_default failed');
      this.allocPtr = ptr;
    }
    return this.allocPtr;
  }
}

function load(basename: string, exports: readonly Declared[]): Surface {
  const { tries, looked } = candidates(basename);
  for (const file of tries) {
    try {
      return new Surface(file, exports);
    } catch (e) {
      if (e instanceof MissingSymbol) throw e;
    }
  }
  throw new LibraryNotFound(basename, looked);
}

let cores: Surface | undefined;
let serdes: Surface | undefined;
let intakes: Surface | undefined;

/** The `guatiao` library, loaded and declared on first use. */
export function core(): Surface {
  return (cores ??= load('guatiao', CORE));
}

/** The `guatiao_serde` library, loaded on first use. */
export function serde(): Surface {
  return (serdes ??= load('guatiao_serde', SERDE));
}

/** The `guatiao_intake` library, loaded on first use. */
export function intake(): Surface {
  return (intakes ??= load('guatiao_intake', INTAKE));
}

// ---- small things every module needs --------------------------------------

/** Frees whatever the value at `node` owns; harmless on an empty one. */
export function freeNode(node: bigint): void {
  core().fn.guatiao_value_free!(node);
}

/**
 * Frees an owned `guatiao_string` at `at` through what it records, by
 * wrapping it in a string value in scratch memory.
 */
export function freeString(at: bigint): void {
  if (size(at, OWNED.cap) === 0) return;
  const node = alloc(SIZE.value);
  try {
    setU32(node, VALUE.tag, Tag.STRING);
    copy(node + BigInt(VALUE.payload), at, SIZE.string);
    freeNode(node);
  } finally {
    release(node);
  }
}

/** The text of an owned `guatiao_string` at `at`, replacing bad UTF-8. */
export function stringText(at: bigint): string {
  const len = size(at, OWNED.len);
  if (len === 0) return '';
  return Buffer.from(bytesAt(ptrAt(at, OWNED.ptr), len)).toString('utf8');
}

/**
 * Throws unless `status` is `GUATIAO_OK`, reading and freeing the message
 * of the `guatiao_provider_error` at `err` first: it is owned text the
 * other side built.
 */
export function check(status: number, err?: bigint): void {
  let message = '';
  if (err !== undefined && err !== 0n) {
    const at = err + BigInt(PROVIDER_ERROR.message);
    message = stringText(at);
    freeString(at);
    zero(at, SIZE.string);
  }
  raiseForStatus(status, message);
}

/** Zeroed scratch for one call, freed by `done()`. */
export class Scratch {
  private readonly blocks: bigint[] = [];

  take(len: number): bigint {
    const ptr = alloc(len);
    this.blocks.push(ptr);
    return ptr;
  }

  done(): void {
    for (const ptr of this.blocks.splice(0)) release(ptr);
  }
}

export { Status };
