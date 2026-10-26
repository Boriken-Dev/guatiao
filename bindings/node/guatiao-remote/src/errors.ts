// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A status a peer answered, as an error: the classes and codes of the
 * `guatiao` package, defined here so a page needs nothing of it.
 *
 * @module
 */

/** `guatiao_status` codes. */
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

/**
 * One failing status. `status` is the raw code and `message` the other
 * side's own words. Thrown with a message alone (`new BadValue('nobody
 * to greet')`), a class carries its own status: that is how a handler of
 * an object this side serves refuses the peer's call.
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
/** The peer, the link or an object is gone; a call past its timeout. */
export class Gone extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_GONE;
}
/** What was asked is not something the other side offers. */
export class Unsupported extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_UNSUPPORTED;
}
/** Not now: the peer runs as many calls as it allows. */
export class Busy extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_BUSY;
}
export class Internal extends GuatiaoError {
  static override readonly STATUS: number = Status.ERR_INTERNAL;
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
