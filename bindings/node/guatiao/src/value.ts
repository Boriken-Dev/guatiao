// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A guatiao value tree: `Value` owns the root; `Ref`, `MapRef` and
 * `ListRef` address a node inside it by path and walk to it fresh on every
 * access, because a write next to a node can move the array it sits in.
 *
 * Reading is a reimplementation of the header's `static inline` helpers
 * over the bytes, so it calls nothing in the guatiao library; building and
 * freeing do.
 *
 * @module
 */

import { ENTRY, OWNED, SIZE, Status, Tag, VALUE } from './abi.js';
import { raiseForStatus } from './errors.js';
import * as n from './native.js';
import { type ReadOptions, asBytes, define, isPlainObject, numberTextFor, readNumber } from './tape.js';

export type { ReadOptions } from './tape.js';

/** A step of a path: a key into a map, an index into a list. */
export type Step = string | number;

/**
 * What a `Ref` walks from: a root node, and whether and how it may be
 * read and changed. A `Value` is one; so is every view a call, a registry
 * or an intake lookup lends.
 * @internal
 */
export interface Owner {
  /** The address of the root node, found again on each call. */
  root(): bigint;
  checkOpen(): void;
  checkWritable(): void;
  /** The allocator to build in, for a write. */
  buildAlloc(): bigint;
  /** The allocator a clone is made in. */
  cloneAlloc(): bigint;
}

// ---- reading, with nothing from the library ------------------------------------

const payload = (node: bigint): bigint => node + BigInt(VALUE.payload);

function tagOf(node: bigint | null): number {
  return node === null ? Tag.ABSENT : n.u32(node, VALUE.tag);
}

function ownedLen(node: bigint): number {
  return n.size(payload(node), OWNED.len);
}

function ownedBytes(node: bigint): Uint8Array {
  const at = payload(node);
  return n.bytesAt(n.ptrAt(at, OWNED.ptr), n.size(at, OWNED.len));
}

function ownedText(node: bigint): string {
  const at = payload(node);
  return n.textAt(n.ptrAt(at, OWNED.ptr), n.size(at, OWNED.len));
}

function listAt(node: bigint | null, index: number): bigint | null {
  if (node === null || tagOf(node) !== Tag.LIST) return null;
  const len = ownedLen(node);
  const i = index < 0 ? index + len : index;
  if (!Number.isInteger(i) || i < 0 || i >= len) return null;
  return n.ptrAt(payload(node), OWNED.ptr) + BigInt(i * SIZE.value);
}

function sameBytes(ptr: bigint, len: number, key: Uint8Array): boolean {
  if (len !== key.length) return false;
  if (len === 0) return true;
  const there = new Uint8Array(n.koffi().view(ptr, len));
  for (let i = 0; i < len; i++) if (there[i] !== key[i]) return false;
  return true;
}

/** The address of the value under `key`, or null. Keys compare as bytes. */
function mapFind(node: bigint | null, key: Uint8Array): bigint | null {
  if (node === null || tagOf(node) !== Tag.MAP) return null;
  const len = ownedLen(node);
  if (len === 0) return null;
  const entries = n.ptrAt(payload(node), OWNED.ptr);
  const table = n.view(entries, len * SIZE.entry);
  for (let i = 0; i < len; i++) {
    const at = i * SIZE.entry + ENTRY.key;
    const keyPtr = n.wordIn(table, at + OWNED.ptr);
    const keyLen = Number(n.wordIn(table, at + OWNED.len));
    if (sameBytes(keyPtr, keyLen, key)) return entries + BigInt(i * SIZE.entry + ENTRY.value);
  }
  return null;
}

function mapKeys(node: bigint): string[] {
  const len = ownedLen(node);
  const entries = n.ptrAt(payload(node), OWNED.ptr);
  const keys: string[] = [];
  for (let i = 0; i < len; i++) {
    const at = entries + BigInt(i * SIZE.entry + ENTRY.key);
    keys.push(n.textAt(n.ptrAt(at, OWNED.ptr), n.size(at, OWNED.len)));
  }
  return keys;
}

