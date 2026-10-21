// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Dates, times, instants, spans and exact lengths of time.
 *
 * Each form is text in a value: a date `2026-10-21`, a time of day
 * `13:45:00.5`, a civil date and time `2026-10-21T13:45:00.5`, an instant
 * `2026-10-21T13:45:00.5Z` and a span `P1Y2M3W4DT5H6M7.5S`; an exact
 * length is the list `[seconds, nanoseconds]`. Every text is read and
 * written by the library's own functions, so this module accepts exactly
 * what any other reader does.
 *
 * `PlainDate`, `PlainTime`, `PlainDateTime` and `Span` keep every
 * nanosecond. An instant reads into a `Date`, which counts milliseconds:
 * what lies below one is dropped, toward the past.
 *
 * @module
 */

import { DATE, DATETIME, SIZE, SPAN, Status, TEXT_ERROR, TIME, TIMESTAMP, Tag, VALUE } from './abi.js';
import { BadValue, Unparsed, raiseForStatus } from './errors.js';
import * as n from './native.js';
import { withNodes } from './value.js';

/** A text form the library reads and writes. */
export type TextForm = 'date' | 'time' | 'datetime' | 'timestamp' | 'span';

type Field = readonly [offset: number, width: 'i32' | 'u8' | 'u32' | 'i64'];

const FIELDS: Readonly<Record<TextForm, readonly Field[]>> = {
  date: [
    [DATE.year, 'i32'],
    [DATE.month, 'u8'],
    [DATE.day, 'u8'],
  ],
  time: [
    [TIME.hour, 'u8'],
    [TIME.minute, 'u8'],
    [TIME.second, 'u8'],
    [TIME.nanosecond, 'u32'],
  ],
  datetime: [
    [DATETIME.date + DATE.year, 'i32'],
    [DATETIME.date + DATE.month, 'u8'],
    [DATETIME.date + DATE.day, 'u8'],
    [DATETIME.time + TIME.hour, 'u8'],
    [DATETIME.time + TIME.minute, 'u8'],
    [DATETIME.time + TIME.second, 'u8'],
    [DATETIME.time + TIME.nanosecond, 'u32'],
  ],
  timestamp: [
    [TIMESTAMP.seconds, 'i64'],
    [TIMESTAMP.nanoseconds, 'u32'],
  ],
  span: [
    [SPAN.years, 'i64'],
    [SPAN.months, 'i64'],
    [SPAN.weeks, 'i64'],
    [SPAN.days, 'i64'],
    [SPAN.hours, 'i64'],
    [SPAN.minutes, 'i64'],
    [SPAN.seconds, 'i64'],
    [SPAN.nanoseconds, 'i32'],
  ],
};

const SIZES: Readonly<Record<TextForm, number>> = {
  date: SIZE.date,
  time: SIZE.time,
  datetime: SIZE.datetime,
  timestamp: SIZE.timestamp,
  span: SIZE.span,
};

/** A form's fields, in the C struct's order: `bigint` for the 64-bit ones. */
export type Fields = (number | bigint)[];

function fieldsOf(form: TextForm): readonly Field[] {
  const found = FIELDS[form];
  if (found === undefined) throw new TypeError(`unknown form: ${String(form)}`);
  return found;
}

function readFields(form: TextForm, buf: Buffer): Fields {
  return fieldsOf(form).map(([at, width]) => {
    switch (width) {
      case 'u8':
        return buf.readUInt8(at);
      case 'u32':
        return buf.readUInt32LE(at);
      case 'i32':
        return buf.readInt32LE(at);
      case 'i64':
        return buf.readBigInt64LE(at);
    }
  });
}

const RANGES = {
  u8: [0n, 255n],
  u32: [0n, 4294967295n],
  i32: [-2147483648n, 2147483647n],
  i64: [-(2n ** 63n), 2n ** 63n - 1n],
} as const;

/** `fields` laid out as `form`'s struct; `BadValue` for one past its C width. */
function writeFields(form: TextForm, fields: readonly (number | bigint)[]): Buffer {
  const layout = fieldsOf(form);
  if (fields.length !== layout.length) throw new TypeError(`a ${form} has ${layout.length} fields`);
  const buf = Buffer.alloc(SIZES[form]);
  layout.forEach(([at, width], i) => {
    const raw = fields[i]!;
    if (typeof raw === 'number' && !Number.isInteger(raw)) throw new TypeError(`${raw} is not an integer`);
    const value = BigInt(raw);
    const [lo, hi] = RANGES[width];
    if (value < lo || value > hi) throw new BadValue(`the fields are not a ${form}`);
    if (width === 'u8') buf.writeUInt8(Number(value), at);
    else if (width === 'u32') buf.writeUInt32LE(Number(value), at);
    else if (width === 'i32') buf.writeInt32LE(Number(value), at);
    else buf.writeBigInt64LE(value, at);
  });
  return buf;
}

