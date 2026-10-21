// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// guatiao_intake against a real --all-features build, over schemas and
// forms written as plain objects.

import { describe, expect, it } from 'vitest';

import { NotFound, Value, WrongKind } from '../src/index.js';
import * as intake from '../src/intake.js';

const SCHEMA = {
  type: 'object',
  properties: { name: { type: 'string', title: 'Name' } },
  required: ['name'],
  additionalProperties: false,
};

const FLAT_SCHEMA = {
  type: 'object',
  properties: {
    auth: {
      type: 'object',
      'x-variant-tag': 'auth',
      oneOf: [
        { type: 'object', properties: { auth: { const: 'sso' } }, required: ['auth'] },
        {
          type: 'object',
          properties: {
            auth: { const: 'userpass' },
            username: { type: 'string' },
            password: { type: 'string', 'x-sensitive': true },
          },
          required: ['auth'],
        },
      ],
    },
    agent: { type: 'array', items: { type: 'object', properties: { name: { type: 'string' } } } },
    env: { type: 'object', additionalProperties: { type: 'string' } },
    host: { type: 'string', title: 'Host' },
  },
};

/** Reuses freed heap blocks, so a read through a stale address would show. */
function churn(): Value[] {
  return Array.from({ length: 200 }, () => new Value({ ['k'.repeat(40)]: Array(8).fill('x'.repeat(64)) }));
}

