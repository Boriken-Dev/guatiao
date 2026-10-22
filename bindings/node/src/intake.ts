// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Naming a place inside a value, whether a form fits a schema, its layout,
 * field visibility, and the flat `key -> text` projection, over
 * `guatiao_intake`, which is loaded on the first call.
 *
 * Every `schema`, `form`, `value` and `values` argument is a `Ref` (a
 * `Value` is one) or plain data, converted for the call.
 *
 * @module
 */

import { ENTRY, OWNED, SIZE, Status, Tag, VALUE } from './abi.js';
import { raiseForStatus } from './errors.js';
import * as n from './native.js';
import { Borrowed, Ref, Value, adopt, withNodes } from './value.js';

/**
 * The key of a provider's `meta` holding its form document: read it with
 * `Registry.providerMeta`, then `check` it against `providerConfig`.
 */
export const providerFormKey = 'form';

function answer(call: (alloc: bigint, out: bigint) => number, read: (status: number, out: Value) => unknown): unknown {
  const alloc = n.intake().alloc;
  const out = n.alloc(SIZE.value);
  let status: number;
  try {
    status = call(alloc, out);
  } catch (e) {
    n.freeNode(out);
    n.release(out);
    throw e;
  }
  return read(status, adopt(out, alloc));
}

function plain(status: number, out: Value): unknown {
  try {
    raiseForStatus(status);
    return out.toJS();
  } finally {
    out.close();
  }
}

/**
 * `null` when `form` fits `schema`; otherwise the error map (`kind`, `at`,
 * and whichever of `path`, `id`, `field`, `expected`, `message` apply).
 */
export function check(schema: unknown, form: unknown): Record<string, unknown> | null {
  return withNodes([schema, form], ([s, f]) =>
    answer(
      (alloc, out) => n.intake().fn.guatiao_intake_check!(s, f, alloc, out),
      (status, out) => {
        try {
          if (status === Status.OK) return null;
          if (status !== Status.ERR_BAD_VALUE) raiseForStatus(status);
          return out.toJS();
        } finally {
          out.close();
        }
      },
    ),
  ) as Record<string, unknown> | null;
}

/** The schema's fields grouped into sections and put in order. */
export function layout(schema: unknown, form: unknown): { section: Record<string, unknown> | null; fields: string[] }[] {
  return withNodes([schema, form], ([s, f]) =>
    answer((alloc, out) => n.intake().fn.guatiao_intake_layout!(s, f, alloc, out), plain),
  ) as { section: Record<string, unknown> | null; fields: string[] }[];
}

/** Whether the field under `key` is shown, given the `values` entered so far. */
export function isVisible(schema: unknown, form: unknown, key: string, values: unknown): boolean {
  return withNodes([schema, form, values], ([s, f, v]) => {
    const out = Buffer.alloc(1);
    raiseForStatus(n.intake().fn.guatiao_intake_is_visible!(s, f, n.str(key), v, out));
    return out[0] !== 0;
  });
}

function borrowedAt(source: Ref, find: (node: bigint) => bigint | null): Ref | undefined {
  const again = (): bigint | null => {
    const found = find(source.requireNode());
    return found === null || found === 0n ? null : found;
  };
  if (again() === null) return undefined;
  return new Ref(new Borrowed(again, source.owner), []);
}

/**
 * The value at `path` (`agent[1].name[home].host`: a dot is a member, a
 * bracket a list position or a map key, decided by what it applies to),
 * or `undefined` for a path that names nothing or is no path.
 *
 * What comes back **borrows** from `value` and is read-only: it is found
 * again on every access, throws once `value` is closed, and sees `value`
 * as it is now. `.clone()` copies it out.
 */
export function at(value: Ref, path: string): Ref | undefined {
  const text = n.str(path);
  return borrowedAt(value, (node) => n.intake().fn.guatiao_intake_path_get!(node, text) as bigint | null);
}

/**
 * The form `schema` implies when nobody wrote one: a section per distinct
 * `x-section` its fields name, ids only, and each member's own form. A
 * new `Value` the caller closes.
 */