/** The text of the string value at `node`, which is freed. */
function takeText(node: bigint): string {
  try {
    return n.u32(node, VALUE.tag) === Tag.STRING ? n.stringText(node + BigInt(VALUE.payload)) : '';
  } finally {
    n.freeNode(node);
  }
}

/** Frees the text error's message and throws for a failing `status`. */
function finish(status: number, err: bigint, form: TextForm): void {
  const expected = takeText(err + BigInt(TEXT_ERROR.expected));
  if (status === Status.OK) return;
  if (status === Status.ERR_BAD_VALUE) {
    throw new Unparsed(expected || 'a valid text', { form, at: n.size(err, TEXT_ERROR.at) });
  }
  raiseForStatus(status, expected);
}

/** `text` read into `form`'s fields. `Unparsed` for text the grammar refuses. @internal */
export function parseFields(form: TextForm, text: string): Fields {
  const lib = n.core();
  fieldsOf(form);
  const out = Buffer.alloc(SIZES[form]);
  const err = n.alloc(SIZE.textError);
  try {
    const status = lib.fn[`guatiao_${form}_parse`]!(lib.alloc, n.str(text), out, err) as number;
    finish(status, err, form);
  } finally {
    n.release(err);
  }
  return readFields(form, out);
}

/** `fields` written as `form`'s canonical text. @internal */
export function formatFields(form: TextForm, fields: readonly (number | bigint)[]): string {
  const lib = n.core();
  const buf = writeFields(form, fields);
  const out = n.alloc(SIZE.value);
  try {
    const status = lib.fn[`guatiao_${form}_format`]!(lib.alloc, buf, out) as number;
    const text = takeText(out);
    if (status === Status.ERR_BAD_VALUE) throw new BadValue(`the fields are not a ${form}`);
    raiseForStatus(status);
    return text;
  } finally {
    n.release(out);
  }
}

/**
 * `text`, read as `form`, in its canonical spelling: an instant in UTC, a
 * fraction without trailing zeros, a span in ISO 8601. Exact to the
 * nanosecond. `Unparsed` for text the grammar refuses.
 */
export function normalize(form: TextForm, text: string): string {
  fieldsOf(form);
  const lib = n.core();
  const out = n.alloc(SIZE.value);
  const err = n.alloc(SIZE.textError);
  try {
    const status = lib.fn[`guatiao_${form}_normalize`]!(lib.alloc, n.str(text), out, err) as number;
    const result = takeText(out);
    finish(status, err, form);
    return result;
  } finally {
    n.release(out);
    n.release(err);
  }
}

// ---- dates and times ------------------------------------------------------

/** A day on the proleptic Gregorian calendar, years 0000 to 9999, no zone. */
export class PlainDate {
  readonly year: number;
  readonly month: number;
  readonly day: number;
  readonly #text: string;

  /** Checked by the library: `BadValue` for a day the month does not have. */
  constructor(year: number, month: number, day: number) {
    this.#text = formatFields('date', [year, month, day]);
    this.year = year;
    this.month = month;
    this.day = day;
  }

  /** `YYYY-MM-DD`. `Unparsed` for text the grammar refuses. */
  static parse(text: string): PlainDate {
    const [y, m, d] = parseFields('date', text) as number[];
    return new PlainDate(y!, m!, d!);
  }

  /**
   * This date moved by `span`: years and months first (a day past the
   * month's end becomes its last day), then weeks and days. `BadValue` for
   * a span with time units, or a result outside the years 0000 to 9999.
   */
  add(span: Span): PlainDate {
    const out = Buffer.alloc(SIZE.date);
    raiseForStatus(
      n.core().fn.guatiao_date_add_span!(writeFields('date', [this.year, this.month, this.day]), span.toStruct(), out),
    );
    const [y, m, d] = readFields('date', out) as number[];
    return new PlainDate(y!, m!, d!);
  }

  equals(other: PlainDate): boolean {
    return other instanceof PlainDate && this.#text === other.#text;
  }

  /** Negative, zero or positive, as `a` is before, on or after `b`. */
  static compare(a: PlainDate, b: PlainDate): number {
    return a.#text < b.#text ? -1 : a.#text > b.#text ? 1 : 0;
  }

  toString(): string {
    return this.#text;
  }

  toJSON(): string {
    return this.#text;
  }
}