function walk(root: bigint, path: readonly Step[], keys: readonly (Uint8Array | null)[]): bigint | null {
  let node: bigint | null = root;
  for (let i = 0; i < path.length && node !== null; i++) {
    const step = path[i]!;
    node = typeof step === 'number' ? listAt(node, step) : mapFind(node, keys[i]!);
  }
  return node;
}

function numberText(node: bigint | null): string {
  if (node === null || tagOf(node) !== Tag.NUMBER) return '';
  return ownedText(node);
}

const INT64_MIN = -(2n ** 63n);
const INT64_MAX = 2n ** 63n - 1n;

function nodeToJS(node: bigint | null, options: ReadOptions): unknown {
  switch (tagOf(node)) {
    case Tag.ABSENT:
      return undefined;
    case Tag.NULL:
      return null;
    case Tag.BOOL:
      return n.u8(payload(node!)) !== 0;
    case Tag.NUMBER:
      return readNumber(ownedText(node!), options.numbers);
    case Tag.STRING:
      return ownedText(node!);
    case Tag.BYTES:
      return ownedBytes(node!);
    case Tag.LIST: {
      const len = ownedLen(node!);
      const base = n.ptrAt(payload(node!), OWNED.ptr);
      const out: unknown[] = [];
      for (let i = 0; i < len; i++) out.push(nodeToJS(base + BigInt(i * SIZE.value), options));
      return out;
    }
    case Tag.MAP: {
      const keys = mapKeys(node!);
      const base = n.ptrAt(payload(node!), OWNED.ptr);
      if (options.maps === 'map') {
        const out = new Map<string, unknown>();
        keys.forEach((key, i) => out.set(key, nodeToJS(base + BigInt(i * SIZE.entry + ENTRY.value), options)));
        return out;
      }
      const out: Record<string, unknown> = {};
      keys.forEach((key, i) => define(out, key, nodeToJS(base + BigInt(i * SIZE.entry + ENTRY.value), options)));
      return out;
    }
    default:
      // A tag this binding does not know is skipped, as the header asks.
      return undefined;
  }
}

// ---- writing, through the library -----------------------------------------------

/**
 * Writes `value` into the node at `raw`, which must be empty (absent or
 * null). On any failure `raw` is freed and left empty, so nobody holds a
 * half-built tree. Each child is built in scratch and moved in; the
 * scratch is freed either way, which frees a child a refused move left
 * there.
 * @internal
 */
export function writeJS(raw: bigint, value: unknown, alloc: bigint, seen: Set<object> = new Set()): void {
  try {
    writeNode(raw, value, alloc, seen);
  } catch (e) {
    n.freeNode(raw);
    throw e;
  }
}

function writeNode(raw: bigint, value: unknown, alloc: bigint, seen: Set<object>): void {
  const fn = n.core().fn;
  const ok = (status: number): void => raiseForStatus(status);
  if (value === undefined) return ok(fn.guatiao_value_absent!(raw));
  if (value === null) return ok(fn.guatiao_value_null!(raw));
  if (value instanceof Ref) {
    return ok(fn.guatiao_value_clone!(alloc, value.requireNode(), raw));
  }
  switch (typeof value) {
    case 'boolean':
      return ok(fn.guatiao_value_bool!(value, raw));
    case 'number':
    case 'bigint':
      return ok(fn.guatiao_value_number!(alloc, n.str(numberTextFor(value)), raw));
    case 'string':
      return ok(fn.guatiao_value_string!(alloc, n.str(value), raw));
    case 'object':
      break;
    default:
      throw new TypeError(`cannot convert a ${typeof value} to a guatiao value`);
  }
  const bytes = asBytes(value);
  if (bytes !== undefined) return ok(fn.guatiao_value_bytes!(alloc, n.bytesArg(bytes), raw));
  if (seen.has(value)) throw new TypeError('the value refers to itself');
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      ok(fn.guatiao_value_list!(alloc, raw));
      withScratch((tmp) => {
        for (const item of value as unknown[]) {
          writeJS(tmp, item, alloc, seen);
          try {
            ok(fn.guatiao_list_push!(alloc, raw, tmp));
          } finally {
            n.freeNode(tmp);
          }
        }
      });
      return;
    }
    let entries: Iterable<[unknown, unknown]>;
    if (value instanceof Map) entries = value.entries() as Iterable<[unknown, unknown]>;
    else if (isPlainObject(value)) entries = Object.entries(value);
    else throw new TypeError(`cannot convert a ${value.constructor?.name ?? 'object'} to a guatiao value`);
    ok(fn.guatiao_value_map!(alloc, raw));
    withScratch((tmp) => {
      for (const [key, item] of entries) {
        if (typeof key !== 'string') throw new TypeError('a map key is a string');
        writeJS(tmp, item, alloc, seen);
        try {
          ok(fn.guatiao_map_set!(alloc, raw, n.str(key), tmp));
        } finally {
          n.freeNode(tmp);
        }
      }
    });
  } finally {
    seen.delete(value);
  }
}

