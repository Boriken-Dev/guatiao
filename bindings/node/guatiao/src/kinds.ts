// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A kind's function table, from JavaScript: calling one, and serving one.
 *
 * A kind that exists elsewhere is loaded from its description, the
 * document its crate commits beside its C header: `kindsOf(document)`
 * answers each kind by name, and `guatiao-kind` writes a module (`.js`
 * and `.d.ts`) exporting each as a value and, in TypeScript, an interface
 * of the same name, which a provider class implements.
 *
 * ```ts
 * import { Greeter } from './greeter_kind.js';   // written by guatiao-kind
 * export class Hello extends Provider implements Greeter {
 *   static override readonly kinds = [Greeter];
 *   greet(name: string) { return { greeting: `hello, ${name}` }; }
 * }
 * ```
 *
 * A kind defined here, with no header to come from, is a class with
 * static fields, or the primitive underneath both, `kind()`: a method
 * per slot in the table's order, the shapes from `t`.
 *
 * ```js
 * export class Echo extends Kind {
 *   static kind = 'echo';
 *   static floorHash = 1;
 *   static slots = { echo: slot(t.str).returns(t.text).required() };
 * }
 * ```
 *
 * @module
 */

import { ALLOC, KIND_HEADER, OBJECT, OWNED, PROVIDER_ERROR, SIZE, Status, Tag, VALUE, WORD } from './abi.js';
import { BadValue, FloorMismatch, GuatiaoError, Internal, NullArgument, Unsupported, errorFor } from './errors.js';
import * as n from './native.js';
import { ListRef, MapRef, Ref, Value, adopt, writeJS, type Owner } from './value.js';

export type {
  Borrowed,
  BytesMut,
  Lent,
  ObjectKind,
  Taken,
  ValueAnswer,
  f32,
  f64,
  i8,
  i16,
  i32,
  i64,
  isize,
  u8,
  u16,
  u32,
  u64,
  usize,
} from './markers.js';

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
  readonly answer?: Shape<any, any, any, any> | ObjectKindLike;
  /** A slot every table fills: one the kind was born with, which its floor hash covers. */
  readonly required?: boolean;
  /** A slot ending in `guatiao_provider_error *err`: it can refuse. */
  readonly fallible?: boolean;
  /** What the slot does, for the description. */
  readonly doc?: string;
  /** The arguments' names, for the description (default `arg0`, `arg1`, ...). */
  readonly argNames?: readonly string[];
}

export type Methods = { readonly [name: string]: MethodSpec };

/**
 * A kind: what `kind()` and `kindsOf()` make. Also the base of a kind
 * defined as a class, whose static fields describe it.
 */
export interface Kind<M extends Methods = Methods, O extends boolean = boolean> {
  readonly name: string;
  /** The `<table>_FLOOR_HASH` the header defines, as written there. */
  readonly floorHash: number;
  /** An object kind: `destroy` first, an object one holder owns. */
  readonly isObject: O;
  /** What the kind is, for the description. */
  readonly doc?: string;
  readonly methods: M;
  /** The table's size in bytes: the header, `destroy`, a word per slot. */
  readonly tableSize: number;
  /** The smallest table this kind accepts: up to its last required slot. */
  readonly floor: number;
  /** A caller's handle on a table of this kind, over `ctx`. */
  bind(table: bigint, size: number, ctx: bigint | null): Remote<Kind<M, O>>;
}

/**
 * The base of a kind defined in JavaScript as a class: its static fields
 * say what `kind()` would be told. `static kind` is its name (default: the
 * class name in snake_case), `static floorHash` its hash, `static object`
 * whether it is an object kind, and `static slots` a `slot()` per slot, in
 * table order. The class itself is the kind: list it in a provider's
 * `static kinds`, bind a table with it, pass it to `t.taken`/`t.lent`.
 * It is described when first used, which must not be inside a call a
 * library is making.
 */
// The class merges with the interface above: an instance would be a kind,
// though none is made; what matters is the constructor and its statics.
export abstract class Kind<M extends Methods = Methods, O extends boolean = boolean> {}

declare const implType: unique symbol;
declare const remoteType: unique symbol;