/** A time of day with no offset, to the nanosecond. */
export class PlainTime {
  readonly hour: number;
  readonly minute: number;
  readonly second: number;
  readonly nanosecond: number;
  readonly #text: string;

  /** Checked by the library: `BadValue` for a time off the clock. */
  constructor(hour: number, minute: number, second = 0, nanosecond = 0) {
    this.#text = formatFields('time', [hour, minute, second, nanosecond]);
    this.hour = hour;
    this.minute = minute;
    this.second = second;
    this.nanosecond = nanosecond;
  }

  /** `HH:MM:SS[.fraction]`. */
  static parse(text: string): PlainTime {
    const [h, m, s, ns] = parseFields('time', text) as number[];
    return new PlainTime(h!, m!, s!, ns!);
  }

  /** @internal */
  fields(): number[] {
    return [this.hour, this.minute, this.second, this.nanosecond];
  }

  equals(other: PlainTime): boolean {
    return other instanceof PlainTime && this.#text === other.#text;
  }

  static compare(a: PlainTime, b: PlainTime): number {
    const x = a.fields();
    const y = b.fields();
    for (let i = 0; i < x.length; i++) if (x[i] !== y[i]) return x[i]! < y[i]! ? -1 : 1;
    return 0;
  }

  toString(): string {
    return this.#text;
  }

  toJSON(): string {
    return this.#text;
  }
}

/** A civil date and time: what a wall clock shows, no zone. */
export class PlainDateTime {
  readonly date: PlainDate;
  readonly time: PlainTime;
  readonly #text: string;

  constructor(date: PlainDate, time: PlainTime) {
    this.date = date;
    this.time = time;
    this.#text = `${date.toString()}T${time.toString()}`;
  }

  /** `YYYY-MM-DDTHH:MM:SS[.fraction]`. */
  static parse(text: string): PlainDateTime {
    const [y, mo, d, h, mi, s, ns] = parseFields('datetime', text) as number[];
    return new PlainDateTime(new PlainDate(y!, mo!, d!), new PlainTime(h!, mi!, s!, ns!));
  }

  /** As `PlainDate.add`, then the time units, carried across midnight. */
  add(span: Span): PlainDateTime {
    const out = Buffer.alloc(SIZE.datetime);
    raiseForStatus(n.core().fn.guatiao_datetime_add_span!(this.#struct(), span.toStruct(), out));
    const [y, mo, d, h, mi, s, ns] = readFields('datetime', out) as number[];
    return new PlainDateTime(new PlainDate(y!, mo!, d!), new PlainTime(h!, mi!, s!, ns!));
  }

  #struct(): Buffer {
    const { year, month, day } = this.date;
    return writeFields('datetime', [year, month, day, ...this.time.fields()]);
  }

  equals(other: PlainDateTime): boolean {
    return other instanceof PlainDateTime && this.#text === other.#text;
  }

  static compare(a: PlainDateTime, b: PlainDateTime): number {
    return PlainDate.compare(a.date, b.date) || PlainTime.compare(a.time, b.time);
  }

  toString(): string {
    return this.#text;
  }

  toJSON(): string {
    return this.#text;
  }
}

// ---- instants ----------------------------------------------------------------

/**
 * An RFC 3339 instant, with any offset, as a `Date`. The offset it was
 * written with is not kept, and what lies below a millisecond is dropped,
 * toward the past.
 */
export function parseTimestamp(text: string): Date {
  const [seconds, nanoseconds] = parseFields('timestamp', text) as [bigint, number];
  return new Date(Number(seconds) * 1000 + Math.floor(nanoseconds / 1e6));
}

/** `date` as an instant in UTC, `...Z`. An invalid `Date` is a `RangeError`. */
export function formatTimestamp(date: Date): string {
  const ms = date.getTime();
  if (!Number.isFinite(ms)) throw new RangeError('an invalid Date names no instant');
  const seconds = Math.floor(ms / 1000);
  return formatFields('timestamp', [BigInt(seconds), (ms - seconds * 1000) * 1e6]);
}

// ---- exact lengths -------------------------------------------------------------

/** An exact length of time, never negative: what `[seconds, nanoseconds]` holds. */
export interface Duration {
  readonly seconds: bigint;
  readonly nanoseconds: number;
}

/**
 * An exact length of time read by the library's own reader: the list
 * `[seconds, nanoseconds]`, a number of seconds, or the map `{secs,
 * nanos}`, as a `Ref` or plain data. `BadValue` for anything else.
 */
export function toDuration(value: unknown): Duration {
  return withNodes([value], ([node]) => {
    const secs = Buffer.alloc(8);
    const nanos = Buffer.alloc(4);
    const status = n.core().fn.guatiao_duration_read!(node, secs, nanos) as number;
    if (status === Status.ERR_BAD_VALUE) {
      throw new BadValue('not an exact length: [seconds, nanoseconds], a number of seconds, or {secs, nanos}');
    }
    raiseForStatus(status);
    return { seconds: secs.readBigUInt64LE(0), nanoseconds: nanos.readUInt32LE(0) };
  });
}

/** `duration` as `[seconds, nanoseconds]`, the list `new Value(...)` takes. */
export function fromDuration(duration: Duration): [bigint, number] {
  const seconds = BigInt(duration.seconds);
  const nanoseconds = duration.nanoseconds;
  if (seconds < 0n) throw new RangeError('an exact length of time is never negative');
  if (!Number.isInteger(nanoseconds) || nanoseconds < 0 || nanoseconds > 999_999_999) {
    throw new RangeError('nanoseconds are 0 to 999999999');
  }
  if (seconds > 2n ** 64n - 1n) throw new RangeError('past the largest exact length');
  return [seconds, nanoseconds];
}

// ---- spans ---------------------------------------------------------------------

/** The units of a span; each is optional, and all share one sign. */
export interface SpanFields {
  years?: number | bigint;
  months?: number | bigint;
  weeks?: number | bigint;
  days?: number | bigint;
  hours?: number | bigint;
  minutes?: number | bigint;
  seconds?: number | bigint;
  nanoseconds?: number;
}

const UNITS = ['years', 'months', 'weeks', 'days', 'hours', 'minutes', 'seconds'] as const;

/**
 * A length in calendar units, each counted on its own, one sign for the
 * whole: `new Span({ days: 1, hours: 2 })` is `P1DT2H`. Equal unit by unit,
 * so a day is not 24 hours. Immutable. Checked by the library: fields of
 * both signs, a field past a signed 64-bit integer, or nanoseconds of a
 * second or more are `BadValue`.
 */
export class Span {
  readonly years: bigint;
  readonly months: bigint;
  readonly weeks: bigint;
  readonly days: bigint;
  readonly hours: bigint;
  readonly minutes: bigint;
  readonly seconds: bigint;
  readonly nanoseconds: number;
  readonly #text: string;