function withScratch(body: (tmp: bigint) => void): void {
  const tmp = n.alloc(SIZE.value);
  try {
    body(tmp);
  } finally {
    n.release(tmp);
  }
}

// ---- Ref, MapRef, ListRef ----------------------------------------------------

const inspect = Symbol.for('nodejs.util.inspect.custom');

/**
 * A node inside a tree: its owner plus a path of keys and indices. Every
 * access walks the path again; nothing here keeps an address.
 *
 * @guatiaoType tree_value
 */
export class Ref {
  /** @internal */
  readonly owner: Owner;
  readonly path: readonly Step[];
  readonly #keys: readonly (Uint8Array | null)[];

  /** @internal `owner` null: the ref is its own owner, as a `Value` is. */
  constructor(owner: Owner | null, path: readonly Step[] = []) {
    this.owner = owner ?? (this as unknown as Owner);
    this.path = Object.freeze([...path]);
    this.#keys = this.path.map((step) => (typeof step === 'string' ? n.encode(step) : null));
  }

  /** The node's address now, or null when the path names nothing. @internal */
  node(): bigint | null {
    this.owner.checkOpen();
    return walk(this.owner.root(), this.path, this.#keys);
  }

  /** @internal */
  requireNode(): bigint {
    const node = this.node();
    if (node === null) throw new RangeError(`no value at ${JSON.stringify(this.path)}`);
    return node;
  }

  /** `Tag` of the node; `Tag.ABSENT` when the path names nothing. */
  get tag(): number {
    return tagOf(this.node());
  }

  isAbsent(): boolean {
    return this.tag === Tag.ABSENT;
  }

  isNull(): boolean {
    return this.tag === Tag.NULL;
  }

  /** The boolean, or `fallback` for anything else. */
  boolOr(fallback = false): boolean {
    const node = this.node();
    return tagOf(node) === Tag.BOOL ? n.u8(payload(node!)) !== 0 : fallback;
  }

  /**
   * The number as a signed 64-bit integer, or `fallback`: a wrong kind, a
   * fraction or an exponent spelling, or a value outside the range all
   * fall back, never truncate.
   */
  intOr(fallback = 0n): bigint {
    const text = numberText(this.node());
    if (!text || text.length >= 48 || /[.eE]/.test(text)) return fallback;
    let value: bigint;
    try {
      value = BigInt(text);
    } catch {
      return fallback;
    }
    return value >= INT64_MIN && value <= INT64_MAX ? value : fallback;
  }

  /** The number as a `number`, or `fallback`. */
  numberOr(fallback = 0): number {
    const text = numberText(this.node());
    if (!text || text.length >= 344) return fallback;
    const value = Number(text);
    return Number.isNaN(value) ? fallback : value;
  }

  /** The text, or `fallback` for anything else. */
  stringOr(fallback = ''): string {
    const node = this.node();
    return tagOf(node) === Tag.STRING ? ownedText(node!) : fallback;
  }

  /**
   * The whole node as plain data: `undefined` for absent, `null`, booleans,
   * numbers (see `ReadOptions`), strings, `Uint8Array`, arrays and plain
   * objects (or `Map`s). An unchecked cast to `T`.
   */
  toJS<T = unknown>(options: ReadOptions = {}): T {
    return nodeToJS(this.node(), options) as T;
  }

  /** This node as a map; `TypeError` when it is not one. */
  asMap(): MapRef {
    if (this.tag !== Tag.MAP) throw new TypeError('the value is not a map');
    return new MapRef(this.owner, this.path);
  }

  /** This node as a list; `TypeError` when it is not one. */
  asList(): ListRef {
    if (this.tag !== Tag.LIST) throw new TypeError('the value is not a list');
    return new ListRef(this.owner, this.path);
  }

  /**
   * This node in the wire encoding: a tag byte per node, LEB128 lengths, a
   * number as its own text, a map in its order. `Value.fromWire` in any
   * language reads the same tree back.
   */
  toWire(): Uint8Array {
    const node = this.requireNode();
    const core = n.core();
    const out = n.alloc(SIZE.value);
    try {
      raiseForStatus(core.fn.guatiao_wire_encode!(core.alloc, node, out));
      return ownedBytes(out);
    } finally {
      n.freeNode(out);
      n.release(out);
    }
  }

  /** A deep, independent copy of this node, as a new `Value` to close. */
  clone(): Value {
    const node = this.requireNode();
    const alloc = this.owner.cloneAlloc();
    const out = n.alloc(SIZE.value);
    try {
      raiseForStatus(n.core().fn.guatiao_value_clone!(alloc, node, out));
    } catch (e) {
      n.release(out);
      throw e;
    }
    return adopt(out, alloc);
  }

  [inspect](): string {
    try {
      return `${this.constructor.name} ${JSON.stringify(this.toJS({ numbers: 'text' }), (_k, v: unknown) =>
        v instanceof Uint8Array ? `<${v.length} bytes>` : v,
      )}`;
    } catch {
      return `${this.constructor.name} <unreadable>`;
    }
  }
}

/**
 * A `Ref` whose node is a map, read and written through the map protocol.
 *
 * @guatiaoType tree_map
 */
export class MapRef extends Ref {
  #node(): bigint | null {
    const node = this.node();
    return node !== null && tagOf(node) === Tag.MAP ? node : null;
  }