export function forSchema(schema: unknown): Value {
  return withNodes([schema], ([s]) =>
    answer(
      (alloc, out) => n.intake().fn.guatiao_intake_for_schema!(s, alloc, out),
      (status, out) => {
        try {
          raiseForStatus(status);
        } catch (e) {
          out.close();
          throw e;
        }
        return out;
      },
    ),
  ) as Value;
}

/**
 * The form to show the field at `path` with: the one `form` assigns, or
 * the one its schema implies. `undefined` when the path names no field, or
 * one with no members and no form. A `Value` the caller closes.
 */
export function formFor(form: unknown, schema: unknown, path: string): Value | undefined {
  return withNodes([form, schema], ([f, s]) =>
    answer(
      (alloc, out) => n.intake().fn.guatiao_intake_form_for!(f, s, n.str(path), alloc, out),
      (status, out) => {
        try {
          raiseForStatus(status);
        } catch (e) {
          out.close();
          throw e;
        }
        if (out.isAbsent()) {
          out.close();
          return undefined;
        }
        return out;
      },
    ),
  ) as Value | undefined;
}

/**
 * The schema field governing the flat `key`, or `undefined`: borrowed from
 * `schema`, read-only, found again on every access.
 */
export function resolve(schema: Ref, key: string): Ref | undefined {
  const text = n.str(key);
  return borrowedAt(schema, (node) => n.intake().fn.guatiao_intake_resolve!(node, text) as bigint | null);
}

/** The flat keys the field `key` projects onto. `WrongKind` when `key` names no field. */
export function flatKeys(schema: unknown, key: string): string[] {
  return withNodes([schema], ([s]) =>
    answer((alloc, out) => n.intake().fn.guatiao_intake_flat_keys!(s, n.str(key), alloc, out), plain),
  ) as string[];
}

/**
 * `value` as the flat store of `key -> text` the field `key` keeps: one
 * entry per scalar leaf. `WrongKind` when `key` names no field or the
 * value's shape contradicts it.
 */
export function flatten(schema: unknown, key: string, value: unknown): Record<string, string> {
  return withNodes([schema, value], ([s, v]) =>
    answer((alloc, out) => n.intake().fn.guatiao_intake_flatten!(s, n.str(key), v, alloc, out), plain),
  ) as Record<string, string>;
}

function isPlain(value: object): boolean {
  const proto = Object.getPrototypeOf(value) as unknown;
  return proto === Object.prototype || proto === null;
}

function isTextStore(node: bigint): boolean {
  if (n.u32(node, VALUE.tag) !== Tag.MAP) return false;
  const at = node + BigInt(VALUE.payload);
  const len = n.size(at, OWNED.len);
  const entries = n.ptrAt(at, OWNED.ptr);
  for (let i = 0; i < len; i++) {
    if (n.u32(entries + BigInt(i * SIZE.entry + ENTRY.value), VALUE.tag) !== Tag.STRING) return false;
  }
  return true;
}

/**
 * The value the flat store holds for the field `key`, the reverse of
 * `flatten`; a `Value` the caller closes. `undefined` when `key` names no
 * field, nothing is stored under it, or the stored shape cannot be its
 * kind. `store` maps strings to strings (a plain object, a `Map`, or a
 * `Ref` onto such a map); anything else is a `TypeError`.
 */
export function unflatten(schema: unknown, key: string, store: unknown): Value | undefined {
  if (store instanceof Ref) {
    if (!isTextStore(store.requireNode())) throw new TypeError('a flat store maps strings to strings');
  } else {
    const entries =
      store instanceof Map
        ? [...(store as Map<unknown, unknown>).entries()]
        : typeof store === 'object' && store !== null && !Array.isArray(store) && isPlain(store)
          ? Object.entries(store)
          : undefined;
    if (entries === undefined || !entries.every(([k, v]) => typeof k === 'string' && typeof v === 'string')) {
      throw new TypeError('a flat store maps strings to strings');
    }
  }
  return withNodes([schema, store], ([s, st]) =>
    answer(
      (alloc, out) => n.intake().fn.guatiao_intake_unflatten!(s, n.str(key), st, alloc, out),
      (status, out) => {
        if (status === Status.ERR_WRONG_KIND) {
          out.close();
          return undefined;
        }
        try {
          raiseForStatus(status);
        } catch (e) {
          out.close();
          throw e;
        }
        return out;
      },
    ),
  ) as Value | undefined;
}
