// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Kinds as a peer's catalog describes them (guatiao's encodings, "A kind,
 * described"): each slot's arguments and answer, which type a call's
 * bytes.
 *
 * @module
 */

import { BadValue, NotFound } from './errors.js';

type Description = Record<string, unknown>;

function map(v: unknown, at: string): Description {
  if (typeof v === 'object' && v !== null && !Array.isArray(v) && !(v instanceof Uint8Array)) {
    return v as Description;
  }
  throw new BadValue(`${at} is not a map`);
}

function text(d: Description, key: string, at: string): string {
  const v = d[key];
  if (typeof v === 'string') return v;
  throw new BadValue(`${at}: \`${key}\` is not text`);
}

/** How an argument crosses: lent for the call, taken by the callee, or borrowed. */
export type Pass = 'borrowed' | 'taken' | 'lent';

/** An argument of a `RemoteSlot`. */
export class RemoteArg {
  private constructor(
    /** Its name in the description, or `''`. */
    readonly name: string,
    /**
     * `bool`, `i8` ... `u64`, `isize`, `usize`, `f32`, `f64`, `str`,
     * `bytes`, `bytes_mut`, `value`, `map`, `list` or `object`.
     */
    readonly type: string,
    /** The object kind, for an `object`. */
    readonly kind: string | undefined,
    readonly pass: Pass,
    /** A borrowed `value` that may be absent. */
    readonly nullable: boolean,
  ) {}

  /** @internal */
  static of(description: unknown, at: string): RemoteArg {
    const d = map(description, at);
    const pass = d['pass'];
    return new RemoteArg(
      typeof d['name'] === 'string' ? d['name'] : '',
      text(d, 'type', at),
      typeof d['kind'] === 'string' ? d['kind'] : undefined,
      pass === 'taken' || pass === 'lent' ? pass : 'borrowed',
      d['nullable'] === true,
    );
  }
}

/** What a `RemoteSlot` answers. */
export class RemoteAnswer {
  private constructor(
    /** A scalar's name, `text`, `value`, `map`, `list` or `object`. */
    readonly type: string,
    /** The object kind, for an `object`. */
    readonly kind: string | undefined,
  ) {}

  /** @internal */
  static of(description: unknown, at: string): RemoteAnswer {
    const d = map(description, at);
    return new RemoteAnswer(text(d, 'type', at), typeof d['kind'] === 'string' ? d['kind'] : undefined);
  }
}

/** One slot of a `RemoteKind`. */
export class RemoteSlot {
  private constructor(
    /** Its field in the table. */
    readonly name: string,
    /** Its index in the kind: what a CALL names. */
    readonly index: number,
    /** A slot every table fills, not null. */
    readonly required: boolean,
    /** It can refuse. */
    readonly fallible: boolean,
    /** Its arguments, in order. */
    readonly args: readonly RemoteArg[],
    /** Its answer; `undefined` when it answers nothing. */
    readonly answer: RemoteAnswer | undefined,
  ) {}

  /** @internal */
  static of(description: unknown, index: number, kind: string): RemoteSlot {
    const at = `kind \`${kind}\`, slot ${index}`;
    const d = map(description, at);
    const name = text(d, 'name', at);
    const listed = d['args'] ?? [];
    if (!Array.isArray(listed)) throw new BadValue(`${at}: \`args\` is not a list`);
    const args = listed.map((a: unknown, i) => RemoteArg.of(a, `${at}, argument ${i}`));
    const answer = d['answer'];
    return new RemoteSlot(
      name,
      index,
      d['required'] === true,
      d['fallible'] === true,
      Object.freeze(args),
      answer === undefined || answer === null ? undefined : RemoteAnswer.of(answer, `${at}, answer`),
    );
  }
}

/** A kind as a peer's catalog describes it. */
export class RemoteKind {
  private constructor(
    /** What a provider lists and a call is typed by. */
    readonly name: string,
    /** An object kind: its values are objects a call hands or lends. */
    readonly object: boolean,
    /** Every slot, in table order. */
    readonly slots: readonly RemoteSlot[],
    readonly doc: string | undefined,
  ) {}

  /**
   * Reads one kind's description, as a catalog's `kinds` holds it.
   * Throws `BadValue` for one that is not a kind.
   */
  static fromDescription(description: unknown): RemoteKind {
    const d = map(description, 'a kind description');
    const name = text(d, 'name', 'a kind');
    const listed = d['slots'];
    if (!Array.isArray(listed)) throw new BadValue(`kind \`${name}\`: \`slots\` is not a list`);
    const slots = listed.map((s: unknown, i) => RemoteSlot.of(s, i, name));
    const doc = d['doc'];
    return new RemoteKind(name, d['object'] === true, Object.freeze(slots), typeof doc === 'string' ? doc : undefined);
  }

  /** The slot called `name`; throws `NotFound` when the kind has none. */
  slot(name: string): RemoteSlot {
    const found = this.slots.find((s) => s.name === name);
    if (found === undefined) throw new NotFound(`the kind \`${this.name}\` has no slot \`${name}\``);
    return found;
  }

  toString(): string {
    return `RemoteKind(${this.name})`;
  }
}
