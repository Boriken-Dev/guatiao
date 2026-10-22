// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The time module against the shared vectors every implementation reads,
// then the mappings to JavaScript's own types and what they lose.

import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { BadValue, Unparsed, Value, type Ref } from '../src/index.js';
import {
  PlainDate,
  PlainDateTime,
  PlainTime,
  Span,
  type TextForm,
  formatFields,
  formatTimestamp,
  fromDuration,
  normalize,
  parseFields,
  parseTimestamp,
  toDuration,
} from '../src/time.js';
import { REPO, readText } from './helpers.js';

// An integer past 2^53 is read from its own text, so a 20-digit number
// stays exact.
const exactly = (_key: string, value: unknown, context?: { source?: string }): unknown =>
  typeof value === 'number' && !Number.isSafeInteger(value) && context?.source !== undefined && /^-?\d+$/.test(context.source)
    ? BigInt(context.source)
    : value;
const vectors = new Value(
  JSON.parse(
    readText(path.join(REPO, 'crates', 'guatiao', 'tests', 'vectors', 'time.json')),
    exactly as (key: string, value: unknown) => unknown,
  ),
);
const cases = (name: string): Ref[] => [...vectors.asMap().get(name)!.asList()];
const field = (c: Ref, key: string): Ref | undefined => c.asMap().get(key);
const exact = (fields: readonly (number | bigint)[]): bigint[] => fields.map((x) => BigInt(x));

const FORMS: TextForm[] = ['date', 'time', 'datetime', 'timestamp', 'span'];

describe('the shared vectors', () => {
  it.each(FORMS)('reads and writes every %s', (form) => {
    const all = cases(form);
    expect(all.length).toBeGreaterThan(0);
    for (const c of all) {
      const text = field(c, 'text')!.toJS<string>();
      const refusedAt = field(c, 'refused_at');
      if (refusedAt !== undefined) {
        for (const read of [() => parseFields(form, text), () => normalize(form, text)]) {
          let caught: unknown;
          try {
            read();
          } catch (e) {
            caught = e;
          }
          expect(caught, text).toBeInstanceOf(Unparsed);
          const refusal = caught as Unparsed;
          expect(refusal.at, text).toBe(refusedAt.toJS<number>());
          expect(refusal.form).toBe(form);
          expect(refusal.expected).not.toBe('');
          if (text.length > 2) expect(refusal.message).not.toContain(text);
        }
      } else {
        const fields = parseFields(form, text);
        const expected = field(c, 'fields')!.toJS<(number | bigint)[]>({ numbers: 'bigint' });
        expect(exact(fields), text).toEqual(exact(expected));
        const canonical = field(c, 'canonical')!.toJS<string>();
        expect(formatFields(form, fields), text).toBe(canonical);
        expect(normalize(form, text), text).toBe(canonical);
      }
    }
  });

  it.each(['date', 'datetime'] as const)('applies every span to a %s', (kind) => {
    for (const c of cases(`${kind}_add_span`)) {
      const start = field(c, 'start')!.toJS<string>();
      const span = Span.parse(field(c, 'span')!.toJS<string>());
      const result = field(c, 'result')!.toJS<string | null>();
      const moved = (): string => (kind === 'date' ? PlainDate.parse(start) : PlainDateTime.parse(start)).add(span).toString();
      if (result === null) expect(moved, `${start} + ${span.toString()}`).toThrow(BadValue);
      else expect(moved(), `${start} + ${span.toString()}`).toBe(result);
    }
  });

  it('reads every exact length', () => {
    for (const c of cases('duration')) {
      const value = field(c, 'value')!;
      const expected = field(c, 'duration')!.toJS<[bigint, bigint] | null>({ numbers: 'bigint' });
      if (expected === null) {
        expect(() => toDuration(value), JSON.stringify(value.toJS({ numbers: 'text' }))).toThrow(BadValue);
      } else {
        const got = toDuration(value);
        expect([got.seconds, BigInt(got.nanoseconds)]).toEqual(expected);
      }
    }
  });
});