describe('intake', () => {
  it('checks a form against its schema', () => {
    expect(intake.check(SCHEMA, {})).toBe(null);
    const error = intake.check(SCHEMA, { sections: [], fields: { nope: {} } });
    expect(error?.kind).toBe('unknown_field');
  });

  it('lays the fields out and says which are shown', () => {
    using schema = new Value(SCHEMA);
    const fields = intake.layout(schema, {}).flatMap((section) => section.fields);
    expect(fields).toEqual(['name']);
    expect(intake.isVisible(schema, {}, 'name', { name: 'ana' })).toBe(true);
    expect(() => intake.isVisible(schema, {}, 'nope', {})).toThrow(NotFound);
  });

  it('names one place inside a value with a path', () => {
    using value = new Value({
      agent: [{ name: 'one' }, { name: 'two' }],
      env: { PATH: '/bin', 'a.b': 'dotted', '1': 'keyed' },
    });
    const at = (path: string): unknown => intake.at(value, path)?.toJS();
    expect(at('agent[0].name')).toBe('one');
    expect(at('agent[1].name')).toBe('two');
    expect(at('env[PATH]')).toBe('/bin');
    expect(at('env[a.b]')).toBe('dotted');
    expect(at('env["1"]')).toBe('keyed');
    expect(intake.at(value, 'agent[9].name')).toBe(undefined);
    expect(intake.at(value, 'nonesuch')).toBe(undefined);
    expect(intake.at(value, 'agent[')).toBe(undefined);
  });

  it('makes a form out of a schema, and finds the one a field is shown with', () => {
    using schema = new Value({
      type: 'object',
      properties: {
        host: { type: 'string', 'x-section': 'net' },
        port: { type: 'integer', 'x-section': 'net' },
        tls: {
          type: 'object',
          properties: { verify: { type: 'boolean', 'x-section': 'trust' } },
          additionalProperties: false,
        },
        label: { type: 'string' },
      },
      additionalProperties: false,
    });
    using made = intake.forSchema(schema);
    const doc = made.toJS<{ sections: { id: string }[]; fields: { tls: { form: { sections: unknown } } } }>();
    expect(doc.sections.map((s) => s.id)).toEqual(['net']);
    expect(doc.fields.tls.form.sections).toEqual([{ id: 'trust' }]);
    expect(intake.check(schema, made)).toBe(null);
    const implied = intake.formFor({}, schema, 'tls');
    expect(implied).toBeDefined();
    implied!.close();
    expect(intake.formFor({}, schema, 'label')).toBe(undefined);
    expect(intake.formFor({}, schema, 'nonesuch')).toBe(undefined);
  });

  it('resolves the field that governs a flat key', () => {
    using schema = new Value(FLAT_SCHEMA);
    expect(intake.resolve(schema, 'host')!.asMap().get('title')!.toJS()).toBe('Host');
    expect(intake.resolve(schema, 'auth.password')!.asMap().get('x-sensitive')!.toJS()).toBe(true);
    expect(intake.resolve(schema, 'nonesuch')).toBe(undefined);
  });

  it('lists the flat keys of a variant field and its arms', () => {
    expect(intake.flatKeys(FLAT_SCHEMA, 'auth')).toEqual(['auth', 'auth.username', 'auth.password']);
    expect(() => intake.flatKeys(FLAT_SCHEMA, 'nonesuch')).toThrow(WrongKind);
  });

  it.each([
    [
      'auth',
      { auth: 'userpass', username: 'alice', password: 's3cret' },
      { auth: 'userpass', 'auth.username': 'alice', 'auth.password': 's3cret' },
    ],
    ['auth', { auth: 'sso' }, { auth: 'sso' }],
    ['agent', [{ name: 'one' }, { name: 'two' }], { 'agent[0].name': 'one', 'agent[1].name': 'two' }],
    ['env', { PATH: '/bin', 'a.b': 'dotted' }, { 'env[PATH]': '/bin', 'env[a.b]': 'dotted' }],
    ['host', 'example.org', { host: 'example.org' }],
  ])('flattens %s and reads it back', (key, value, flat) => {
    expect(intake.flatten(FLAT_SCHEMA, key, value)).toEqual(flat);
    using back = intake.unflatten(FLAT_SCHEMA, key, flat)!;
    expect(back.toJS()).toEqual(value);
  });

  it('answers undefined where a store does not apply, and refuses one that is not text', () => {
    using store = new Value({ 'env[PATH]': '/bin' });
    using env = intake.unflatten(FLAT_SCHEMA, 'env', store)!;
    expect(env.toJS()).toEqual({ PATH: '/bin' });
    expect(intake.unflatten(FLAT_SCHEMA, 'auth', { auth: 'nonesuch' })).toBe(undefined);
    expect(intake.unflatten(FLAT_SCHEMA, 'nonesuch', {})).toBe(undefined);
    expect(intake.unflatten(FLAT_SCHEMA, 'host', {})).toBe(undefined);
    expect(() => intake.unflatten(FLAT_SCHEMA, 'host', { host: 1 })).toThrow(TypeError);
    expect(() => intake.unflatten(FLAT_SCHEMA, 'host', ['host'])).toThrow(TypeError);
    using numbers = new Value({ host: 1 });
    expect(() => intake.unflatten(FLAT_SCHEMA, 'host', numbers)).toThrow(TypeError);
    expect(() => intake.flatten(FLAT_SCHEMA, 'auth', [1])).toThrow(WrongKind);
  });

  it('refuses to read a borrowed view once its source is closed', () => {
    const schema = new Value(FLAT_SCHEMA);
    const host = intake.resolve(schema, 'host')!;
    const value = new Value({ a: { b: 'c'.repeat(50) } });
    const view = intake.at(value, 'a')!;
    schema.close();
    value.close();
    const junk = churn();
    expect(() => host.toJS()).toThrow(/closed/);
    expect(() => view.toJS()).toThrow(/closed/);
    for (const item of junk) item.close();
  });

  it('follows a change to the schema a field was resolved from', () => {
    using schema = new Value(FLAT_SCHEMA);
    const host = intake.resolve(schema, 'host')!;
    schema.asMap().get('properties')!.asMap().set('host', { title: 'Moved' });
    expect(host.asMap().get('title')!.toJS()).toBe('Moved');
  });

  it('changes nothing through a borrowed view', () => {
    using schema = new Value(FLAT_SCHEMA);
    const host = intake.resolve(schema, 'host')!.asMap();
    expect(() => host.delete('title')).toThrow(TypeError);
    expect(() => host.set('title', 'x')).toThrow(TypeError);
    expect(() => host.clear()).toThrow(TypeError);
    expect(() => host.copyFrom(host)).toThrow(TypeError);
    expect(host.get('title')!.toJS()).toBe('Host');
    using value = new Value({ tags: ['a', 'b'] });
    const tags = intake.at(value, 'tags')!.asList();
    expect(() => tags.push('c')).toThrow(TypeError);
    expect(() => tags.removeAt(0)).toThrow(TypeError);
    expect(() => tags.set(0, 'z')).toThrow(TypeError);
    expect(() => tags.insert(0, 'z')).toThrow(TypeError);
    expect(() => tags.clear()).toThrow(TypeError);
    expect(value.toJS()).toEqual({ tags: ['a', 'b'] });
    using copy = tags.clone();
    expect(copy.toJS()).toEqual(['a', 'b']);
  });

  it('takes a ref where it takes a value', () => {
    using config = new Value({ host: 'h', env: { PATH: '/bin' } });
    const fields = config.asMap();
    expect(intake.flatten(FLAT_SCHEMA, 'host', fields.get('host'))).toEqual({ host: 'h' });
    expect(intake.flatten(FLAT_SCHEMA, 'env', fields.get('env'))).toEqual({ 'env[PATH]': '/bin' });
    using holder = new Value({ flat: { 'env[PATH]': '/bin' } });
    using back = intake.unflatten(FLAT_SCHEMA, 'env', holder.asMap().get('flat'))!;
    expect(back.toJS()).toEqual({ PATH: '/bin' });
  });
});