/**
 * A kind whose types are written out rather than inferred: what the
 * `.d.ts` that `guatiao-kind` writes declares, `I` the interface a
 * provider implements and `R` what a caller holds.
 */
export interface DescribedKind<I, R, O extends boolean = boolean> extends Kind<Methods, O> {
  /** For the type checker only: nothing holds it at run time. */
  readonly [implType]: I;
  /** For the type checker only. */
  readonly [remoteType]: R;
}

declare const slotSpec: unique symbol;

/**
 * One slot of a kind defined as a class, built by `slot(...args)` and
 * refined by `.returns(answer)`, `.required()` and `.fallible()`.
 */
export interface Slot<S extends MethodSpec = MethodSpec> {
  /** What the slot says, for the type checker and the description. */
  readonly [slotSpec]: S;
  /** What the slot answers: a shape, or an object kind. */
  returns<A extends Shape<any, any, any, any> | ObjectKindLike>(answer: A): Slot<Omit<S, 'answer'> & { readonly answer: A }>;
  /** A slot every table fills. */
  required(): Slot<Omit<S, 'required'> & { readonly required: true }>;
  /** A slot ending in `guatiao_provider_error *err`: it can refuse. */
  fallible(): Slot<Omit<S, 'fallible'> & { readonly fallible: true }>;
  /** What the slot does, for the description. */
  doc(text: string): Slot<S>;
  /** The arguments' names, in order, for the description. */
  named(...names: string[]): Slot<S>;
}

/** A class extending `Kind`, as a type: its static fields. */
export interface KindClass {
  readonly prototype: Kind;
  readonly floorHash: number;
  readonly slots: { readonly [name: string]: Slot | MethodSpec };
  readonly kind?: string;
  readonly object?: boolean;
  readonly doc?: string;
}

/** Anything naming a kind: a value `kind()` or `kindsOf()` made, or a class extending `Kind`. */
export type KindLike = Kind<any, any> | KindClass;
/**
 * A kind of objects one holder owns. A class says so with `static object`,
 * which plain JavaScript types as a `boolean`: the value is checked when
 * the class is described.
 */
export type ObjectKindLike = Kind<any, true> | (KindClass & { readonly object: boolean });
/** A kind a provider serves. */
export type ProviderKindLike = Kind<any, false> | KindClass;

type SpecOf<T> = T extends Slot<infer S> ? S : T extends MethodSpec ? T : never;

/** The slots of a kind, as `kind()` would be told them. */
export type MethodsOf<K> =
  K extends Kind<infer M, any> ? M : K extends { readonly slots: infer S } ? { [N in keyof S]: SpecOf<S[N]> } : never;

type CallArgs<S extends MethodSpec> = S['args'] extends infer A extends readonly unknown[]
  ? { -readonly [I in keyof A]: A[I] extends Shape<infer X, any, any, any> ? X : never }
  : [];
type ImplArgs<S extends MethodSpec> = S['args'] extends infer A extends readonly unknown[]
  ? { -readonly [I in keyof A]: A[I] extends Shape<any, infer X, any, any> ? X : never }
  : [];
type CallAnswer<A> =
  A extends ObjectKindLike ? ObjectHandle<A> : A extends Shape<any, any, infer X, any> ? X : void;
type ImplAnswer<A> =
  A extends ObjectKindLike ? Implementation<A> | ObjectHandle<A> : A extends Shape<any, any, any, infer X> ? X : void;
type CallMethod<S extends MethodSpec> = (...args: CallArgs<S>) => CallAnswer<S['answer']>;
type ImplMethod<S extends MethodSpec> = (...args: ImplArgs<S>) => ImplAnswer<S['answer']>;

type ImplOf<M> = M extends Methods
  ? { [N in keyof M as M[N]['required'] extends true ? N : never]: ImplMethod<M[N]> } & {
      [N in keyof M as M[N]['required'] extends true ? never : N]?: ImplMethod<M[N]>;
    }
  : never;

/**
 * What implements a kind: a provider class, or an object handed to a
 * table. Required methods are mandatory, the rest optional.
 */
export type Implementation<K> = K extends { readonly [implType]: infer I } ? I : ImplOf<MethodsOf<K>>;

