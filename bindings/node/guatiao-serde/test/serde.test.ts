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

  it('parses straight to plain data as loads().toJS() reads it', () => {
    for (const format of ['json', 'toml', 'yaml'] as const) {
      // TOML holds no integer past 64 bits.
      const big = format === 'json' ? 2n ** 70n : 2n ** 62n;
      const data = { z: 3, a: [1.5, 'two', true, big], nested: { n: '1.10', 'ü': {} } };
      const text = serde.dumps(data, { format });
      for (const options of [{}, { numbers: 'text' as const }, { numbers: 'bigint' as const, maps: 'map' as const }]) {
        using value = serde.loads(text, { format });
        expect(serde.parse(text, { format, ...options }), `${format} ${JSON.stringify(options)}`).toStrictEqual(
          value.toJS(options),
        );
      }
    }
    expect(serde.parse('{"n": 1.10}', { numbers: 'text' })).toStrictEqual({ n: '1.10' });
  });

  it('renders plain data as dumps writes it', () => {
    using kept = new Value({ k: 'kept' });
    for (const format of ['json', 'toml', 'yaml'] as const) {
      const big = format === 'json' ? 2n ** 70n : 2n ** 62n;
      const data = { z: 3, a: [1.5, 'two', true, big], bytes: new Uint8Array([1, 2]), ref: kept, 'ü': '€' };
      for (const pretty of [false, true]) {
        const options = { format, pretty, how: serde.BYTES_BASE64 };
        expect(serde.render(data, options), `${format} ${pretty}`).toBe(serde.dumps(data, options));
      }
    }
    expect(serde.render([1, 2])).toBe('[1,2]');
  });

  it('refuses in parse and render what loads and dumps refuse', () => {
    for (const [text, format] of [
      ['{"a":\n 1,,}', 'json'],
      ['a = = 1', 'toml'],
      ['a: [1', 'yaml'],
    ] as const) {
      let fromLoads: unknown;
      let fromParse: unknown;
      try {
        serde.loads(text, { format });
      } catch (e) {
        fromLoads = e;
      }
      try {
        serde.parse(text, { format });
      } catch (e) {
        fromParse = e;
      }
      expect(fromParse).toBeInstanceOf(ParseError);
      const [a, b] = [fromLoads as ParseError, fromParse as ParseError];
      expect([b.format, b.line, b.column, b.reason]).toEqual([a.format, a.line, a.column, a.reason]);
    }
    expect(() => serde.render([1, 2], { format: 'toml' })).toThrow(WrongKind);
    expect(() => serde.render({ a: 1 }, { format: 'xml' as never })).toThrow(TypeError);
    expect(() => serde.parse('{}', { format: 'xml' as never })).toThrow(TypeError);
    expect(() => serde.render({ a: Number.POSITIVE_INFINITY })).toThrow(RangeError);
    expect(() => serde.render({ b: new Uint8Array([1]) }, { how: serde.BYTES_REFUSE })).toThrow(BadValue);
  });
});
