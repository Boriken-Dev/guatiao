// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A kind's function table, from JavaScript: calling one, and serving one.
 *
 * A kind is declared once, from its C header, as a typed value: a method
 * per slot in the header's order, the shapes from `t`, and the
 * `FLOOR_HASH` the header defines. TypeScript infers from it what a
 * provider implements (`Implementation`) and what a caller holds
 * (`Remote`).
 *
 * ```ts
 * export const Listener = kind('listener', {
 *   floorHash: 1867921573,
 *   object: true,
 *   methods: { heard: { args: [t.str], required: true } },
 * });
 * export const Greeter = kind('greeter', {
 *   floorHash: 2695856491,
 *   methods: {
 *     greet: { args: [t.str], answer: t.map, required: true, fallible: true },
 *     greetInto: { args: [t.str, t.lent(Listener)], fallible: true },
 *   },
 * });
 * export type Greeter = Implementation<typeof Greeter>;
 * ```
 *
 * @module
 */

import { ALLOC, KIND_HEADER, OBJECT, OWNED, PROVIDER_ERROR, SIZE, Status, Tag, VALUE, WORD } from './abi.js';
import { BadValue, FloorMismatch, GuatiaoError, Internal, NullArgument, Unsupported, errorFor } from './errors.js';
import * as n from './native.js';
import { ListRef, MapRef, Ref, Value, adopt, writeJS, type Owner } from './value.js';

// ---- the types a description gives ---------------------------------------------

declare const types: unique symbol;

/**
 * One argument or answer of a slot, as the C header spells it. The type
 * parameters say what crosses: what a caller passes and what a method
 * receives, what a method returns and what a caller gets.
 */
export interface Shape<CallIn = unknown, ImplIn = unknown, CallOut = unknown, ImplOut = unknown> {
  readonly name: string;
  /** What crosses, for the type checker only: nothing holds it at run time. */
  readonly [types]: { callIn: CallIn; implIn: ImplIn; callOut: CallOut; implOut: ImplOut };
}

/** One slot: its arguments, its answer, and whether it is required and fallible. */
export interface MethodSpec {
  /** The arguments after `ctx`, in the header's order. */
  readonly args?: readonly Shape<any, any, any, any>[];
  /** A shape, an object kind (an object the caller then owns), or none. */
  readonly answer?: Shape<any, any, any, any> | Kind<any, true>;
  /** A slot every table fills: one the kind was born with, which its floor hash covers. */
  readonly required?: boolean;
  /** A slot ending in `guatiao_provider_error *err`: it can refuse. */
  readonly fallible?: boolean;
}

export type Methods = { readonly [name: string]: MethodSpec };

/** A kind, as `kind()` describes it. */
export interface Kind<M extends Methods = Methods, O extends boolean = boolean> {
  readonly name: string;
  /** The `<table>_FLOOR_HASH` the header defines, as written there. */
  readonly floorHash: number;
  /** An object kind: `destroy` first, an object one holder owns. */
  readonly isObject: O;
  readonly methods: M;
  /** The table's size in bytes: the header, `destroy`, a word per slot. */
  readonly tableSize: number;
  /** The smallest table this kind accepts: up to its last required slot. */
  readonly floor: number;
  /** A caller's handle on a table of this kind, over `ctx`. */
  bind(table: bigint, size: number, ctx: bigint | null): Remote<Kind<M, O>>;
}

type CallArgs<S extends MethodSpec> = S['args'] extends infer A extends readonly unknown[]
  ? { -readonly [I in keyof A]: A[I] extends Shape<infer X, any, any, any> ? X : never }
  : [];
type ImplArgs<S extends MethodSpec> = S['args'] extends infer A extends readonly unknown[]
  ? { -readonly [I in keyof A]: A[I] extends Shape<any, infer X, any, any> ? X : never }
  : [];
type CallAnswer<A> =
  A extends Kind<any, true> ? ObjectHandle<A> : A extends Shape<any, any, infer X, any> ? X : void;
type ImplAnswer<A> =
  A extends Kind<any, true> ? Implementation<A> | ObjectHandle<A> : A extends Shape<any, any, any, infer X> ? X : void;
type CallMethod<S extends MethodSpec> = (...args: CallArgs<S>) => CallAnswer<S['answer']>;
type ImplMethod<S extends MethodSpec> = (...args: ImplArgs<S>) => ImplAnswer<S['answer']>;

/**
 * What implements a kind: a provider class, or an object handed to a
 * table. Required methods are mandatory, the rest optional.
 */
export type Implementation<K> =
  K extends Kind<infer M, any>
    ? { [N in keyof M as M[N]['required'] extends true ? N : never]: ImplMethod<M[N]> } & {
        [N in keyof M as M[N]['required'] extends true ? never : N]?: ImplMethod<M[N]>;
      }
    : never;

/** A caller's handle on a table: a method per slot, and what it is. */
export interface RemoteBase<K> {
  readonly kind: K;
  readonly table: bigint;
  readonly size: number;
  readonly ctx: bigint | null;
  /** Whether the table carries the slot for `name`. */
  has(name: K extends Kind<infer M, any> ? keyof M & string : string): boolean;
}

/** A caller's handle on a table of `K`. */
export type Remote<K> = RemoteBase<K> & (K extends Kind<infer M, any> ? { [N in keyof M]: CallMethod<M[N]> } : never);

/** An object of an object kind this side owns: `close()` destroys it, once. */
export type ObjectHandle<K> = Remote<K> & Disposable & { close(): void; readonly closed: boolean };

/** What a slot answering a map may return. */
export type MapAnswer = MapRef | Value | { readonly [key: string]: unknown } | ReadonlyMap<string, unknown>;
/** What a slot answering a list may return. */
export type ListAnswer = ListRef | Value | readonly unknown[];

// ---- one call's lifetime ------------------------------------------------------------

/**
 * What one call keeps alive and what runs once it returns. A view lent to
 * a callee checks it on every access, so one kept past the call throws
 * instead of reading memory that is no longer lent.
 * @internal
 */
export class Scope {
  #open = true;
  readonly #keep: unknown[] = [];
  readonly #after: (() => void)[] = [];
  #given: (() => void)[] = [];
  readonly #blocks: bigint[] = [];

  get open(): boolean {
    return this.#open;
  }

  keep<T>(value: T): T {
    this.#keep.push(value);
    return value;
  }

  /** Runs once the call is over, in reverse order. */
  after(fn: () => void): void {
    this.#after.push(fn);
  }

  /** Undoes a hand-over if the call never happens. */
  given(fn: () => void): void {
    this.#given.push(fn);
  }