  get size(): number {
    const node = this.#node();
    return node === null ? 0 : ownedLen(node);
  }

  has(key: string): boolean {
    return mapFind(this.#node(), n.encode(key)) !== null;
  }

  /** The entry under `key`, or `undefined` when there is none. */
  get(key: string): Ref | undefined {
    return this.has(key) ? new Ref(this.owner, [...this.path, key]) : undefined;
  }

  /** Writes `value` under `key`, replacing what was there. Returns this map. */
  set(key: string, value: unknown): this {
    this.owner.checkWritable();
    const alloc = this.owner.buildAlloc();
    withScratch((tmp) => {
      writeJS(tmp, value, alloc);
      try {
        raiseForStatus(n.core().fn.guatiao_map_set!(alloc, this.requireNode(), n.str(key), tmp));
      } finally {
        n.freeNode(tmp);
      }
    });
    return this;
  }

  /** Removes and frees the entry under `key`; whether there was one. */
  delete(key: string): boolean {
    this.owner.checkWritable();
    const status = n.core().fn.guatiao_map_discard!(this.requireNode(), n.str(key)) as number;
    if (status === Status.ERR_NOT_FOUND) return false;
    raiseForStatus(status);
    return true;
  }

  /** The keys, in insertion order, as they are now. */
  keys(): string[] {
    const node = this.requireNode();
    if (tagOf(node) !== Tag.MAP) throw new TypeError('the value is not a map');
    return mapKeys(node);
  }

  values(): Ref[] {
    return this.keys().map((key) => new Ref(this.owner, [...this.path, key]));
  }

  entries(): [string, Ref][] {
    return this.keys().map((key) => [key, new Ref(this.owner, [...this.path, key])]);
  }

  [Symbol.iterator](): IterableIterator<[string, Ref]> {
    return this.entries()[Symbol.iterator]();
  }

  clear(): void {
    this.owner.checkWritable();
    raiseForStatus(n.core().fn.guatiao_map_clear!(this.requireNode()));
  }

  /** Copies every entry of `source`, replacing collisions in place. */
  copyFrom(source: Ref): void {
    this.owner.checkWritable();
    const alloc = this.owner.buildAlloc();
    raiseForStatus(n.core().fn.guatiao_map_copy_from!(alloc, this.requireNode(), source.requireNode()));
  }
}

/**
 * A `Ref` whose node is a list. The C surface appends, discards by index
 * and clears; `set` and `insert` are built from those (every element
 * cloned, the list cleared, the new order pushed back) and are O(n).
 *
 * @guatiaoType tree_list
 */
export class ListRef extends Ref {
  get length(): number {
    const node = this.node();
    return node !== null && tagOf(node) === Tag.LIST ? ownedLen(node) : 0;
  }

