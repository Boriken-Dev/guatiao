// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao_serde against a real --all-features build.

import { describe, expect, it } from 'vitest';

import { BadValue, Value, WrongKind } from 'guatiao';

import * as serde from '../src/index.js';
import { ParseError } from '../src/index.js';

describe('serde', () => {
  it('round-trips JSON with key order and integers kept', () => {
    using doc = new Value({ z: 3, a: 1, list: [1, 'two', null, true] });
    using parsed = serde.loads(serde.dumps(doc));
    const m = parsed.asMap();
    expect(m.keys()).toEqual(['z', 'a', 'list']);
    expect(parsed.toJS()).toEqual({ z: 3, a: 1, list: [1, 'two', null, true] });
  });

  it('keeps the exact text of a number', () => {
    using doc = new Value({ n: Value.number('1.10'), big: 2n ** 70n });
    using parsed = serde.loads(serde.dumps(doc));
    expect(parsed.asMap().get('n')!.toJS({ numbers: 'text' })).toBe('1.10');
    expect(parsed.asMap().get('big')!.toJS()).toBe(2n ** 70n);
  });

  it('indents for a person when asked', () => {
    using doc = new Value({ a: 1 });
    expect(serde.dumps(doc, { pretty: true }).length).toBeGreaterThan(serde.dumps(doc).length);
  });

  it('round-trips TOML and YAML', () => {
    using doc = new Value({ a: 1, b: 'x' });
    for (const format of ['toml', 'yaml'] as const) {
      using parsed = serde.loads(serde.dumps(doc, { format }), { format });
      expect(parsed.toJS()).toEqual({ a: 1, b: 'x' });
    }
  });

  it('refuses a TOML document that is not a map, and a format it does not know', () => {
    using doc = new Value([1, 2]);
    expect(() => serde.dumps(doc, { format: 'toml' })).toThrow(WrongKind);
    expect(() => serde.dumps(doc, { format: 'xml' as never })).toThrow(TypeError);
  });

  it('says where a document stopped parsing', () => {
    let caught: unknown;
    try {
      serde.loads('{"a":\n 1,,}');
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(ParseError);
    expect(caught).toBeInstanceOf(BadValue);
    const error = caught as ParseError;
    expect([error.format, error.line, error.column]).toEqual(['json', 2, 4]);
    expect(error.reason).not.toBe('');
    expect(error.message).toContain('line 2, column 4');
    expect(() => serde.loads('a = = 1', { format: 'toml' })).toThrow(
      expect.objectContaining({ format: 'toml', line: 1, column: 5 }) as unknown as Error,
    );
    expect(() => serde.loads('a: [1', { format: 'yaml' })).toThrow(
      expect.objectContaining({ format: 'yaml', line: 1 }) as unknown as Error,
    );
  });

  it('dumps plain data as it is', () => {
    expect(serde.dumps({ a: [1, true] })).toBe('{"a":[1,true]}');
  });
});
