// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * `guatiao_status` as JavaScript errors, and the binding's own.
 *
 * @module
 */

import { Status } from './abi.js';

/**
 * One `guatiao_status` failure. `status` is the raw code and `message` the
 * other side's own words, empty when it gave none.
 *
 * Thrown with a message alone (`new BadValue('nobody to greet')`), a class
 * carries its own status: that is how a provider written here refuses a
 * call with the status and words its caller reads.
 */
export class GuatiaoError extends Error {
  /** The status of an error thrown with a message alone. */
  static readonly STATUS: number = Status.ERR_INTERNAL;
  readonly status: number;

  constructor(message?: string);
  constructor(status: number, message?: string);
  constructor(first?: number | string, second = '') {
    super(typeof first === 'string' ? first : second);
    this.name = new.target.name;
    this.status = typeof first === 'number' ? first : (new.target as typeof GuatiaoError).STATUS;
  }
}

export class BadValue extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_BAD_VALUE;
}
export class AllocFailed extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_ALLOC;
}
export class WrongKind extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_WRONG_KIND;
}
export class NotFound extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_NOT_FOUND;
}
export class NullArgument extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_NULL;
}
export class Gone extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_GONE;
}
/** What was asked is not something the other side offers. */
export class Unsupported extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_UNSUPPORTED;
}
/** Not now: something is still in use. */
export class Busy extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_BUSY;
}
export class Internal extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_INTERNAL;
}

/**
 * A document that does not parse, and where it stopped. `format` is
 * `'json'`, `'toml'` or `'yaml'`; `line` and `column` count from 1 (the
 * column in characters) and are 0 when the parser did not say where;
 * `reason` says what it expected, without quoting the document.
 */
export class ParseError extends BadValue {
  readonly format: string;
  readonly line: number;
  readonly column: number;
  readonly reason: string;

  constructor(reason: string, where: { format: string; line: number; column: number }) {
    super(Status.ERR_BAD_VALUE, where.line ? `${reason} at line ${where.line}, column ${where.column}` : reason);
    this.reason = reason;
    this.format = where.format;
    this.line = where.line;
    this.column = where.column;
  }
}

/**
 * A date, time, instant or span whose text its grammar refused. `form` is
 * `'date'`, `'time'`, `'datetime'`, `'timestamp'` or `'span'`; `at` is the
 * byte offset, in the UTF-8 text, where reading stopped; `expected` says
 * what would have been accepted there. Neither quotes the text.
 */
export class Unparsed extends BadValue {
  readonly form: string;
  readonly at: number;
  readonly expected: string;

  constructor(expected: string, where: { form: string; at: number }) {
    super(Status.ERR_BAD_VALUE, `${expected} at byte ${where.at}`);
    this.expected = expected;
    this.form = where.form;
    this.at = where.at;
  }
}

/** No `basename` library was found by any route. */
export class LibraryNotFound extends Error {
  readonly basename: string;
  constructor(basename: string, looked: readonly string[]) {
    super(`could not find the ${basename} library: looked in ${looked.join('; ')}`);
    this.name = 'LibraryNotFound';
    this.basename = basename;
  }
}

/**
 * `symbol` is not exported by the library that was loaded: it was built
 * without the feature the symbol belongs to.
 */
export class MissingSymbol extends Error {
  readonly path: string;
  readonly symbol: string;
  readonly feature: string | undefined;
  constructor(path: string, symbol: string, feature?: string) {
    super(`${path} was not built with ${symbol}${feature ? ` (feature '${feature}')` : ''}`);
    this.name = 'MissingSymbol';
    this.path = path;
    this.symbol = symbol;
    this.feature = feature;
  }
}

/** A kind table that is not the one a kind describes, or too small for it. */
export class FloorMismatch extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FloorMismatch';
  }
}

const BY_STATUS: Readonly<Record<number, new (status: number, message?: string) => GuatiaoError>> = {
  [Status.ERR_BAD_VALUE]: BadValue,
  [Status.ERR_ALLOC]: AllocFailed,
  [Status.ERR_WRONG_KIND]: WrongKind,
  [Status.ERR_NOT_FOUND]: NotFound,
  [Status.ERR_NULL]: NullArgument,
  [Status.ERR_GONE]: Gone,
  [Status.ERR_UNSUPPORTED]: Unsupported,
  [Status.ERR_BUSY]: Busy,
  [Status.ERR_INTERNAL]: Internal,
};

/** The error a failing `status` is, carrying `message`. */
export function errorFor(status: number, message = ''): GuatiaoError {
  const Class = BY_STATUS[status] ?? GuatiaoError;
  return new Class(status, message);
}

/** Throws unless `status` is `GUATIAO_OK`. */
export function raiseForStatus(status: number, message = ''): void {
  if (status !== Status.OK) throw errorFor(status, message);
}