  #index(index: number): number {
    const node = this.requireNode();
    if (tagOf(node) !== Tag.LIST) throw new TypeError('the value is not a list');
    const len = ownedLen(node);
    const i = index < 0 ? index + len : index;
    if (!Number.isInteger(i) || i < 0 || i >= len) throw new RangeError(`index ${index} is outside a list of ${len}`);
    return i;
  }

  /** The element at `index` (negative counts from the end), or `undefined`. */
  at(index: number): Ref | undefined {
    const len = this.length;
    const i = index < 0 ? index + len : index;
    return Number.isInteger(i) && i >= 0 && i < len ? new Ref(this.owner, [...this.path, i]) : undefined;
  }

  /** Appends each value. Returns the new length. */
  push(...values: unknown[]): number {
    this.owner.checkWritable();
    const alloc = this.owner.buildAlloc();
    withScratch((tmp) => {
      for (const value of values) {
        writeJS(tmp, value, alloc);
        try {
          raiseForStatus(n.core().fn.guatiao_list_push!(alloc, this.requireNode(), tmp));
        } finally {
          n.freeNode(tmp);
        }
      }
    });
    return this.length;
  }

  /** Removes and frees the element at `index`. `RangeError` past the end. */
  removeAt(index: number): void {
    this.owner.checkWritable();
    const i = this.#index(index);
    raiseForStatus(n.core().fn.guatiao_list_discard!(this.requireNode(), i));
  }

  clear(): void {
    this.owner.checkWritable();
    raiseForStatus(n.core().fn.guatiao_list_clear!(this.requireNode()));
  }

  /** Replaces the element at `index`. O(n). */
  set(index: number, value: unknown): void {
    this.owner.checkWritable();
    const i = this.#index(index);
    this.#rebuild((elements, alloc) => {
      const replacement = n.alloc(SIZE.value);
      try {
        writeJS(replacement, value, alloc);
      } catch (e) {
        n.release(replacement);
        throw e;
      }
      n.freeNode(elements[i]!);
      n.release(elements[i]!);
      elements[i] = replacement;
    });
  }

  /** Inserts `value` before `index` (clamped to the list). O(n). */
  insert(index: number, value: unknown): void {
    this.owner.checkWritable();
    this.#rebuild((elements, alloc) => {
      const len = elements.length;
      const i = Math.min(index < 0 ? Math.max(0, len + index) : index, len);
      const replacement = n.alloc(SIZE.value);
      try {
        writeJS(replacement, value, alloc);
      } catch (e) {
        n.release(replacement);
        throw e;
      }
      elements.splice(i, 0, replacement);
    });
  }

  #rebuild(change: (elements: bigint[], alloc: bigint) => void): void {
    const node = this.requireNode();
    if (tagOf(node) !== Tag.LIST) throw new TypeError('the value is not a list');
    const alloc = this.owner.buildAlloc();
    const fn = n.core().fn;
    const elements: bigint[] = [];
    try {
      const len = ownedLen(node);
      const base = n.ptrAt(payload(node), OWNED.ptr);
      for (let i = 0; i < len; i++) {
        const copy = n.alloc(SIZE.value);
        elements.push(copy);
        raiseForStatus(fn.guatiao_value_clone!(alloc, base + BigInt(i * SIZE.value), copy));
      }
      change(elements, alloc);
      raiseForStatus(fn.guatiao_list_clear!(this.requireNode()));
      for (const element of elements) raiseForStatus(fn.guatiao_list_push!(alloc, this.requireNode(), element));
    } finally {
      for (const element of elements) {
        n.freeNode(element);
        n.release(element);
      }
    }
  }

  [Symbol.iterator](): IterableIterator<Ref> {
    const refs: Ref[] = [];
    for (let i = 0; i < this.length; i++) refs.push(new Ref(this.owner, [...this.path, i]));
    return refs[Symbol.iterator]();
  }
}

