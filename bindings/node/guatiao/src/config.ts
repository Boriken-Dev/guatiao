// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A configuration's schema, written as a value whose TypeScript type is
 * inferred from it, and the configuration built back from the plain data
 * a host checked against it. Imported as `s` from `guatiao/provide`.
 *
 * ```ts
 * const ShouterConfig = s.object({
 *   prefix: s.string({ minLength: 1 }),
 *   times: s.optional(s.integer({ minimum: 1 }), { default: 1 }),
 *   style: s.optional(s.literal('loud', 'soft')),
 * });
 * type ShouterConfig = s.Infer<typeof ShouterConfig>;
 * ```
 *
 * Each builder takes JSON Schema keywords as its last argument, added to
 * what it writes. An object refuses keys it does not declare.
 *
 * @module
 */

declare const output: unique symbol;

/** JSON Schema keywords, added to what a builder writes. */
export type Keywords = { readonly [keyword: string]: unknown };

/** A schema for values of type `T`, and how to build one from checked plain data. */
export interface Schema<T> {
  /** The JSON Schema 2020-12 document. */
  readonly json: Readonly<Record<string, unknown>>;
  /** Builds the configuration from plain data the host checked against `json`. */
  build(data: unknown): T;
  /** @internal */
  readonly [output]?: T;
}

/** A member of an object that may be left out, and what it then is. */
export interface Optional<T, D extends boolean> extends Schema<T> {
  readonly optional: true;
  readonly hasDefault: D;
  readonly fallback: unknown;
}

/** The type a schema builds. */
export type Infer<S> = S extends Optional<infer T, true> ? T : S extends Optional<infer T, false> ? T | undefined : S extends Schema<infer T> ? T : never;

function leaf<T>(json: Record<string, unknown>, build: (data: unknown) => T): Schema<T> {
  return Object.freeze({ json: Object.freeze(json), build });
}

/** Text. */
export function string(keywords: Keywords = {}): Schema<string> {
  return leaf({ type: 'string', ...keywords }, (data) => data as string);
}

/** An integer, as a `number`; one past what a `number` holds exactly is a `RangeError`. */
export function integer(keywords: Keywords = {}): Schema<number> {
  return leaf({ type: 'integer', ...keywords }, (data) => {
    const value = Number(data);
    if (!Number.isSafeInteger(value)) throw new RangeError(`${String(data)} does not fit a number exactly`);
    return value;
  });
}

/** A number. */
export function number(keywords: Keywords = {}): Schema<number> {
  return leaf({ type: 'number', ...keywords }, (data) => Number(data));
}

export function boolean(keywords: Keywords = {}): Schema<boolean> {
  return leaf({ type: 'boolean', ...keywords }, (data) => data as boolean);
}

/** One of the given values. */
export function literal<const V extends readonly (string | number | boolean | null)[]>(...values: V): Schema<V[number]> {
  return leaf({ enum: [...values] }, (data) => data as V[number]);
}

/** Any value, as plain data. */
export function any(keywords: Keywords = {}): Schema<unknown> {
  return leaf({ ...keywords }, (data) => data);
}

/** A list of `item`. */
export function array<T>(item: Schema<T>, keywords: Keywords = {}): Schema<T[]> {
  return leaf({ type: 'array', items: item.json, ...keywords }, (data) => (data as unknown[]).map((x) => item.build(x)));
}

/** A map of text keys to `value`. */
export function record<T>(value: Schema<T>, keywords: Keywords = {}): Schema<Record<string, T>> {
  return leaf({ type: 'object', additionalProperties: value.json, ...keywords }, (data) => {
    const out: Record<string, T> = {};
    for (const [key, item] of Object.entries(data as Record<string, unknown>)) {
      Object.defineProperty(out, key, { value: value.build(item), enumerable: true, writable: true, configurable: true });
    }
    return out;
  });
}

/** `inner`, or null. */
export function nullable<T>(inner: Schema<T>): Schema<T | null> {
  return leaf({ anyOf: [inner.json, { type: 'null' }] }, (data) => (data === null ? null : inner.build(data)));
}

/**
 * A member that may be left out: `undefined` then, or `options.default`,
 * which the schema also states.
 */
export function optional<T>(inner: Schema<T>): Optional<T, false>;
export function optional<T>(inner: Schema<T>, options: { default: T }): Optional<T, true>;
export function optional<T>(inner: Schema<T>, options?: { default: T }): Optional<T, boolean> {
  const hasDefault = options !== undefined && 'default' in options;
  const json = hasDefault ? { ...inner.json, default: options.default } : inner.json;
  return Object.freeze({
    json: Object.freeze(json),
    build: (data: unknown) => inner.build(data),
    optional: true as const,
    hasDefault,
    fallback: hasDefault ? options.default : undefined,
  });
}

type Fields = { readonly [name: string]: Schema<unknown> };
type Present<F extends Fields> = { [K in keyof F as F[K] extends Optional<any, false> ? never : K]: Infer<F[K]> };
type Missing<F extends Fields> = { [K in keyof F as F[K] extends Optional<any, false> ? K : never]?: Infer<F[K]> };
type Flat<T> = { [K in keyof T]: T[K] };

/** A record of named members; a member not marked `optional` is required. */
export function object<const F extends Fields>(fields: F, keywords: Keywords = {}): Schema<Flat<Present<F> & Missing<F>>> {
  const properties: Record<string, unknown> = {};
  const required: string[] = [];
  for (const [name, member] of Object.entries(fields)) {
    properties[name] = member.json;
    if (!(member as Partial<Optional<unknown, boolean>>).optional) required.push(name);
  }
  const json: Record<string, unknown> = { type: 'object', properties, additionalProperties: false, ...keywords };
  if (required.length) json.required = required;
  return leaf(json, (data) => {
    const given = data as Record<string, unknown>;
    const out: Record<string, unknown> = {};
    for (const [name, member] of Object.entries(fields)) {
      const optional = member as Partial<Optional<unknown, boolean>>;
      if (Object.prototype.hasOwnProperty.call(given, name) && given[name] !== undefined) {
        Object.defineProperty(out, name, { value: member.build(given[name]), enumerable: true, writable: true, configurable: true });
      } else if (optional.hasDefault) {
        out[name] = optional.fallback;
      }
    }
    return out as Flat<Present<F> & Missing<F>>;
  });
}

/** Whether `value` is a schema these builders made. */
export function isSchema(value: unknown): value is Schema<unknown> {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as Schema<unknown>).build === 'function' &&
    typeof (value as Schema<unknown>).json === 'object'
  );
}