/** A caller's handle on a table: a method per slot, and what it is. */
export interface RemoteBase<K> {
  /** The kind, described: for a class extending `Kind`, what its statics describe. */
  readonly kind: K extends Kind<any, any> ? K : Kind;
  readonly table: bigint;
  readonly size: number;
  readonly ctx: bigint | null;
  /** Whether the table carries the slot for `name`. */
  has(name: keyof MethodsOf<K> & string): boolean;
}

/** A caller's handle on a table of `K`. */
export type Remote<K> = K extends { readonly [remoteType]: infer R }
  ? R
  : RemoteBase<K> & (MethodsOf<K> extends infer M extends Methods ? { [N in keyof M]: CallMethod<M[N]> } : never);

/** An object of an object kind this side owns: `close()` destroys it, once. */
export type ObjectHandle<K> = Remote<K> & Disposable & { close(): void; readonly closed: boolean };

/** What a slot answering a map may return. @guatiaoType answer_map */
export type MapAnswer = MapRef | Value | { readonly [key: string]: unknown } | ReadonlyMap<string, unknown>;
/** What a slot answering a list may return. @guatiaoType answer_list */
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

  /** How a description spells this shape. */
  abstract describe(): TypeDescription;

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

  override describe(): TypeDescription {
    return { type: this.name };
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
  override describe(): TypeDescription {
    return { type: 'str' };
  }
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
  override describe(): TypeDescription {
    return { type: 'bytes' };
  }
  override pass(value: unknown): unknown {
    return n.bytesArg(bytesOf(value, 't.bytes'));
  }
  override receive(arg: unknown): unknown {
    const view = arg as StrView;
    return n.bytesAt(view.ptr ?? 0n, Number(view.len));
  }
}

/** An out-buffer: the caller's writable memory, for the call. */
class BytesMutShape extends ShapeImpl {
  readonly name = 'bytesMut';
  override readonly cArg = 'guatiao_bytes_mut';
  override describe(): TypeDescription {
    return { type: 'bytes_mut' };
  }
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
  override describe(): TypeDescription {
    return { type: 'text' };
  }
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

