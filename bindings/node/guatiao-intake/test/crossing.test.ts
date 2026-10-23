// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A tree crosses between guatiao_intake and guatiao: built by one library,
// read and freed through the other, every block accounted for; and a
// provider's form, read through the core's registry, checked here.

import { describe, expect, it } from 'vitest';

import { Registry, Value } from 'guatiao';

import * as intake from '../src/index.js';
import { Counting } from './counting.js';
import { libraryDir } from './setup.js';

describe('a tree between intake and the core', () => {
  it('the core builds it, intake reads it, and its view refuses once it is closed', () => {
    using counting = new Counting(intake.library().alloc);
    const value = new Value({ agent: [{ name: 'a' }, { name: 'b' }] }, { alloc: counting.alloc });
    const found = intake.at(value, 'agent[1].name')!;
    expect(found.toJS()).toBe('b');
    value.close();
    expect(() => found.toJS()).toThrow();
    expect(counting.live).toBe(0);
  });

  it('intake builds it, the core changes and frees it', () => {
    const schema = new Value({ type: 'object', properties: { name: { type: 'string', 'x-section': 'who' } } });
    const form = intake.forSchema(schema);
    form.asMap().set('note', 'added through the core');
    expect(form.asMap().get('sections')!.toJS()).toEqual([{ id: 'who' }]);
    form.close();
    schema.close();
  });

  it("reads a provider's form from its meta, and it fits the schema", () => {
    using reg = new Registry('node-tests', '0.1');
    reg.scanDir(libraryDir(), { rules: 'kind=greeter' });
    using meta = reg.providerMeta('derived_greeter_shouter')!;
    const form = meta.asMap().get(intake.providerFormKey)!;
    const schema = reg.providerConfig('derived_greeter_shouter')!;
    expect(intake.check(schema, form)).toBe(null);
    const data = form.toJS() as {
      sections: { id: string }[];
      fields: Record<string, Record<string, unknown>>;
    };
    expect(data.sections.map((s) => s.id)).toEqual(['words', 'volume']);
    expect(data.fields.prefix!.placeholder).toBe('hey');
    expect(data.fields.marks!.unit).toBe('marks');
    expect(intake.isVisible(schema, form, 'marks', { exclaim: false })).toBe(false);
    expect(intake.isVisible(schema, form, 'marks', { exclaim: true })).toBe(true);
  });
});