  constructor(fields: SpanFields = {}) {
    const units = UNITS.map((unit) => {
      const raw = fields[unit] ?? 0n;
      if (typeof raw === 'number' && !Number.isSafeInteger(raw)) {
        throw new TypeError(`${unit} must be an integer that a number holds exactly, or a bigint`);
      }
      return BigInt(raw);
    });
    const nanoseconds = fields.nanoseconds ?? 0;
    if (!Number.isInteger(nanoseconds)) throw new TypeError('nanoseconds must be an integer');
    this.#text = formatFields('span', [...units, nanoseconds]);
    [this.years, this.months, this.weeks, this.days, this.hours, this.minutes, this.seconds] = units as [
      bigint,
      bigint,
      bigint,
      bigint,
      bigint,
      bigint,
      bigint,
    ];
    this.nanoseconds = nanoseconds;
  }

  /** An ISO 8601 duration (`P1DT2H`, `-P1D`) or the friendly form (`1h 30m`). */
  static parse(text: string): Span {
    const fields = parseFields('span', text);
    return new Span({
      years: fields[0]!,
      months: fields[1]!,
      weeks: fields[2]!,
      days: fields[3]!,
      hours: fields[4]!,
      minutes: fields[5]!,
      seconds: fields[6]!,
      nanoseconds: Number(fields[7]!),
    });
  }

  /** @internal */
  toStruct(): Buffer {
    return writeFields('span', [
      this.years,
      this.months,
      this.weeks,
      this.days,
      this.hours,
      this.minutes,
      this.seconds,
      this.nanoseconds,
    ]);
  }

  /** The same span with the other sign. */
  negated(): Span {
    return new Span({
      years: -this.years,
      months: -this.months,
      weeks: -this.weeks,
      days: -this.days,
      hours: -this.hours,
      minutes: -this.minutes,
      seconds: -this.seconds,
      nanoseconds: -this.nanoseconds,
    });
  }

  get isZero(): boolean {
    return this.#text === 'PT0S';
  }

  get isNegative(): boolean {
    return this.#text.startsWith('-');
  }

  equals(other: Span): boolean {
    return other instanceof Span && this.#text === other.#text;
  }

  /** The canonical ISO 8601 text. */
  toString(): string {
    return this.#text;
  }

  toJSON(): string {
    return this.#text;
  }
}
