// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The types a kind written as a TypeScript interface is spelled with.
 *
 * TypeScript has one `number`; a slot's C type does not, so a scalar is
 * `u32`, `i64`, `f64` and so on. Each is its plain type plus a mark only
 * the compiler sees, so a plain `number` still fits a `u32` and an `i64`
 * never fits a `u32`. Nothing here exists at run time.
 *
 * `describeInterfaces` (`guatiao/describe`) reads an interface through
 * the declarations that carry a `@guatiaoType` tag, which is how it
 * tells these from a type of the same name that is someone else's.
 *
 * @module
 */

import type { ListRef, MapRef, Ref, Value } from './value.js';

declare const mark: unique symbol;

/** `T`, told apart from the same type with another mark; a plain `T` still fits. */
type Marked<T, Name extends string> = T & { readonly [mark]?: Name };

// ---- scalars -----------------------------------------------------------------------------

/** @guatiaoType i8 */
export type i8 = Marked<number, 'i8'>;
/** @guatiaoType i16 */
export type i16 = Marked<number, 'i16'>;
/** @guatiaoType i32 */
export type i32 = Marked<number, 'i32'>;
/** @guatiaoType i64 */
export type i64 = Marked<bigint, 'i64'>;
/** @guatiaoType isize */
export type isize = Marked<bigint, 'isize'>;
/** @guatiaoType u8 */
export type u8 = Marked<number, 'u8'>;
/** @guatiaoType u16 */
export type u16 = Marked<number, 'u16'>;
/** @guatiaoType u32 */
export type u32 = Marked<number, 'u32'>;
/** @guatiaoType u64 */
export type u64 = Marked<bigint, 'u64'>;
/** @guatiaoType usize */
export type usize = Marked<bigint, 'usize'>;
/** @guatiaoType f32 */
export type f32 = Marked<number, 'f32'>;
/** @guatiaoType f64 */
export type f64 = Marked<number, 'f64'>;

/** The caller's bytes, written in place (`bytes_mut`): a plain `Uint8Array` is read-only `bytes`. @guatiaoType bytes_mut */
export type BytesMut = Marked<Uint8Array, 'bytes_mut'>;

// ---- objects and how an argument crosses ---------------------------------------------------------

/**
 * What an interface extends to be an object kind: its table has `destroy`
 * first, and one holder owns an object of it. A provider never serves one.
 *
 * @guatiaoType object_kind
 */
export interface ObjectKind {
  readonly [mark]?: 'object';
}

/** What `Taken<T>` and `Lent<T>` may wrap: a tree, or an object kind. */
export type Passable = Ref | ObjectKind;

/**
 * A tree read for the call, which the caller keeps (`const` in C): `Ref`
 * for a value, `MapRef` for a map. A bare `Ref | MapRef` argument means
 * the same.
 *
 * @guatiaoType borrowed
 */
export type Borrowed<T extends Ref> = T;

/**
 * An argument handed over: the callee owns it from the call on. A tree
 * arrives as the `Value` that owns it, an object as a handle whose
 * `close()` destroys it.
 *
 * @guatiaoType taken
 */
export type Taken<T extends Passable> = T extends ObjectKind
  ? T & Disposable & { close(): void } & { readonly [mark]?: 'taken' }
  : Value & { readonly [mark]?: T extends ListRef ? 'taken list' : T extends MapRef ? 'taken map' : 'taken value' };

/**
 * An argument lent for the call alone: the callee may change a tree where
 * it stands (`&mut`), never free or keep it.
 *
 * @guatiaoType lent
 */
export type Lent<T extends Passable> = T & { readonly [mark]?: 'lent' };

// ---- answers -----------------------------------------------------------------------------------

/** What a slot answering a value may return (`value`). @guatiaoType answer_value */
export type ValueAnswer = unknown;