  /** The call happened: what was handed over is the callee's. */
  called(): void {
    this.#given = [];
  }

  /** `len` zeroed bytes, released when the scope closes. */
  block(len: number): bigint {
    return this.own(n.alloc(len));
  }

  /** Memory from `n.alloc`, released last when the scope closes. */
  own(ptr: bigint): bigint {
    this.#blocks.push(ptr);
    return ptr;
  }

  checkOpen(): void {
    if (!this.#open) throw new TypeError('this view was lent for one call, which has returned');
  }

  close(): void {
    if (!this.#open) return;
    this.#open = false;
    let failure: unknown;
    const run = (fns: (() => void)[]): void => {
      for (const fn of fns.reverse()) {
        try {
          fn();
        } catch (e) {
          failure ??= e;
        }
      }
    };
    run(this.#after.splice(0));
    run(this.#given.splice(0));
    for (const ptr of this.#blocks.splice(0)) n.release(ptr);
    this.#keep.length = 0;
    if (failure !== undefined) throw failure;
  }
}

/** A `Ref` owner over memory a call lends: valid while the scope is open, writable only when lent. */
class CallView implements Owner {
  readonly #node: bigint;
  readonly #scope: Scope;
  readonly #alloc: bigint | undefined;

  constructor(node: bigint, scope: Scope, alloc?: bigint) {
    this.#node = node;
    this.#scope = scope;
    this.#alloc = alloc;
  }

  root(): bigint {
    return this.#node;
  }

  checkOpen(): void {
    this.#scope.checkOpen();
  }

  checkWritable(): void {
    this.#scope.checkOpen();
    if (this.#alloc === undefined) throw new TypeError('a borrowed value is read-only');
  }

  buildAlloc(): bigint {
    this.checkWritable();
    return this.#alloc!;
  }

  cloneAlloc(): bigint {
    return n.core().alloc;
  }
}

// ---- moving trees ----------------------------------------------------------------

const binding = (): bigint => n.core().alloc;

/** Whether `ptr` names an allocator that can build. */
function complete(ptr: bigint | null): boolean {
  if (!ptr) return false;
  return n.u32(ptr, ALLOC.structSize) >= ALLOC.free + WORD && n.ptrAt(ptr, ALLOC.alloc) !== 0n && n.ptrAt(ptr, ALLOC.free) !== 0n;
}

const TAG_NAMES: Readonly<Record<number, string>> = Object.fromEntries(
  Object.entries(Tag).map(([name, code]) => [code, name.toLowerCase()]),
);

/**
 * A fresh root node, from `alloc`ed memory the caller releases, holding
 * `value`'s tree given up by whoever held it: a `Value`, or a view of a
 * whole one, is moved (it is left absent); another `Ref` is copied into
 * `alloc`; plain data is built there.
 */
function ownedTree(value: unknown, tag: number | undefined, alloc: bigint): bigint {
  let source: unknown = value;
  if (source instanceof Ref && !(source instanceof Value) && source.path.length === 0 && source.owner instanceof Value) {
    source = source.owner;
  }
  let root: bigint;
  if (source instanceof Value) {
    if (tag !== undefined && source.tag !== tag) {
      throw new TypeError(`expected a ${TAG_NAMES[tag]}, got a ${TAG_NAMES[source.tag] ?? 'value'}`);
    }
    return source.moveOut();
  }
  root = n.alloc(SIZE.value);
  try {
    if (source instanceof Ref) raiseCheck(n.core().fn.guatiao_value_clone!(alloc, source.requireNode(), root));
    else writeJS(root, source, alloc);
    if (tag !== undefined && n.u32(root, VALUE.tag) !== tag) {
      throw new TypeError(`expected a ${TAG_NAMES[tag]}`);
    }
  } catch (e) {
    n.freeNode(root);
    n.release(root);
    throw e;
  }
  return root;
}

function raiseCheck(status: number): void {
  if (status !== Status.OK) throw errorFor(status);
}

/** Fills a `guatiao_provider_error *` with `status` and owned `message`. */
function writeErr(err: bigint | null, status: number, message: string): void {
  if (!err) return;
  n.setU32(err, PROVIDER_ERROR.status, status);
  const at = err + BigInt(PROVIDER_ERROR.message);
  n.zero(at, SIZE.string);
  if (!message) return;
  try {
    const text = new Value(message);
    const moved = text.moveOut();
    text.close();
    n.copy(at, moved + BigInt(VALUE.payload), SIZE.string);
    n.release(moved);
  } catch {
    n.zero(at, SIZE.string);
  }
}

// ---- shapes ----------------------------------------------------------------------

type Mode = 'borrow' | 'take' | 'lend' | 'answer';

/** The runtime half of a shape. @internal */
abstract class ShapeImpl {
  abstract readonly name: string;
  /** The koffi type in the slot's prototype, as an argument; undefined for none. */
  readonly cArg: string | undefined = undefined;
  /** Whether it can be an answer (written through a `void *out`). */
  readonly answers: boolean = false;
  /** Handed over: the callee owns it on every path. */
  readonly takes: boolean = false;

  // the caller
  pass(_value: unknown, _scope: Scope): unknown {
    throw new TypeError(`${this.name} is not an argument`);
  }
  outSlot(_scope: Scope): bigint {
    throw new TypeError(`${this.name} is not an answer`);
  }
  answer(_out: bigint, _scope: Scope): unknown {
    throw new TypeError(`${this.name} is not an answer`);
  }

  // the callee
  receive(_arg: unknown, _scope: Scope): unknown {
    throw new TypeError(`${this.name} is not an argument`);
  }
  write(_out: bigint, _result: unknown): void {
    throw new TypeError(`${this.name} is not an answer`);
  }
  /** Releases what `receive` handed a method that never ran. */
  unused(_received: unknown): void {}

  toString(): string {
    return `t.${this.name}`;
  }
}

type Width = { size: number; read: (v: DataView) => unknown; write: (v: DataView, x: unknown) => void };

class Scalar extends ShapeImpl {
  override readonly cArg: string;
  override readonly answers = true;
  readonly #kind: 'bool' | 'int' | 'float';
  readonly #wide: boolean;
  readonly #lo: bigint;
  readonly #hi: bigint;
  readonly #width: Width;

  constructor(
    readonly name: string,
    cType: string,
    kind: 'bool' | 'int' | 'float',
    bits = 0,
    signed = false,
  ) {
    super();
    this.cArg = cType;
    this.#kind = kind;
    this.#wide = bits === 64;
    this.#lo = signed ? -(1n << BigInt(bits - 1)) : 0n;
    this.#hi = signed ? (1n << BigInt(bits - 1)) - 1n : (1n << BigInt(bits)) - 1n;
    this.#width = widthOf(cType);
  }

  #checked(value: unknown): unknown {
    if (this.#kind === 'bool') {
      if (typeof value !== 'boolean') throw new TypeError(`t.${this.name} takes a boolean, not ${typeof value}`);
      return value;
    }
    if (this.#kind === 'float') {
      if (typeof value !== 'number') throw new TypeError(`t.${this.name} takes a number, not ${typeof value}`);
      return value;
    }
    if (typeof value === 'number' && !Number.isInteger(value)) throw new TypeError(`t.${this.name} takes an integer, not ${value}`);
    if (typeof value !== 'number' && typeof value !== 'bigint') {
      throw new TypeError(`t.${this.name} takes an integer, not ${typeof value}`);
    }
    if (!this.#wide && typeof value === 'bigint') throw new TypeError(`t.${this.name} takes a number`);
    const big = BigInt(value);
    if (big < this.#lo || big > this.#hi) throw new RangeError(`${value} does not fit t.${this.name}`);
    return this.#wide ? big : value;
  }

  #normal(value: unknown): unknown {
    if (this.#kind === 'bool') return Boolean(value);
    if (this.#kind === 'float') return Number(value);
    if (!this.#wide) return Number(value);
    // koffi hands a 64-bit integer back as a number or a bigint, and an
    // unsigned one past 2^63 as negative: bring it to the declared width.
    const big = BigInt(value as number | bigint);
    return this.#lo < 0n ? BigInt.asIntN(64, big) : BigInt.asUintN(64, big);
  }

  override pass(value: unknown): unknown {
    return this.#checked(value);
  }
  override outSlot(scope: Scope): bigint {
    return scope.block(8);
  }
  override answer(out: bigint): unknown {
    return this.#normal(this.#width.read(n.view(out, this.#width.size)));
  }
  override receive(arg: unknown): unknown {
    return this.#normal(arg);
  }
  override write(out: bigint, result: unknown): void {
    this.#width.write(n.view(out, this.#width.size), this.#checked(result));
  }
}

function widthOf(cType: string): Width {
  const big = (x: unknown): bigint => BigInt(x as number | bigint);
  switch (cType) {
    case 'bool':
      return { size: 1, read: (v) => v.getUint8(0) !== 0, write: (v, x) => v.setUint8(0, x ? 1 : 0) };
    case 'int8_t':
      return { size: 1, read: (v) => v.getInt8(0), write: (v, x) => v.setInt8(0, x as number) };
    case 'uint8_t':
      return { size: 1, read: (v) => v.getUint8(0), write: (v, x) => v.setUint8(0, x as number) };
    case 'int16_t':
      return { size: 2, read: (v) => v.getInt16(0, true), write: (v, x) => v.setInt16(0, x as number, true) };
    case 'uint16_t':
      return { size: 2, read: (v) => v.getUint16(0, true), write: (v, x) => v.setUint16(0, x as number, true) };
    case 'int32_t':
      return { size: 4, read: (v) => v.getInt32(0, true), write: (v, x) => v.setInt32(0, x as number, true) };
    case 'uint32_t':
      return { size: 4, read: (v) => v.getUint32(0, true), write: (v, x) => v.setUint32(0, x as number, true) };
    case 'int64_t':
    case 'intptr_t':
      return { size: 8, read: (v) => v.getBigInt64(0, true), write: (v, x) => v.setBigInt64(0, big(x), true) };
    case 'uint64_t':
    case 'size_t':
      return { size: 8, read: (v) => v.getBigUint64(0, true), write: (v, x) => v.setBigUint64(0, big(x), true) };
    case 'float':
      return { size: 4, read: (v) => v.getFloat32(0, true), write: (v, x) => v.setFloat32(0, x as number, true) };
    default:
      return { size: 8, read: (v) => v.getFloat64(0, true), write: (v, x) => v.setFloat64(0, x as number, true) };
  }
}

interface StrView {
  ptr: bigint | null;
  len: number | bigint;
}

function textArg(arg: unknown, what: string): string {
  const view = arg as StrView;
  try {
    return n.textAt(view.ptr ?? 0n, Number(view.len));
  } catch {
    throw new BadValue(`${what} is not UTF-8`);
  }
}

class Str extends ShapeImpl {
  readonly name = 'str';
  override readonly cArg = 'guatiao_str';
  override pass(value: unknown): unknown {
    if (typeof value !== 'string') throw new TypeError(`t.str takes a string, not ${typeof value}`);
    return n.str(value);
  }
  override receive(arg: unknown): unknown {
    return textArg(arg, 'the text argument');
  }
}

function bytesOf(value: unknown, what: string): Uint8Array {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError(`${what} takes bytes: a Uint8Array, an ArrayBuffer or a typed array`);
}

class Bytes extends ShapeImpl {
  readonly name = 'bytes';
  override readonly cArg = 'guatiao_bytes';
  override pass(value: unknown): unknown {
    return n.bytesArg(bytesOf(value, 't.bytes'));
  }
  override receive(arg: unknown): unknown {
    const view = arg as StrView;
    return n.bytesAt(view.ptr ?? 0n, Number(view.len));
  }
}

/** An out-buffer: the caller's writable memory, for the call. */
class BytesMut extends ShapeImpl {
  readonly name = 'bytesMut';
  override readonly cArg = 'guatiao_bytes_mut';
  override pass(value: unknown): unknown {
    const bytes = bytesOf(value, 't.bytesMut');
    // The callee writes straight into the caller's memory, pinned for the call.
    const shared = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return { ptr: shared.length ? shared : null, len: shared.length };
  }
  override receive(arg: unknown, scope: Scope): unknown {
    // A copy the method writes into, copied back when it returns: a view
    // over the caller's memory would outlive the call.
    const view = arg as StrView;
    const len = Number(view.len);
    const ptr = view.ptr ?? 0n;
    const copy = n.bytesAt(ptr, len);
    if (len) scope.after(() => n.write(ptr, copy.subarray(0, len)));
    return copy;
  }
}

/** Owned text as an answer (`guatiao_string *out`). */
class Text extends ShapeImpl {
  readonly name = 'text';
  override readonly answers = true;
  override outSlot(scope: Scope): bigint {
    return scope.block(SIZE.string);
  }
  override answer(out: bigint): unknown {
    try {
      return n.textAt(n.ptrAt(out, OWNED.ptr), n.size(out, OWNED.len));
    } catch {
      throw new BadValue('the answer is not UTF-8');
    } finally {
      n.freeString(out);
    }
  }
  override write(out: bigint, result: unknown): void {
    if (typeof result !== 'string') throw new TypeError(`a text answer is a string, not ${typeof result}`);
    const text = new Value(result);
    const moved = text.moveOut();
    text.close();
    n.copy(out, moved + BigInt(VALUE.payload), SIZE.string);
    n.release(moved);
  }
}

/** A value, a map or a list, in one of the ways a tree crosses. */
class Tree extends ShapeImpl {
  override readonly cArg: string | undefined;
  override readonly answers: boolean;
  override readonly takes: boolean;

  constructor(
    readonly name: string,
    readonly tag: number | undefined,
    readonly mode: Mode,
  ) {
    super();
    this.answers = mode === 'answer';
    this.takes = mode === 'take';
    this.cArg = mode === 'answer' ? undefined : 'void *';
  }

  /** The struct that crosses inside `node`: the node for a value, its payload for a map or list. */
  #arm(node: bigint): bigint {
    if (this.tag === undefined) return node;
    if (n.u32(node, VALUE.tag) !== this.tag) throw new TypeError(`expected a ${TAG_NAMES[this.tag]}`);
    return node + BigInt(VALUE.payload);
  }

  /** A node in `into` holding a copy of the struct at `arg`. */
  #nodeOver(arg: bigint, into: bigint): bigint {
    if (this.tag === undefined) {
      n.copy(into, arg, SIZE.value);
    } else {
      n.setU32(into, VALUE.tag, this.tag);
      n.copy(into + BigInt(VALUE.payload), arg, SIZE.map);
    }
    return into;
  }

  #wrap(ref: Ref): Ref {
    if (this.tag === Tag.MAP) return new MapRef(ref.owner, ref.path);
    if (this.tag === Tag.LIST) return new ListRef(ref.owner, ref.path);
    return ref;
  }

  // -- the caller

  override pass(value: unknown, scope: Scope): unknown {
    if (this.mode === 'borrow') {
      if ((value === null || value === undefined) && this.tag === undefined) return null;
      let ref: Ref;
      if (value instanceof Ref) ref = value;
      else {
        const built = scope.keep(new Value(value));
        scope.after(() => built.close());
        ref = built;
      }
      return this.#arm(ref.requireNode());
    }
    if (this.mode === 'lend') {
      if (!(value instanceof Ref)) {
        throw new TypeError(`${this.toString()} takes a Value or a Ref, so what the callee changes has somewhere to go`);
      }
      value.owner.checkWritable();
      return this.#arm(value.requireNode());
    }
    // Handed over: given up now; freed if the call never happens.
    const node = scope.own(ownedTree(value, this.tag, binding()));
    scope.given(() => n.freeNode(node));
    return this.#arm(node);
  }

  override outSlot(scope: Scope): bigint {
    // The carrier: owns nothing, and names the allocator to build in.
    if (this.tag === undefined) {
      const out = scope.block(SIZE.value);
      n.setPtr(out, VALUE.payload + OWNED.alloc, binding());
      return out;
    }
    const out = scope.block(SIZE.map);
    n.setPtr(out, OWNED.alloc, binding());
    return out;
  }

  override answer(out: bigint): unknown {
    return adopt(this.#nodeOver(out, n.alloc(SIZE.value)), binding());
  }

  // -- the callee

  override receive(arg: unknown, scope: Scope): unknown {
    const ptr = (arg as bigint | null) ?? 0n;
    if (!ptr) {
      if (this.mode === 'borrow' && this.tag === undefined) return null;
      throw new NullArgument(`the ${this.name} argument is null`);
    }
    if (this.mode === 'take') {
      return adopt(this.#nodeOver(ptr, n.alloc(SIZE.value)), binding());
    }
    let node: bigint;
    if (this.tag === undefined) {
      node = ptr;
    } else {
      // A node of this side's own over the caller's struct: what the
      // method changes lands here, and is written back once it returns.
      node = this.#nodeOver(ptr, scope.block(SIZE.value));
      if (this.mode === 'lend') scope.after(() => n.copy(ptr, node + BigInt(VALUE.payload), SIZE.map));
    }
    const alloc = this.mode === 'lend' ? recordedAlloc(node) : undefined;
    return this.#wrap(new Ref(new CallView(node, scope, alloc), []));
  }

  override unused(received: unknown): void {
    if (received instanceof Value) received.close();
  }

  override write(out: bigint, result: unknown): void {
    const named = n.ptrAt(out, this.tag === undefined ? VALUE.payload + OWNED.alloc : OWNED.alloc);
    const alloc = complete(named) ? named : binding();
    let node = ownedTree(result, this.tag, alloc);
    if (alloc === named && recordedAlloc(node) !== named) {
      // A tree moved in from elsewhere: copied into the allocator the
      // caller asked for, so nothing it keeps needs another one.
      const copy = n.alloc(SIZE.value);
      try {
        raiseCheck(n.core().fn.guatiao_value_clone!(alloc, node, copy));
      } catch (e) {
        n.release(copy);
        n.freeNode(node);
        n.release(node);
        throw e;
      }
      n.freeNode(node);
      n.release(node);
      node = copy;
    }
    try {
      n.copy(out, this.tag === undefined ? node : node + BigInt(VALUE.payload), this.tag === undefined ? SIZE.value : SIZE.map);
    } finally {
      n.release(node);
    }
  }
}

/** The allocator a container node records, else this binding's. */
function recordedAlloc(node: bigint): bigint {
  const tag = n.u32(node, VALUE.tag);
  if (tag >= Tag.NUMBER && tag <= Tag.MAP) {
    const named = n.ptrAt(node, VALUE.payload + OWNED.alloc);
    if (complete(named)) return named;
  }
  return binding();
}

/** An object of an object kind: handed over, lent, or answered. */
class ObjectShape extends ShapeImpl {
  readonly name: string;
  override readonly cArg: string | undefined;
  override readonly answers: boolean;
  override readonly takes: boolean;

  constructor(
    readonly kind: KindImpl,
    readonly mode: 'take' | 'lend' | 'answer',
  ) {
    super();
    this.name = mode === 'answer' ? kind.name : `${mode === 'lend' ? 'lent' : 'taken'}(${kind.name})`;
    this.answers = mode === 'answer';
    this.takes = mode === 'take';
    this.cArg = mode === 'take' ? 'guatiao_object' : mode === 'lend' ? 'void *' : undefined;
  }

  #raw(value: unknown, scope: Scope | undefined, lent: boolean): RawObject {
    if (value instanceof RemoteImpl && value.isHandle) {
      if (value.kind !== this.kind) {
        throw new TypeError(`an object of ${value.kind.name} where ${this.kind.name} was asked for`);
      }
      return lent ? value.rawView() : value.giveUp();
    }
    if (typeof value !== 'object' || value === null) {
      throw new TypeError(`an object implementing ${this.kind.name}, not ${value === null ? 'null' : typeof value}`);
    }
    return serve(this.kind, value, scope);
  }

  override pass(value: unknown, scope: Scope): unknown {
    if (this.mode === 'lend') {
      const raw = this.#raw(value, scope, true);
      const at = scope.block(SIZE.object);
      writeRaw(at, raw);
      return at;
    }
    const raw = this.#raw(value, undefined, false);
    scope.given(() => destroyRaw(raw));
    return { table: raw.table, size: raw.size, ctx: raw.ctx };
  }

  override outSlot(scope: Scope): bigint {
    return scope.block(SIZE.object);
  }

  override answer(out: bigint): unknown {
    return this.kind.handle(readRaw(out));
  }

  override receive(arg: unknown, scope: Scope): unknown {
    if (this.mode === 'take') {
      const view = arg as { table: bigint | null; size: number | bigint; ctx: bigint | null };
      return this.kind.handle({ table: view.table ?? 0n, size: Number(view.size), ctx: view.ctx ?? 0n });
    }
    const ptr = (arg as bigint | null) ?? 0n;
    if (!ptr) throw new NullArgument('no object was lent');
    const raw = readRaw(ptr);
    return this.kind.remote(raw.table, raw.size, raw.ctx, scope);
  }

  override unused(received: unknown): void {
    if (received instanceof RemoteImpl && received.isHandle) received.close();
  }

  override write(out: bigint, result: unknown): void {
    writeRaw(out, this.#raw(result, undefined, false));
  }
}

interface RawObject {
  table: bigint;
  size: number;
  ctx: bigint;
}

function readRaw(at: bigint): RawObject {
  return { table: n.ptrAt(at, OBJECT.table), size: n.size(at, OBJECT.size), ctx: n.ptrAt(at, OBJECT.ctx) };
}

function writeRaw(at: bigint, raw: RawObject): void {
  n.setPtr(at, OBJECT.table, raw.table);
  n.setU64(at, OBJECT.size, BigInt(raw.size));
  n.setPtr(at, OBJECT.ctx, raw.ctx);
}

let destroyProto: unknown;

/** Calls an object's `destroy` slot, the first after its header. */
function destroyRaw(raw: RawObject): void {
  if (!raw.table || raw.size < SIZE.kindHeader + WORD) return;
  const slot = n.ptrAt(raw.table, SIZE.kindHeader);
  if (!slot) return;
  const k = n.koffi();
  destroyProto ??= k.proto('void', ['void *']);
  k.call(slot, destroyProto as never, raw.ctx || null);
}

// ---- t: the shapes ---------------------------------------------------------------

type Plain = unknown;

/** The shapes a slot carries. */
export const t = {
  bool: new Scalar('bool', 'bool', 'bool') as unknown as Shape<boolean, boolean, boolean, boolean>,
  i8: new Scalar('i8', 'int8_t', 'int', 8, true) as unknown as Shape<number, number, number, number>,
  i16: new Scalar('i16', 'int16_t', 'int', 16, true) as unknown as Shape<number, number, number, number>,
  i32: new Scalar('i32', 'int32_t', 'int', 32, true) as unknown as Shape<number, number, number, number>,
  i64: new Scalar('i64', 'int64_t', 'int', 64, true) as unknown as Shape<number | bigint, bigint, bigint, number | bigint>,
  u8: new Scalar('u8', 'uint8_t', 'int', 8) as unknown as Shape<number, number, number, number>,
  u16: new Scalar('u16', 'uint16_t', 'int', 16) as unknown as Shape<number, number, number, number>,
  u32: new Scalar('u32', 'uint32_t', 'int', 32) as unknown as Shape<number, number, number, number>,
  u64: new Scalar('u64', 'uint64_t', 'int', 64) as unknown as Shape<number | bigint, bigint, bigint, number | bigint>,
  usize: new Scalar('usize', 'size_t', 'int', 64) as unknown as Shape<number | bigint, bigint, bigint, number | bigint>,
  isize: new Scalar('isize', 'intptr_t', 'int', 64, true) as unknown as Shape<number | bigint, bigint, bigint, number | bigint>,
  f32: new Scalar('f32', 'float', 'float') as unknown as Shape<number, number, number, number>,
  f64: new Scalar('f64', 'double', 'float') as unknown as Shape<number, number, number, number>,
  /** `guatiao_str`: borrowed text. */
  str: new Str() as unknown as Shape<string, string, never, never>,
  /** `guatiao_bytes`: borrowed bytes. */
  bytes: new Bytes() as unknown as Shape<Uint8Array | ArrayBuffer | ArrayBufferView, Uint8Array, never, never>,
  /** `guatiao_bytes_mut`: the caller's buffer, written in place. */
  bytesMut: new BytesMut() as unknown as Shape<Uint8Array, Uint8Array, never, never>,
  /** Owned text as an answer (`guatiao_string *out`). */
  text: new Text() as unknown as Shape<never, never, string, string>,
  /** A value: borrowed as an argument (`const guatiao_value *`, null allowed), or an answer. */
  value: new Tree('value', undefined, 'borrow') as unknown as Shape<Ref | Plain | null, Ref | null, Value, Ref | Plain>,
  /** A map: borrowed as an argument (`const guatiao_map *`), or an answer. */
  map: new Tree('map', Tag.MAP, 'borrow') as unknown as Shape<Ref | MapAnswer, MapRef, Value, MapAnswer>,
  /** A list, as an answer; `t.taken(t.list)` and `t.lent(t.list)` as arguments. */
  list: new Tree('list', Tag.LIST, 'answer') as unknown as Shape<never, never, Value, ListAnswer>,
  /** An argument handed over with the call: the callee owns it on every path. */
  taken,
  /** An argument lent for the call (`&mut`): the callee may change it where it stands. */
  lent,
};

type TreeShape = typeof t.value | typeof t.map | typeof t.list;

function taken<K extends Kind<any, true>>(what: K): Shape<Implementation<K> | ObjectHandle<K>, ObjectHandle<K>, never, never>;
function taken(what: typeof t.map): Shape<Ref | MapAnswer, Value, never, never>;
function taken(what: typeof t.list): Shape<Ref | ListAnswer, Value, never, never>;
function taken(what: typeof t.value): Shape<Ref | Plain, Value, never, never>;
function taken(what: TreeShape | Kind<any, true>): Shape<any, any, any, any> {
  if (what instanceof Tree) return new Tree(`taken(${what.name})`, what.tag, 'take') as never;
  if (what instanceof KindImpl && what.isObject) return new ObjectShape(what, 'take') as never;
  throw new TypeError(`t.taken takes t.value, t.map, t.list or an object kind, not ${String(what)}`);
}

function lent<K extends Kind<any, true>>(what: K): Shape<Implementation<K> | ObjectHandle<K>, Remote<K>, never, never>;
function lent(what: typeof t.map): Shape<Ref, MapRef, never, never>;
function lent(what: typeof t.list): Shape<Ref, ListRef, never, never>;
function lent(what: typeof t.value): Shape<Ref, Ref, never, never>;
function lent(what: TreeShape | Kind<any, true>): Shape<any, any, any, any> {
  if (what instanceof Tree) return new Tree(`lent(${what.name})`, what.tag, 'lend') as never;
  if (what instanceof KindImpl && what.isObject) return new ObjectShape(what, 'lend') as never;
  throw new TypeError(`t.lent takes t.value, t.map, t.list or an object kind, not ${String(what)}`);
}

// ---- a kind ------------------------------------------------------------------------

/** One slot, resolved. @internal */
interface Slot {
  readonly name: string;
  readonly index: number;
  readonly offset: number;
  readonly args: readonly ShapeImpl[];
  readonly answer: ShapeImpl | undefined;
  readonly required: boolean;
  readonly fallible: boolean;
  /** The slot's C argument types, `ctx` first, for its koffi prototype. */
  readonly cTypes: readonly string[];
}

const protos = new WeakMap<Slot, unknown>();

/** The slot's koffi prototype, made on first use: describing a kind loads nothing. */
function protoOf(slot: Slot): unknown {
  let proto = protos.get(slot);
  if (proto === undefined) {
    proto = n.koffi().proto('uint32_t', [...slot.cTypes]);
    protos.set(slot, proto);
  }
  return proto;
}

/** Names a remote answers itself, so no method may be called them. */
const RESERVED = new Set(['has', 'close', 'closed', 'kind', 'ctx', 'size', 'table', 'then', 'constructor']);

let kindIds = 0;

/** @internal */
export class KindImpl implements Kind<Methods, boolean> {
  readonly id = ++kindIds;
  readonly name: string;
  readonly floorHash: number;
  readonly isObject: boolean;
  readonly methods: Methods;
  readonly slots: readonly Slot[];
  readonly tableSize: number;
  readonly floor: number;
  readonly destroyOffset: number | undefined;
  /** The callbacks a table this binding serves points at: one per slot, `destroy` first. */
  readonly shims: readonly bigint[];
  readonly destroyShim: bigint | undefined;
  readonly #remoteClass: new (raw: RawObject, scope?: Scope) => RemoteImpl;
  readonly #handleClass: new (raw: RawObject) => RemoteImpl;

  constructor(name: string, spec: { floorHash: number; object?: boolean; methods: Methods }) {
    if (typeof name !== 'string' || !name) throw new TypeError('a kind has a name');
    if (!Number.isInteger(spec.floorHash) || spec.floorHash < 0 || spec.floorHash > 0xffffffff) {
      throw new TypeError(`${name}: floorHash is the header's <table>_FLOOR_HASH, a 32-bit unsigned integer`);
    }
    this.name = name;
    this.floorHash = spec.floorHash;
    this.isObject = spec.object === true;
    this.methods = spec.methods;
    let offset = SIZE.kindHeader;
    if (this.isObject) {
      this.destroyOffset = offset;
      offset += WORD;
    }
    const slots: Slot[] = [];
    let floor = this.isObject ? offset : SIZE.kindHeader;
    for (const [methodName, method] of Object.entries(spec.methods)) {
      if (RESERVED.has(methodName) || methodName in Object.prototype) {
        throw new TypeError(`${name}.${methodName}: the name is a remote's own; name it otherwise, a slot is found by position`);
      }
      const args = (method.args ?? []).map((shape, i) => {
        if (!(shape instanceof ShapeImpl) || shape.cArg === undefined) {
          throw new TypeError(`${name}.${methodName}: argument ${i} (${String(shape)}) is not an argument shape`);
        }
        return shape;
      });
      let answer: ShapeImpl | undefined;
      if (method.answer !== undefined) {
        if (method.answer instanceof KindImpl) {
          if (!method.answer.isObject) throw new TypeError(`${name}.${methodName}: ${method.answer.name} is not an object kind`);
          answer = new ObjectShape(method.answer, 'answer');
        } else if (method.answer instanceof Tree && method.answer.mode === 'borrow') {
          answer = new Tree(method.answer.name, method.answer.tag, 'answer');
        } else if (method.answer instanceof ShapeImpl && method.answer.answers) {
          answer = method.answer;
        } else {
          throw new TypeError(`${name}.${methodName}: ${String(method.answer)} cannot be an answer`);
        }
      }
      const cTypes = ['void *', ...args.map((a) => a.cArg!), ...(answer ? ['void *'] : []), ...(method.fallible ? ['void *'] : [])];
      const slot: Slot = {
        name: methodName,
        index: slots.length,
        offset,
        args,
        answer,
        required: method.required === true,
        fallible: method.fallible === true,
        cTypes,
      };
      slots.push(slot);
      if (slot.required) floor = offset + WORD;
      offset += WORD;
    }
    this.slots = slots;
    this.tableSize = offset;
    this.floor = floor;
    // Registered now, once per slot: koffi cannot register a callback from
    // inside one, and a table is often first served from inside a call.
    refuseInsideACall(`the ${name} kind`);
    const k = n.koffi();
    this.shims = slots.map((slot) => k.register(shim(this, slot), k.pointer(protoOf(slot) as never)));
    this.destroyShim = this.isObject ? k.register(destroyShim(name), k.pointer(k.proto('void', ['void *']))) : undefined;

    const kind = this;
    class KindRemote extends RemoteImpl {
      constructor(raw: RawObject, scope?: Scope) {
        super(kind, raw, scope, false);
      }
    }
    class KindHandle extends RemoteImpl {
      constructor(raw: RawObject) {
        super(kind, raw, undefined, true);
      }
    }
    for (const slot of slots) {
      const call = function (this: RemoteImpl, ...args: unknown[]): unknown {
        return this.call(slot, args);
      };
      Object.defineProperty(call, 'name', { value: slot.name });
      for (const C of [KindRemote, KindHandle]) {
        Object.defineProperty(C.prototype, slot.name, { value: call, writable: false, enumerable: false, configurable: true });
      }
    }
    this.#remoteClass = KindRemote;
    this.#handleClass = KindHandle;
    Object.freeze(this);
  }

  bind(table: bigint, size: number, ctx: bigint | null): never {
    return this.remote(table, size, ctx ?? 0n) as never;
  }

  /** @internal */
  remote(table: bigint, size: number, ctx: bigint, scope?: Scope): RemoteImpl {
    return new this.#remoteClass({ table, size, ctx }, scope);
  }

  /** @internal */
  handle(raw: RawObject): RemoteImpl {
    if (!this.isObject) throw new TypeError(`${this.name} is not an object kind`);
    return new this.#handleClass(raw);
  }

  slot(name: string): Slot | undefined {
    return this.slots.find((s) => s.name === name);
  }

  toString(): string {
    return `kind ${this.name}`;
  }
}

/**
 * Describes a kind from its C header: `floorHash` as the header's
 * `<table>_FLOOR_HASH`, `object: true` for an object kind, and a method
 * per slot in the header's order. A description that is not one throws
 * `TypeError` naming the method.
 */
export function kind<const M extends Methods, const O extends boolean = false>(
  name: string,
  spec: { readonly floorHash: number; readonly object?: O; readonly methods: M },
): Kind<M, O> {
  return new KindImpl(name, spec as { floorHash: number; object?: boolean; methods: Methods }) as unknown as Kind<M, O>;
}

// ---- calling a table ----------------------------------------------------------------

const handles = new FinalizationRegistry<RawObject>((raw) => destroyRaw(raw));

/** @internal */
export class RemoteImpl {
  readonly kind: KindImpl;
  readonly table: bigint;
  readonly size: number;
  readonly ctx: bigint | null;
  readonly isHandle: boolean;
  readonly #scope: Scope | undefined;
  readonly #fns = new Map<number, (...args: unknown[]) => number>();
  #open = true;

  constructor(kind: KindImpl, raw: RawObject, scope: Scope | undefined, isHandle: boolean) {
    if (!raw.table) throw new FloorMismatch(`no ${kind.name} table`);
    if (raw.size < kind.floor) {
      throw new FloorMismatch(`the ${kind.name} table is ${raw.size} bytes, its required slots need ${kind.floor}`);
    }
    const hash = n.u32(raw.table, KIND_HEADER.floorHash);
    if (hash !== kind.floorHash) {
      throw new FloorMismatch(`the table's floor hash ${hash} is not the ${kind.name} kind's ${kind.floorHash}`);
    }
    this.kind = kind;
    this.table = raw.table;
    this.size = raw.size;
    this.ctx = raw.ctx || null;
    this.isHandle = isHandle;
    this.#scope = scope;
    if (isHandle) handles.register(this, { ...raw }, this);
  }

  #check(): void {
    this.#scope?.checkOpen();
    if (!this.#open) throw new TypeError('this object is closed, or was handed over');
  }

  #slotFn(slot: Slot): ((...args: unknown[]) => number) | undefined {
    if (slot.offset + WORD > this.size) return undefined;
    const ptr = n.ptrAt(this.table, slot.offset);
    if (!ptr) return undefined;
    let fn = this.#fns.get(slot.index);
    if (fn === undefined) {
      fn = n.koffi().decode(ptr, protoOf(slot) as never) as (...args: unknown[]) => number;
      this.#fns.set(slot.index, fn);
    }
    return fn;
  }

  has(name: string): boolean {
    this.#check();
    const slot = this.kind.slot(name);
    return slot !== undefined && this.#slotFn(slot) !== undefined;
  }

  /** @internal */
  call(slot: Slot, args: unknown[]): unknown {
    this.#check();
    if (args.length !== slot.args.length) {
      throw new TypeError(`${slot.name} takes ${slot.args.length} arguments, ${args.length} given`);
    }
    const fn = this.#slotFn(slot);
    if (fn === undefined) throw new Unsupported(`this ${this.kind.name} table has no ${slot.name}`);
    const scope = new Scope();
    try {
      const cArgs: unknown[] = new Array(args.length);
      // Handed-over arguments last: a conversion that fails before them
      // leaves the caller's trees where they were.
      slot.args.forEach((shape, i) => {
        if (!shape.takes) cArgs[i] = shape.pass(args[i], scope);
      });
      slot.args.forEach((shape, i) => {
        if (shape.takes) cArgs[i] = shape.pass(args[i], scope);
      });
      const out = slot.answer?.outSlot(scope);
      const err = slot.fallible ? scope.block(SIZE.providerError) : undefined;
      const extra = [...(out !== undefined ? [out] : []), ...(err !== undefined ? [err] : [])];
      const status = fn(this.ctx, ...cArgs, ...extra);
      scope.called();
      if (status !== Status.OK) {
        if (err !== undefined) n.check(status, err);
        throw errorFor(status, `${slot.name} failed, which its kind says it cannot`);
      }
      return slot.answer?.answer(out!, scope);
    } finally {
      scope.close();
    }
  }

  // -- an object this side owns

  get closed(): boolean {
    return !this.#open;
  }

  /** @internal */
  rawView(): RawObject {
    this.#check();
    return { table: this.table, size: this.size, ctx: this.ctx ?? 0n };
  }

  /** Gives the object up: from now on it is someone else's to destroy. @internal */
  giveUp(): RawObject {
    const raw = this.rawView();
    this.#open = false;
    handles.unregister(this);
    return raw;
  }

  /** Destroys an object this side owns; a second call does nothing. */
  close(): void {
    if (!this.isHandle || !this.#open) return;
    this.#open = false;
    handles.unregister(this);
    destroyRaw({ table: this.table, size: this.size, ctx: this.ctx ?? 0n });
  }

  [Symbol.dispose](): void {
    this.close();
  }

  toString(): string {
    return `${this.isHandle ? 'ObjectHandle' : 'Remote'} ${this.kind.name}`;
  }
}

// ---- serving a table over JavaScript objects -----------------------------------------------

/**
 * What a C caller can reach: each served object under a `ctx` that is a
 * number shaped like an address, never reused, so a call naming one that
 * was released answers `GUATIAO_ERR_GONE` rather than reaching another.
 */
const live = new Map<bigint, object>();
let nextCtx = 0x10000n;

/** A `ctx` naming `target` until `release(ctx)`. @internal */
export function hold(target: object): bigint {
  nextCtx += 16n;
  live.set(nextCtx, target);
  return nextCtx;
}

/**
 * Lets go of what `ctx` names; harmless for one already released. With
 * `dispose`, the object's `[Symbol.dispose]()` runs, when it has one: its
 * holder destroyed it. @internal
 */
export function release(ctx: bigint | null, dispose = false): void {
  if (!ctx) return;
  const target = live.get(BigInt(ctx));
  live.delete(BigInt(ctx));
  if (dispose && target !== undefined) {
    const disposeFn = (target as Partial<Disposable>)[Symbol.dispose];
    if (typeof disposeFn === 'function') disposeFn.call(target);
  }
}

class GoneError extends GuatiaoError {}

/** How deep this thread is inside calls a library made into this binding. */
let callDepth = 0;

/** @internal Counts a callback the binding is running, for `refuseInsideACall`. */
export function inCall<T>(body: () => T): T {
  callDepth += 1;
  try {
    return body();
  } finally {
    callDepth -= 1;
  }
}

/**
 * Throws when called from inside a call a library made into this binding:
 * koffi cannot register a callback there, and one registered there fails
 * when it is called. @internal
 */
export function refuseInsideACall(what: string): void {
  if (callDepth > 0) {
    throw new Error(`${what} cannot be made inside a call a library is making; make it at module level`);
  }
}

function held(ctx: unknown): object {
  if (!ctx) throw new GoneError(Status.ERR_NULL, 'the call names no object');
  const found = live.get(BigInt(ctx as bigint));
  if (found === undefined) throw new GoneError(Status.ERR_GONE, 'the object this call names is gone');
  return found;
}

/** How many objects a C caller can still reach: served and not yet destroyed or released. */
export function liveObjects(): number {
  return live.size;
}

/** The status and words a refusal crosses with. @internal */
export function statusOf(e: unknown): [number, string] {
  if (e instanceof GuatiaoError) return [e.status === Status.OK ? Status.ERR_INTERNAL : e.status, e.message];
  if (e instanceof Error) return [Status.ERR_INTERNAL, `${e.name}: ${e.message}`];
  return [Status.ERR_INTERNAL, String(e)];
}

function refuse(e: unknown, err: bigint | null | undefined, where: string): number {
  const [status, message] = statusOf(e);
  if (err === undefined) {
    process.emitWarning(`${where} cannot fail, and threw ${message}`, { type: 'GuatiaoWarning' });
  } else {
    writeErr(err, status, message);
  }
  return status;
}

/** @internal */
export { writeErr };

function shim(kind: KindImpl, slot: Slot): (...args: unknown[]) => number {
  const where = `${kind.name}.${slot.name}`;
  return (ctx: unknown, ...rest: unknown[]): number => inCall(() => serveCall(ctx, rest));

  function serveCall(ctx: unknown, rest: unknown[]): number {
    const scope = new Scope();
    const err = slot.fallible ? ((rest[rest.length - 1] as bigint | null) ?? null) : undefined;
    let status: number = Status.OK;
    const values: unknown[] = new Array(slot.args.length);
    const received: [ShapeImpl, unknown][] = [];
    let ran = false;
    try {
      // What was handed over is taken first, so a refusal below still
      // owns it, and frees it, rather than leaking it.
      slot.args.forEach((shape, i) => {
        if (shape.takes) received.push([shape, (values[i] = shape.receive(rest[i], scope))]);
      });
      const target = held(ctx) as Record<string, (...a: unknown[]) => unknown>;
      slot.args.forEach((shape, i) => {
        if (!shape.takes) values[i] = shape.receive(rest[i], scope);
      });
      ran = true;
      const result = target[slot.name]!(...values);
      if (result !== null && typeof result === 'object' && typeof (result as { then?: unknown }).then === 'function') {
        throw new Internal(`${where} answered a promise; a slot answers before it returns`);
      }
      if (slot.answer !== undefined) slot.answer.write(rest[slot.args.length] as bigint, result);
    } catch (e) {
      if (!ran) for (const [shape, value] of received) shape.unused(value);
      status = refuse(e, err, where);
    }
    try {
      scope.close();
    } catch (e) {
      if (status === Status.OK) status = refuse(e, err, where);
    }
    return status;
  }
}

function destroyShim(where: string): (ctx: unknown) => void {
  return (ctx: unknown): void => {
    try {
      inCall(() => release(ctx as bigint | null, true));
    } catch (e) {
      process.emitWarning(`${where}.destroy threw ${statusOf(e)[1]}`, { type: 'GuatiaoWarning' });
    }
  };
}

/** Tables built so far, by kind and the slots implemented: kept for the process. */
const tables = new Map<string, bigint>();

/** Whether `target` (an object, or a class's prototype) implements `slot`. @internal */
export function implementsSlot(target: object, slot: Slot): boolean {
  return typeof (target as Record<string, unknown>)[slot.name] === 'function';
}

/**
 * The table serving `kind` through `target`'s methods (an object, or a
 * class's prototype), built on first use and kept for the process: a slot
 * for each method it has, null for the rest, so a Rust caller runs the
 * kind's own default for an appended one. `TypeError` for a missing
 * required method. @internal
 */
export function tableFor(kind: KindImpl, target: object, what = 'the object'): bigint {
  const has = kind.slots.map((slot) => implementsSlot(target, slot));
  const missing = kind.slots.filter((slot, i) => slot.required && !has[i]).map((s) => s.name);
  if (missing.length) throw new TypeError(`${what} does not implement ${missing.join(', ')}, which the ${kind.name} kind requires`);
  const key = `${kind.id}:${has.map((h) => (h ? 1 : 0)).join('')}`;
  const found = tables.get(key);
  if (found !== undefined) return found;
  const table = n.alloc(kind.tableSize);
  n.setU32(table, KIND_HEADER.structSize, kind.tableSize);
  n.setU32(table, KIND_HEADER.floorHash, kind.floorHash);
  if (kind.destroyOffset !== undefined) n.setPtr(table, kind.destroyOffset, kind.destroyShim!);
  kind.slots.forEach((slot, i) => {
    if (has[i]) n.setPtr(table, slot.offset, kind.shims[i]!);
  });
  tables.set(key, table);
  return table;
}

/** `target` as a `guatiao_object` of `kind`: held until its holder destroys it, or, lent, until `scope` closes. */
function serve(kind: KindImpl, target: object, scope?: Scope): RawObject {
  const table = tableFor(kind, target);
  const ctx = hold(target);
  if (scope !== undefined) scope.after(() => release(ctx));
  return { table, size: kind.tableSize, ctx };
}

/** The `Kind` a value describes, or a `TypeError`. @internal */
export function kindOf(what: unknown): KindImpl {
  if (what instanceof KindImpl) return what;
  throw new TypeError(`${String(what)} is not a kind made by kind()`);
}