  override describe(): TypeDescription {
    const type = this.tag === Tag.MAP ? 'map' : this.tag === Tag.LIST ? 'list' : 'value';
    if (this.mode === 'answer') return { type };
    return { type, pass: this.mode === 'take' ? 'taken' : this.mode === 'lend' ? 'lent' : 'borrowed' };
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

  override describe(): TypeDescription {
    if (this.mode === 'answer') return { type: 'object', kind: this.kind.name };
    return { type: 'object', kind: this.kind.name, pass: this.mode === 'take' ? 'taken' : 'lent' };
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
  bytesMut: new BytesMutShape() as unknown as Shape<Uint8Array, Uint8Array, never, never>,
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

function taken<K extends ObjectKindLike>(what: K): Shape<Implementation<K> | ObjectHandle<K>, ObjectHandle<K>, never, never>;
function taken(what: typeof t.map): Shape<Ref | MapAnswer, Value, never, never>;
function taken(what: typeof t.list): Shape<Ref | ListAnswer, Value, never, never>;
function taken(what: typeof t.value): Shape<Ref | Plain, Value, never, never>;
function taken(what: TreeShape | ObjectKindLike): Shape<any, any, any, any> {
  if (what instanceof Tree) return new Tree(`taken(${what.name})`, what.tag, 'take') as never;
  const object = objectKind(what);
  if (object !== undefined) return new ObjectShape(object, 'take') as never;
  throw new TypeError(`t.taken takes t.value, t.map, t.list or an object kind, not ${String(what)}`);
}

function lent<K extends ObjectKindLike>(what: K): Shape<Implementation<K> | ObjectHandle<K>, Remote<K>, never, never>;
function lent(what: typeof t.map): Shape<Ref, MapRef, never, never>;
function lent(what: typeof t.list): Shape<Ref, ListRef, never, never>;
function lent(what: typeof t.value): Shape<Ref, Ref, never, never>;
function lent(what: TreeShape | ObjectKindLike): Shape<any, any, any, any> {
  if (what instanceof Tree) return new Tree(`lent(${what.name})`, what.tag, 'lend') as never;
  const object = objectKind(what);
  if (object !== undefined) return new ObjectShape(object, 'lend') as never;
  throw new TypeError(`t.lent takes t.value, t.map, t.list or an object kind, not ${String(what)}`);
}

/** `what` as an object kind, or undefined when it is not one. */
function objectKind(what: unknown): KindImpl | undefined {
  const found = what instanceof KindImpl ? what : isKindClass(what) ? asKind(what) : undefined;
  return found?.isObject ? found : undefined;
}

// ---- a kind ------------------------------------------------------------------------

/** One slot, resolved. @internal */
interface ResolvedSlot {
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

const protos = new WeakMap<ResolvedSlot, unknown>();

/** The slot's koffi prototype, made on first use: describing a kind loads nothing. */
function protoOf(slot: ResolvedSlot): unknown {
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
  readonly doc: string | undefined;
  readonly methods: Methods;
  readonly slots: readonly ResolvedSlot[];
  readonly tableSize: number;
  readonly floor: number;
  readonly destroyOffset: number | undefined;
  /** The callbacks a table this binding serves points at: one per slot, `destroy` first. */
  readonly shims: readonly bigint[];
  readonly destroyShim: bigint | undefined;
  readonly #remoteClass: new (raw: RawObject, scope?: Scope) => RemoteImpl;
  readonly #handleClass: new (raw: RawObject) => RemoteImpl;

  constructor(name: string, spec: { floorHash: number; object?: boolean; doc?: string; methods: Methods }) {
    if (typeof name !== 'string' || !name) throw new TypeError('a kind has a name');
    if (!Number.isInteger(spec.floorHash) || spec.floorHash < 0 || spec.floorHash > 0xffffffff) {
      throw new TypeError(`${name}: floorHash is the header's <table>_FLOOR_HASH, a 32-bit unsigned integer`);
    }
    this.name = name;
    this.floorHash = spec.floorHash;
    this.isObject = spec.object === true;
    this.doc = spec.doc;
    this.methods = spec.methods;
    let offset = SIZE.kindHeader;
    if (this.isObject) {
      this.destroyOffset = offset;
      offset += WORD;
    }
    const slots: ResolvedSlot[] = [];
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
      const names = method.argNames;
      if (names !== undefined && (names.length !== args.length || names.some((n) => typeof n !== 'string' || n === ''))) {
        throw new TypeError(`${name}.${methodName}: a name for each of its ${args.length} arguments, not ${JSON.stringify(names)}`);
      }
      let answer: ShapeImpl | undefined;
      if (method.answer !== undefined) {
        const answerKind =
          method.answer instanceof KindImpl ? method.answer : isKindClass(method.answer) ? asKind(method.answer) : undefined;
        if (answerKind !== undefined) {
          if (!answerKind.isObject) throw new TypeError(`${name}.${methodName}: ${answerKind.name} is not an object kind`);
          answer = new ObjectShape(answerKind, 'answer');
        } else if (method.answer instanceof Tree && method.answer.mode === 'borrow') {
          answer = new Tree(method.answer.name, method.answer.tag, 'answer');
        } else if (method.answer instanceof ShapeImpl && method.answer.answers) {
          answer = method.answer;
        } else {
          throw new TypeError(`${name}.${methodName}: ${String(method.answer)} cannot be an answer`);
        }
      }
      const cTypes = ['void *', ...args.map((a) => a.cArg!), ...(answer ? ['void *'] : []), ...(method.fallible ? ['void *'] : [])];
      const slot: ResolvedSlot = {
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

  slot(name: string): ResolvedSlot | undefined {
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

// ---- a kind defined as a class ----------------------------------------------------------

const SLOT_SPEC = Symbol('guatiao.slot');

interface SlotImpl {
  readonly [SLOT_SPEC]: MethodSpec;
}

function slotOf(spec: MethodSpec): Slot {
  const made = {
    [SLOT_SPEC]: Object.freeze({ ...spec }),
    returns: (answer: MethodSpec['answer']) => slotOf({ ...spec, answer }),
    required: () => slotOf({ ...spec, required: true }),
    fallible: () => slotOf({ ...spec, fallible: true }),
    doc: (text: string) => slotOf({ ...spec, doc: text }),
    named: (...names: string[]) => slotOf({ ...spec, argNames: names }),
  };
  return Object.freeze(made) as unknown as Slot;
}

/**
 * One slot of a kind defined as a class: its arguments after `ctx`, in
 * the table's order. `.returns(answer)`, `.required()` and `.fallible()`
 * say the rest.
 */
export function slot<const A extends readonly Shape<any, any, any, any>[]>(
  ...args: A
): Slot<{ readonly args: A; readonly required: false; readonly fallible: false }> {
  return slotOf({ args, required: false, fallible: false }) as never;
}

function isSlot(value: unknown): value is SlotImpl {
  return typeof value === 'object' && value !== null && SLOT_SPEC in value;
}

/** Whether `value` is a class extending `Kind`. @internal */
export function isKindClass(value: unknown): value is KindClass {
  return typeof value === 'function' && value.prototype instanceof Kind;
}

function snakeCase(name: string): string {
  return name.replace(/(?<=[a-z0-9])([A-Z])/g, '_$1').toLowerCase();
}

/** Kinds described from their classes, once each. */
const fromClasses = new WeakMap<object, KindImpl>();

function describeClass(cls: KindClass & { readonly name: string }): KindImpl {
  let found = fromClasses.get(cls);
  if (found !== undefined) return found;
  const name = cls.kind ?? snakeCase(cls.name);
  if (cls.slots === null || typeof cls.slots !== 'object') {
    throw new TypeError(`${cls.name}: a kind class says its slots in static slots`);
  }
  const methods: Record<string, MethodSpec> = {};
  for (const [method, given] of Object.entries(cls.slots)) {
    methods[method] = isSlot(given) ? given[SLOT_SPEC] : (given as MethodSpec);
  }
  found = new KindImpl(name, {
    floorHash: cls.floorHash,
    object: cls.object === true,
    ...(cls.doc === undefined ? {} : { doc: cls.doc }),
    methods,
  });
  fromClasses.set(cls, found);
  return found;
}

// ---- a kind loaded from its description ------------------------------------------------

/** One kind as its description writes it: see the encodings page of the docs. */
export interface KindDescription {
  readonly name: string;
  readonly floor_hash: number;
  readonly object?: boolean;
  readonly doc?: string;
  readonly slots: readonly {
    readonly name: string;
    readonly required?: boolean;
    readonly fallible?: boolean;
    readonly args?: readonly TypeDescription[];
    readonly answer?: TypeDescription;
    readonly doc?: string;
  }[];
}

/** An argument or answer as a description writes it. */
export interface TypeDescription {
  readonly name?: string;
  readonly type: string;
  readonly pass?: 'borrowed' | 'taken' | 'lent';
  readonly kind?: string;
  readonly nullable?: boolean;
}

/** A document of kinds: what a kind crate commits beside its header. */
export interface KindsDocument {
  readonly format: 1;
  readonly kinds: readonly KindDescription[];
}

/** A slot's name in JavaScript: `greet_into` is `greetInto`. */
export function methodName(slot: string): string {
  return slot.replace(/_+([a-z0-9])/g, (_m, c: string) => c.toUpperCase());
}

/** A method's slot name in a description: `greetInto` is `greet_into`, the inverse of `methodName`. */
export function slotName(method: string): string {
  return snakeCase(method);
}

const SCALARS = new Set(['bool', 'i8', 'i16', 'i32', 'i64', 'u8', 'u16', 'u32', 'u64', 'usize', 'isize', 'f32', 'f64']);

function shapeOf(
  given: TypeDescription,
  where: string,
  answer: boolean,
  kindNamed: (name: string) => KindImpl,
): Shape<any, any, any, any> | Kind<any, true> {
  const type = given?.type;
  const shapes = t as unknown as Record<string, Shape<any, any, any, any>>;
  if (typeof type !== 'string') throw new TypeError(`${where}: no type`);
  if (SCALARS.has(type)) return shapes[type]!;
  if (answer) {
    if (type === 'text' || type === 'value' || type === 'map' || type === 'list') return shapes[type]!;
    if (type === 'object') return objectNamed(given, where, kindNamed) as unknown as Kind<any, true>;
    throw new TypeError(`${where}: an answer cannot be ${type}`);
  }
  switch (type) {
    case 'str':
      return t.str;
    case 'bytes':
      return t.bytes;
    case 'bytes_mut':
      return t.bytesMut;
    case 'value':
    case 'map':
    case 'list': {
      const tree = shapes[type] as TreeShape;
      const pass = given.pass ?? 'borrowed';
      if (pass === 'taken') return taken(tree as typeof t.map);
      if (pass === 'lent') return lent(tree as typeof t.map);
      if (pass === 'borrowed' && type !== 'list') return tree;
      throw new TypeError(`${where}: a ${type} cannot be passed ${String(pass)}`);
    }
    case 'object': {
      const object = objectNamed(given, where, kindNamed) as unknown as Kind<any, true>;
      if (given.pass === 'taken') return taken(object);
      if (given.pass === 'lent') return lent(object);
      throw new TypeError(`${where}: an object is passed taken or lent, not ${String(given.pass)}`);
    }
    default:
      throw new TypeError(`${where}: ${type} is not a type an argument can have`);
  }
}

function objectNamed(given: TypeDescription, where: string, kindNamed: (name: string) => KindImpl): KindImpl {
  if (typeof given.kind !== 'string') throw new TypeError(`${where}: an object names its kind`);
  const found = kindNamed(given.kind);
  if (!found.isObject) throw new TypeError(`${where}: ${found.name} is not an object kind`);
  return found;
}

function fromDescription(description: KindDescription, kindNamed: (name: string) => KindImpl): KindImpl {
  const name = description?.name;
  if (typeof name !== 'string' || !name) throw new TypeError('a kind description has a name');
  if (!Array.isArray(description.slots)) throw new TypeError(`${name}: a kind description has slots`);
  const methods: Record<string, MethodSpec> = {};
  for (const slot of description.slots) {
    if (typeof slot?.name !== 'string') throw new TypeError(`${name}: a slot has a name`);
    const where = `${name}.${slot.name}`;
    const args = (slot.args ?? []).map((arg: TypeDescription, i: number) => shapeOf(arg, `${where} argument ${i}`, false, kindNamed) as Shape<any, any, any, any>);
    const spec: MethodSpec = {
      args,
      required: slot.required === true,
      fallible: slot.fallible === true,
      ...(slot.answer === undefined ? {} : { answer: shapeOf(slot.answer, `${where} answer`, true, kindNamed) }),
      ...(slot.doc === undefined ? {} : { doc: slot.doc }),
      argNames: (slot.args ?? []).map((a: TypeDescription, i: number) => a.name ?? `arg${i}`),
    };
    const method = methodName(slot.name);
    if (method in methods) throw new TypeError(`${where}: two slots are ${method} in JavaScript`);
    methods[method] = spec;
  }
  return new KindImpl(name, {
    floorHash: description.floor_hash,
    object: description.object === true,
    ...(description.doc === undefined ? {} : { doc: description.doc }),
    methods,
  });
}

function documentOf(given: KindsDocument | string): KindsDocument {
  const document = typeof given === 'string' ? (JSON.parse(given) as KindsDocument) : given;
  if (document === null || typeof document !== 'object' || document.format !== 1 || !Array.isArray(document.kinds)) {
    throw new TypeError('a kinds document is { format: 1, kinds: [...] }');
  }
  return document;
}

/**
 * Each kind a document describes (the object, or its JSON text), by its
 * name. An object kind a slot names is found in the same document, else
 * among `known`. Every kind is described now, which must not be inside a
 * call a library is making.
 */
export function kindsOf(
  document: KindsDocument | string,
  options: { readonly known?: Iterable<KindLike> } = {},
): Readonly<Record<string, Kind>> {
  const given = documentOf(document);
  const known = new Map<string, KindImpl>();
  for (const k of options.known ?? []) {
    const impl = asKind(k);
    known.set(impl.name, impl);
  }
  const descriptions = new Map(given.kinds.map((d) => [d?.name, d]));
  const made = new Map<string, KindImpl>();
  const making = new Set<string>();
  const kindNamed = (name: string): KindImpl => {
    const done = made.get(name);
    if (done !== undefined) return done;
    const description = descriptions.get(name);
    if (description === undefined) {
      const found = known.get(name);
      if (found === undefined) throw new TypeError(`no kind ${name} in the document or among the known kinds`);
      return found;
    }
    if (making.has(name)) throw new TypeError(`${name} refers to itself through its slots, which a description cannot load`);
    making.add(name);
    const impl = fromDescription(description, kindNamed);
    making.delete(name);
    made.set(name, impl);
    return impl;
  };
  const out: Record<string, Kind> = {};
  for (const description of given.kinds) out[description.name] = kindNamed(description.name) as unknown as Kind;
  return Object.freeze(out);
}

/** One kind's description as a kind; the object kinds it names come from `known`. */
export function kindOf(description: KindDescription, options: { readonly known?: Iterable<KindLike> } = {}): Kind {
  return kindsOf({ format: 1, kinds: [description] }, options)[description.name]!;
}

/**
 * The kinds in `kinds`, and the object kinds their slots reach, as the
 * document a kind crate commits beside its header: what `kindsOf` loads
 * and `guatiao-kind` and the Dart generator write code from. A kind made by
 * `kind()` or `kindsOf()` or defined as a class is described as it was
 * given; a method's slot name is its snake_case (`greetInto` is
 * `greet_into`), and a borrowed value is described as one that is not
 * null. A kind written as a TypeScript interface is `describeInterfaces`'s.
 */
export function describeKinds(kinds: Iterable<KindLike>): KindsDocument {
  const queue: KindImpl[] = [];
  const named = new Map<string, KindImpl>();
  const add = (impl: KindImpl): void => {
    const other = named.get(impl.name);
    if (other === undefined) {
      named.set(impl.name, impl);
      queue.push(impl);
    } else if (other !== impl) {
      throw new TypeError(`two different kinds are called ${impl.name}`);
    }
  };
  for (const k of kinds) add(asKind(k));
  const described: KindDescription[] = [];
  for (let i = 0; i < queue.length; i += 1) {
    const impl = queue[i]!;
    const slots = impl.slots.map((slot) => {
      const spec = impl.methods[slot.name]!;
      const name = slotName(slot.name);
      if (methodName(name) !== slot.name) {
        throw new TypeError(`${impl.name}.${slot.name}: its slot would be named ${name}, which JavaScript writes ${methodName(name)}; name the method that`);
      }
      for (const shape of [...slot.args, ...(slot.answer === undefined ? [] : [slot.answer])]) {
        if (shape instanceof ObjectShape) add(shape.kind);
      }
      return {
        name,
        required: slot.required,
        fallible: slot.fallible,
        args: slot.args.map((shape, j) => ({ name: spec.argNames?.[j] ?? `arg${j}`, ...shape.describe() })),
        ...(slot.answer === undefined ? {} : { answer: slot.answer.describe() }),
        ...(spec.doc === undefined ? {} : { doc: spec.doc }),
      };
    });
    described.push({
      name: impl.name,
      floor_hash: impl.floorHash,
      object: impl.isObject,
      ...(impl.doc === undefined ? {} : { doc: impl.doc }),
      slots,
    });
  }
  return { format: 1, kinds: described };
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

  #slotFn(slot: ResolvedSlot): ((...args: unknown[]) => number) | undefined {
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
  call(slot: ResolvedSlot, args: unknown[]): unknown {
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

function shim(kind: KindImpl, slot: ResolvedSlot): (...args: unknown[]) => number {
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
export function implementsSlot(target: object, slot: ResolvedSlot): boolean {
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

/** The kind `what` names, or a `TypeError`. @internal */
export function asKind(what: unknown): KindImpl {
  if (what instanceof KindImpl) return what;
  if (isKindClass(what)) return describeClass(what as KindClass & { readonly name: string });
  throw new TypeError(`${String(what)} is not a kind: make one with kind() or kindsOf(), or a class extending Kind`);
}