describe('the mappings', () => {
  it('holds a date, a time and a civil date and time to the nanosecond', () => {
    const date = PlainDate.parse('2024-02-29');
    expect([date.year, date.month, date.day]).toEqual([2024, 2, 29]);
    expect(new PlainDate(0, 1, 1).toString()).toBe('0000-01-01');
    expect(() => new PlainDate(2026, 2, 30)).toThrow(BadValue);
    expect(() => new PlainDate(2026, 300, 1)).toThrow(BadValue);
    const time = PlainTime.parse('12:00:00.000000999');
    expect(time.nanosecond).toBe(999);
    expect(new PlainTime(8, 5, 3, 120_000_000).toString()).toBe('08:05:03.12');
    expect(() => new PlainTime(24, 0)).toThrow(BadValue);
    const at = PlainDateTime.parse('2026-10-21T13:45:00.5');
    expect(at.toString()).toBe('2026-10-21T13:45:00.5');
    expect(at.equals(new PlainDateTime(new PlainDate(2026, 10, 21), new PlainTime(13, 45, 0, 500_000_000)))).toBe(true);
    expect(PlainDate.compare(new PlainDate(2026, 1, 2), new PlainDate(2025, 12, 31))).toBe(1);
    expect(JSON.stringify({ at })).toBe('{"at":"2026-10-21T13:45:00.5"}');
  });

  it('reads an instant into a Date, dropping below a millisecond toward the past', () => {
    expect(parseTimestamp('2026-10-21T19:15:00.5+05:30').toISOString()).toBe('2026-10-21T13:45:00.500Z');
    expect(parseTimestamp('1969-12-31T23:59:59.9999999Z').toISOString()).toBe('1969-12-31T23:59:59.999Z');
    expect(parseTimestamp('0000-01-01T00:00:00Z').getUTCFullYear()).toBe(0);
    expect(formatTimestamp(new Date(Date.UTC(2026, 9, 21, 13, 45)))).toBe('2026-10-21T13:45:00Z');
    expect(formatTimestamp(new Date(-1))).toBe('1969-12-31T23:59:59.999Z');
    expect(() => formatTimestamp(new Date(Number.NaN))).toThrow(RangeError);
  });

  it('holds an exact length as seconds and nanoseconds', () => {
    expect(toDuration([90, 500_000_000])).toEqual({ seconds: 90n, nanoseconds: 500_000_000 });
    expect(toDuration(1.5)).toEqual({ seconds: 1n, nanoseconds: 500_000_000 });
    expect(toDuration({ secs: 1, nanos: 999 })).toEqual({ seconds: 1n, nanoseconds: 999 });
    using v = new Value({ d: [3, 0] });
    expect(toDuration(v.asMap().get('d'))).toEqual({ seconds: 3n, nanoseconds: 0 });
    expect(fromDuration({ seconds: 90n, nanoseconds: 5 })).toEqual([90n, 5]);
    expect(() => fromDuration({ seconds: -1n, nanoseconds: 0 })).toThrow(RangeError);
    expect(() => toDuration('90')).toThrow(BadValue);
  });

  it('keeps a span as its own class backed by the library', () => {
    const span = Span.parse('1y 2mo 3w 4d 5h 6m 7s 500ms 1us 1ns');
    expect(span.toString()).toBe('P1Y2M3W4DT5H6M7.500001001S');
    expect(span.nanoseconds).toBe(500_001_001);
    expect(span.days).toBe(4n);
    expect(span.equals(Span.parse('P1Y2M3W4DT5H6M7.500001001S'))).toBe(true);
    expect(new Span({ days: 1 }).equals(new Span({ hours: 24 }))).toBe(false);
    expect(new Span({ days: -1, hours: -2 }).equals(Span.parse('-P1DT2H'))).toBe(true);
    expect(new Span({ days: 1 }).negated().toString()).toBe('-P1D');
    expect(new Span().isZero).toBe(true);
    expect(new Span({ days: -1 }).isNegative).toBe(true);
    expect(Span.parse('P9223372036854775807D').days).toBe(2n ** 63n - 1n);
    expect(() => new Span({ days: 1, hours: -1 })).toThrow(BadValue);
    expect(() => new Span({ nanoseconds: 1_000_000_000 })).toThrow(BadValue);
    expect(() => new Span({ days: 2n ** 63n })).toThrow(BadValue);
    expect(() => new Span({ days: 1.5 })).toThrow(TypeError);
    let caught: unknown;
    try {
      Span.parse('1h 1h');
    } catch (e) {
      caught = e;
    }
    expect((caught as Unparsed).at).toBe(4);
  });

  it('moves a date and a civil date and time by a span', () => {
    expect(new PlainDate(2026, 1, 31).add(Span.parse('P1M')).toString()).toBe('2026-02-28');
    expect(PlainDateTime.parse('2026-10-21T23:30:00').add(new Span({ hours: 1 })).toString()).toBe(
      '2026-10-22T00:30:00',
    );
    expect(() => new PlainDate(2026, 10, 21).add(new Span({ hours: 1 }))).toThrow(BadValue);
  });

  it('normalizes to the nanosecond, and refuses a form it does not know', () => {
    expect(normalize('timestamp', '2026-10-21T19:15:00.000000001+05:30')).toBe('2026-10-21T13:45:00.000000001Z');
    expect(() => normalize('week' as TextForm, 'x')).toThrow(TypeError);
  });
});