// ---- Value --------------------------------------------------------------------

/** Options for building a `Value`. */
export interface ValueOptions {
  /**
   * The address of a `guatiao_alloc` to build in, which the caller keeps
   * alive for as long as the tree lives. Default: the library's own.
   */
  alloc?: bigint;
}

const ADOPT = Symbol('adopt');
interface Adopting extends ValueOptions {
  [ADOPT]?: bigint;
}

const finalizer = new FinalizationRegistry<bigint>((root) => {
  n.freeNode(root);
  n.release(root);
});

/**
 * Owns a root node, freed by `close()` (or `using`); a value dropped
 * without one is freed when it is collected.
 *
 * `new Value(obj)` converts plain data: `undefined` is absent, `null`
 * null, then booleans, numbers and `bigint`s (written as their exact
 * text), strings, `Uint8Array` / `ArrayBuffer` / any typed array (bytes),
 * arrays (lists), and plain objects or `Map`s with string keys (maps). A
 * `Value` or `Ref` is deep-copied. Anything else is a `TypeError`, and a
 * non-finite number a `RangeError`.
 */
export class Value extends Ref implements Disposable {
  #root: bigint;
  #alloc: bigint | undefined;
  #closed = false;

  constructor(value?: unknown, options: ValueOptions = {}) {
    super(null, []);
    const adopted = (options as Adopting)[ADOPT];
    this.#alloc = options.alloc;
    this.#root = adopted ?? n.alloc(SIZE.value);
    if (adopted === undefined && value !== undefined) {
      try {
        writeJS(this.#root, value, this.buildAlloc());
      } catch (e) {
        n.release(this.#root);
        this.#closed = true;
        throw e;
      }
    }
    finalizer.register(this, this.#root, this);
  }

  /** @internal */
  root(): bigint {
    return this.#root;
  }

  /** @internal */
  checkOpen(): void {
    if (this.#closed) throw new TypeError('this value is closed');
  }

  /** @internal */
  checkWritable(): void {
    this.checkOpen();
  }

  /** @internal */
  buildAlloc(): bigint {
    return this.#alloc ?? n.core().alloc;
  }

  /** @internal */
  cloneAlloc(): bigint {
    return this.buildAlloc();
  }

  get closed(): boolean {
    return this.#closed;
  }

  /** Frees the tree; a second call does nothing. */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    finalizer.unregister(this);
    try {
      n.freeNode(this.#root);
    } finally {
      n.release(this.#root);
    }
  }

  [Symbol.dispose](): void {
    this.close();
  }

  /** Converts `value` as the constructor does. */
  static from(value: unknown, options: ValueOptions = {}): Value {
    return new Value(value, options);
  }

  static absent(): Value {
    return new Value();
  }

  static null(): Value {
    return new Value(null);
  }

  static bool(b: boolean): Value {
    return new Value(Boolean(b));
  }

  static string(text: string, options: ValueOptions = {}): Value {
    return new Value(String(text), options);
  }

  static bytes(data: Uint8Array | ArrayBuffer, options: ValueOptions = {}): Value {
    return new Value(data, options);
  }

  /** A number from its exact text, verbatim: no `number` on the way. */
  static number(text: string, options: ValueOptions = {}): Value {
    const value = new Value(undefined, options);
    try {
      raiseForStatus(n.core().fn.guatiao_value_number!(value.buildAlloc(), n.str(text), value.#root));
    } catch (e) {
      value.close();
      throw e;
    }
    return value;
  }

  /**
   * The value the wire encoding in `data` holds. `BadValue` for bytes that
   * are not exactly one value: truncated, an unknown tag, a length not in
   * its shortest form, text that is not UTF-8, a number outside the
   * grammar, a repeated key, anything after the value.
   */
  static fromWire(data: Uint8Array, options: ValueOptions = {}): Value {
    const value = new Value(undefined, options);
    try {
      raiseForStatus(
        n.core().fn.guatiao_wire_decode!(value.buildAlloc(), n.bytesArg(data), value.#root),
        'bytes that are not a value in the wire encoding',
      );
    } catch (e) {
      value.close();
      throw e;
    }
    return value;
  }

  static map(options: ValueOptions = {}): Value {
    return new Value({}, options);
  }

  static list(options: ValueOptions = {}): Value {
    return new Value([], options);
  }

  /**
   * Moves the root out: the address of a fresh node holding the tree,
   * which the caller now owns; this value is left absent.
   * @internal
   */
  moveOut(): bigint {
    this.checkOpen();
    const moved = n.alloc(SIZE.value);
    n.copy(moved, this.#root, SIZE.value);
    n.zero(this.#root, SIZE.value);
    return moved;
  }

  override [inspect](): string {
    return this.#closed ? 'Value <closed>' : super[inspect]();
  }
}

/**
 * A `Value` taking over the root node at `root`, which a call wrote into
 * memory from `alloc()` (this package's): the `Value` frees the tree and
 * that memory. `alloc` is the allocator the tree was built in, kept to
 * grow it with; the core library's default when not given.
 */
export function adopt(root: bigint, alloc?: bigint): Value {
  const options: Adopting = { [ADOPT]: root };
  if (alloc !== undefined) options.alloc = alloc;
  return new Value(undefined, options);
}

/**
 * A read-only owner over memory this binding does not free: `find` asks
 * for the node again on every access, and `keep`, when given, is the
 * owner the memory belongs to, checked open first.
 * @internal
 */
export class Borrowed implements Owner {
  readonly #find: () => bigint | null;
  readonly #keep: Owner | undefined;

  constructor(find: () => bigint | null, keep?: Owner) {
    this.#find = find;
    this.#keep = keep;
  }

  root(): bigint {
    const node = this.#find();
    if (node === null || node === 0n) throw new RangeError('the borrowed value is gone');
    return node;
  }

  checkOpen(): void {
    this.#keep?.checkOpen();
  }

  checkWritable(): void {
    throw new TypeError('a borrowed value is read-only');
  }

  buildAlloc(): bigint {
    throw new TypeError('a borrowed value is read-only');
  }

  cloneAlloc(): bigint {
    return n.core().alloc;
  }
}

/**
 * The node `ref` names now, to pass to a call while nothing changes its
 * tree. Throws once its value is closed, and a `RangeError` when its path
 * names nothing.
 */
export function nodeOf(ref: Ref): bigint {
  return ref.requireNode();
}

/**
 * A read-only `Ref` onto the node `find` answers, asked again on every
 * access, which throws once `source`'s value is closed and refuses every
 * write. `find` answers `null` (or `0n`) once the node is gone.
 */
export function borrow(source: Ref, find: () => bigint | null): Ref {
  return new Ref(new Borrowed(find, source.owner), []);
}

/**
 * Runs `body` with each item as a node: a `Ref` as it is, plain data built
 * for the call (in the core library's allocator) and freed after it.
 */
export function withNodes<T>(items: readonly unknown[], body: (nodes: bigint[]) => T): T {
  const built: Value[] = [];
  try {
    const nodes = items.map((item) => {
      if (item instanceof Ref) return item.requireNode();
      const value = new Value(item);
      built.push(value);
      return value.requireNode();
    });
    return body(nodes);
  } finally {
    for (const value of built) value.close();
  }
}

export { Tag };
