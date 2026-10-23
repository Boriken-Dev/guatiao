// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A tree crosses between guatiao_serde and guatiao: built by one library,
// read and freed through the other, every block accounted for.

import { describe, expect, it } from 'vitest';

import { Value } from 'guatiao';

import * as serde from '../src/index.js';
import { ParseError } from '../src/index.js';
import { Counting } from './counting.js';

describe('a tree between serde and the core', () => {
  it('serde builds it, the core changes and frees it', () => {
    using counting = new Counting(serde.library().alloc);
    const value = serde.loads('{"a": [1, "two", {"b": null}]}', { alloc: counting.alloc });
    expect(counting.live).toBeGreaterThan(0);
    value.asMap().set('c', 'added through the core');
    expect(value.toJS()).toEqual({ a: [1, 'two', { b: null }], c: 'added through the core' });
    value.close();
    expect(counting.live).toBe(0);
  });

  it('the core builds it, serde writes it, the core frees it', () => {
    using counting = new Counting(serde.library().alloc);
    const value = new Value({ port: 5900, tags: ['x'] }, { alloc: counting.alloc });
    expect(serde.dumps(value)).toBe('{"port":5900,"tags":["x"]}');
    expect(serde.dumps(value.asMap().get('tags'))).toBe('["x"]');
    value.close();
    expect(counting.live).toBe(0);
  });

  it('leaves nothing after a document that does not parse', () => {
    using counting = new Counting(serde.library().alloc);
    expect(() => serde.loads('{"a": [1,, 2]}', { alloc: counting.alloc })).toThrow(ParseError);
    expect(counting.live).toBe(0);
  });

  it("is serde's own library, which carries the core too", () => {
    const lib = serde.library();
    // A GUATIAO_LIBRARY file that exports serde serves it; any other setting
    // leads to serde's own file.
    if (lib.path !== process.env.GUATIAO_LIBRARY) expect(lib.path).toMatch(/guatiao_serde\.(dll|so|dylib)$/);
  });
});
