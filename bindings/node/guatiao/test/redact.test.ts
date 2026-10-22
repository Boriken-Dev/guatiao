// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// redact: a copy with every secret the schema marks hidden.

import { describe, expect, it } from 'vitest';

import { Value, WrongKind, redact } from '../src/index.js';

const SCHEMA = {
  type: 'object',
  properties: {
    user: { type: 'string' },
    password: { type: 'string', writeOnly: true, 'x-sensitive': true },
    token: { type: 'string', writeOnly: true },
    tls: { type: 'object', properties: { ca: { type: 'string' }, key: { $ref: '#/$defs/Key' } } },
    recovery: { type: 'array', items: { type: 'string', 'x-sensitive': true } },
  },
  $defs: { Key: { type: 'string', 'x-sensitive': true } },
};

const LOGIN = {
  user: 'ana',
  password: 'hunter2',
  token: 'tok-9f',
  tls: { ca: '/etc/ca.pem', key: '-----BEGIN KEY-----' },
  recovery: ['r1', 'r2'],
};

describe('redact', () => {
  it('hides every secret at any depth', () => {
    using shown = redact(SCHEMA, LOGIN);
    expect(shown.toJS()).toEqual({
      user: 'ana',
      password: '[redacted]',
      token: '[redacted]',
      tls: { ca: '/etc/ca.pem', key: '[redacted]' },
      recovery: ['[redacted]', '[redacted]'],
    });
  });

  it('takes values and refs as they are', () => {
    using schema = new Value(SCHEMA);
    using doc = new Value({ inner: LOGIN });
    using shown = redact(schema, doc.asMap().get('inner'), '***');
    expect(shown.asMap().get('password')!.toJS()).toBe('***');
    expect(shown.asMap().get('user')!.toJS()).toBe('ana');
  });

  it('hides an undeclared key, and refuses a schema that is not a map', () => {
    using shown = redact(SCHEMA, { user: 'ana', extra: '?' }, '*');
    expect(shown.toJS()).toEqual({ user: 'ana', extra: '*' });
    expect(() => redact(5, LOGIN)).toThrow(WrongKind);
  });
});
