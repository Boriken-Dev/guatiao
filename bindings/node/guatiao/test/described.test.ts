// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind loaded from its description, the module guatiao-kind writes from
// one, and a kind defined in JavaScript as a class.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { BadValue, Registry } from '../src/index.js';
import { generate } from '../src/generate.js';
import { Kind, type KindsDocument, type Remote, describeKinds, kind, kindOf, kindsOf, slot, t } from '../src/kinds.js';
import { Provider, library } from '../src/provide.js';
import * as committed from '../../../../examples/greeter_kind/node/greeter_kind.js';
import { GREETER_KIND_H, REPO, readText } from './helpers.js';

const KIND_DIR = path.join(REPO, 'examples', 'greeter_kind');
const DOCUMENT = path.join(KIND_DIR, 'include', 'greeter_kind.kinds.json');
const GENERATED = path.join(KIND_DIR, 'node', 'greeter_kind');

const document = (): KindsDocument => JSON.parse(readText(DOCUMENT)) as KindsDocument;

describe('a kind loaded from its description', () => {
  it('is the kind its header declares', () => {
    const header = readText(GREETER_KIND_H);
    const kinds = kindsOf(document());
    expect(Object.keys(kinds)).toEqual(['greeter', 'listener', 'conversation', 'counter', 'wide']);
    for (const [name, table] of [
      ['greeter', 'greeter_vtable'],
      ['listener', 'listener_vtable'],
      ['conversation', 'conversation_vtable'],
      ['counter', 'counter_vtable'],
      ['wide', 'wide_vtable'],
    ] as const) {
      const described = kinds[name]!;
      const hash = new RegExp(`#define ${table}_FLOOR_HASH (\\d+)`).exec(header);
      expect(Number(hash?.[1]), table).toBe(described.floorHash);
      const body = new RegExp(`typedef struct ${table} \\{([\\s\\S]*?)\\} ${table};`).exec(header)![1]!;
      const slots = body.match(/\(\*\w+\)/g)!;
      expect(described.tableSize, table).toBe(8 + slots.length * 8);
      expect(described.isObject, table).toBe(body.includes('(*destroy)'));
    }
    expect(Object.keys(kinds.greeter!.methods)).toEqual(['greet', 'shout', 'start', 'greetInto', 'greetFrom', 'check']);
    expect(kinds.greeter!.floor).toBe(16);
  });

  it('takes the JSON text as well, and one kind alone', () => {
    const text = readText(DOCUMENT);
    expect(kindsOf(text).counter!.floorHash).toBe(committed.Counter.floorHash);
    const listener = document().kinds.find((k) => k.name === 'listener')!;
    expect(kindOf(listener).isObject).toBe(true);
    const greeter = document().kinds.find((k) => k.name === 'greeter')!;
    expect(() => kindOf(greeter)).toThrow(/no kind listener/);
    expect(kindOf(greeter, { known: [committed.Listener, committed.Conversation] }).tableSize).toBe(committed.Greeter.tableSize);
  });

  it('refuses a document it cannot read, naming where', () => {
    const one = (slots: unknown, extra: unknown[] = []): KindsDocument =>
      ({ format: 1, kinds: [{ name: 'k', floor_hash: 1, slots }, ...extra] }) as never;
    expect(() => kindsOf({ format: 2, kinds: [] } as never)).toThrow(/format: 1/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'u128' }] }]))).toThrow(/k\.x argument 0: u128/);
    expect(() => kindsOf(one([{ name: 'x', answer: { type: 'str' } }]))).toThrow(/k\.x answer: an answer cannot be str/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'list' }] }]))).toThrow(/a list cannot be passed borrowed/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'nowhere', pass: 'lent' }] }]))).toThrow(
      /no kind nowhere/,
    );
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'k', pass: 'lent' }] }]))).toThrow(
      /refers to itself/,
    );
    const provider = { name: 'p', floor_hash: 2, slots: [{ name: 'y' }] };
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'p', pass: 'taken' }] }], [provider]))).toThrow(
      /p is not an object kind/,
    );
    expect(() => kindsOf(one([{ name: 'a_b' }, { name: 'aB' }]))).toThrow(/two slots are aB/);
  });
});

describe('the module guatiao-kind writes', () => {
  it('is what the committed description writes', () => {
    const fresh = generate(document(), {
      source: '../include/greeter_kind.kinds.json',
      command: 'guatiao-kind ../include/greeter_kind.kinds.json --out greeter_kind',
    });
    expect(readText(`${GENERATED}.js`)).toBe(fresh.js);
    expect(readText(`${GENERATED}.d.ts`)).toBe(fresh.dts);
  });

  it('exports each kind under its type name, loaded', () => {
    expect(committed.Greeter.name).toBe('greeter');
    expect(committed.Conversation.isObject).toBe(true);
    expect(committed.Wide.methods).toHaveProperty('echoU64');
  });

  it('is checked by the command it names', (ctx) => {
    const cli = path.join(REPO, 'bindings', 'node', 'guatiao', 'dist', 'kind_cli.js');
    if (!existsSync(cli)) return ctx.skip('the package is not built: npm run build');
    const run = (...args: string[]): string =>
      execFileSync(process.execPath, [cli, ...args], { cwd: path.join(KIND_DIR, 'node'), encoding: 'utf8' });
    expect(run('../include/greeter_kind.kinds.json', '--out', 'greeter_kind', '--check')).toBe('');
    expect(() =>
      execFileSync(process.execPath, [cli, DOCUMENT, '--out', path.join(KIND_DIR, 'node', 'nothing_here'), '--check'], {
        stdio: 'pipe',
      }),
    ).toThrow(/not what/);
  });

  it('names its types after the kinds and keeps its own names apart', () => {
    const { dts } = generate({
      format: 1,
      kinds: [{ name: 'session.backend_v2', floor_hash: 3, object: false, slots: [{ name: 'open', required: true, args: [{ name: 'default', type: 'u8' }] }] }],
    });
    expect(dts).toContain('export interface SessionBackendV2 {');
    expect(dts).toContain('  open(default_: number): void;');
    expect(() => generate({ format: 1, kinds: [{ name: 'a', floor_hash: 1, slots: [] }, { name: 'A', floor_hash: 2, slots: [] }] })).toThrow(
      /two kinds are A/,
    );
  });
});

// ---- a kind defined in JavaScript, as a class --------------------------------------

class Note extends Kind {
  static readonly kind = 'note';
  static readonly floorHash = 77;
  static readonly object = true;
  static readonly slots = {
    add: slot(t.str).required(),
    count: slot().returns(t.u32).required(),
  };
}

class Echo extends Kind {
  static readonly floorHash = 1234;
  static readonly slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
    tell: slot(t.str, t.lent(Note)).fallible(),
    open: slot().returns(Note),
  };
}

class Shelf {
  readonly #lines: string[] = [];
  add(line: string): void {
    this.#lines.push(line);
  }
  count(): number {
    return this.#lines.length;
  }
}

class Parrot extends Provider {
  static override readonly kinds = [Echo];
  echo(what: string): string {
    if (!what) throw new BadValue('nothing to repeat');
    return `${what}, ${what}`;
  }
  tell(what: string, note: Remote<typeof Note>): void {
    note.add(what);
    note.add(what.toUpperCase());
  }
  open(): Shelf {
    return new Shelf();
  }
}

describe('a kind defined as a class', () => {
  it('is named after its class unless it says, and described once', () => {
    using reg = new Registry('classes', '0.1');
    reg.registerLibrary(library('parrots', '1.0', { providers: [Parrot] }));
    const echo = reg.bind('parrots_parrot', Echo);
    expect(echo.kind).toBe(reg.bind('parrots_parrot', Echo).kind);
    expect(echo.kind.name).toBe('echo');
    expect(echo.kind.floorHash).toBe(1234);
    expect(reg.providers('echo').map((p) => p.key)).toEqual(['parrots_parrot']);
  });

  it('is served and called through its own class', () => {
    using reg = new Registry('classes', '0.1');
    reg.registerLibrary(library('parrots', '1.0', { providers: [Parrot] }));
    const echo = reg.bind('parrots_parrot', Echo);
    expect(echo.echo('hi')).toBe('hi, hi');
    expect(() => echo.echo('')).toThrow(/nothing to repeat/);
    const shelf = new Shelf();
    echo.tell('ana', shelf);
    expect(shelf.count()).toBe(2);
    using opened = echo.open();
    opened.add('x');
    expect(opened.count()).toBe(1);
  });

  it('refuses a provider missing a required slot, naming it', () => {
    class Mute extends Provider {
      static override readonly kinds = [Echo];
    }
    expect(() => library('mute', '1', { providers: [Mute] })).toThrow(
      new TypeError('Mute does not implement echo, which the echo kind requires'),
    );
  });

  it('refuses statics that do not describe a kind', () => {
    class Hashless extends Kind {
      static readonly slots = { x: slot() };
    }
    class Slotless extends Kind {
      static readonly floorHash = 1;
    }
    class Uses extends Provider {
      static override readonly kinds = [Hashless as never];
    }
    expect(() => library('a', '1', { providers: [Uses] })).toThrow(/floorHash/);
    class UsesToo extends Provider {
      static override readonly kinds = [Slotless as never];
    }
    expect(() => library('a', '1', { providers: [UsesToo] })).toThrow(/static slots/);
  });

  it('and the descriptor underneath it agree on the table', () => {
    const same = kind('echo', {
      floorHash: 1234,
      methods: {
        echo: { args: [t.str], answer: t.text, required: true, fallible: true },
        tell: { args: [t.str, t.lent(Note)], fallible: true },
        open: { answer: Note },
      },
    });
    using reg = new Registry('classes', '0.1');
    reg.registerLibrary(library('parrots', '1.0', { providers: [Parrot] }));
    const echo = reg.bind('parrots_parrot', same);
    expect(echo.echo('ho')).toBe('ho, ho');
    expect(echo.kind.tableSize).toBe(reg.bind('parrots_parrot', Echo).kind.tableSize);
  });
});

// ---- a kind described again ---------------------------------------------------------------------------

class Memo extends Kind {
  static readonly kind = 'memo';
  static readonly floorHash = 77;
  static readonly object = true;
  static readonly doc = 'Takes lines.';
  static readonly slots = {
    add: slot(t.str).named('line').doc('One more line.').required(),
    count: slot().returns(t.u32).required(),
  };
}

class Relay extends Kind {
  static readonly floorHash = 1234;
  static readonly slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
    tellAll: slot(t.str, t.lent(Memo), t.taken(t.map), t.bytes).fallible(),
    open: slot().returns(Memo),
  };
}

describe('a kind described again', () => {
  const expected: KindsDocument = {
    format: 1,
    kinds: [
      {
        name: 'relay',
        floor_hash: 1234,
        object: false,
        slots: [
          {
            name: 'echo',
            required: true,
            fallible: true,
            args: [{ name: 'arg0', type: 'str' }],
            answer: { type: 'text' },
          },
          {
            name: 'tell_all',
            required: false,
            fallible: true,
            args: [
              { name: 'arg0', type: 'str' },
              { name: 'arg1', type: 'object', kind: 'memo', pass: 'lent' },
              { name: 'arg2', type: 'map', pass: 'taken' },
              { name: 'arg3', type: 'bytes' },
            ],
          },
          { name: 'open', required: false, fallible: false, args: [], answer: { type: 'object', kind: 'memo' } },
        ],
      },
      {
        name: 'memo',
        floor_hash: 77,
        object: true,
        doc: 'Takes lines.',
        slots: [
          {
            name: 'add',
            required: true,
            fallible: false,
            args: [{ name: 'line', type: 'str' }],
            doc: 'One more line.',
          },
          { name: 'count', required: true, fallible: false, args: [], answer: { type: 'u32' } },
        ],
      },
    ],
  };

  it('is the document it was loaded from, the greeter kinds included', () => {
    expect(describeKinds(Object.values(kindsOf(document())))).toEqual(document());
  });

  it('is described, a class too, with the object kinds its slots reach', () => {
    expect(describeKinds([Relay])).toEqual(expected);
    expect(describeKinds([Memo, Relay]).kinds.map((k) => k.name)).toEqual(['memo', 'relay']);
  });

  it('is the kind its description loads', () => {
    const loaded = kindsOf(describeKinds([Relay]));
    expect(loaded.relay?.floorHash).toBe(1234);
    expect(describeKinds(Object.values(loaded))).toEqual(expected);
  });

  it('describes a kind() as it was given, and refuses what a description cannot say', () => {
    const made = kind('plain', { floorHash: 5, methods: { greetInto: { args: [t.str], fallible: true } } });
    expect(describeKinds([made]).kinds[0]?.slots[0]?.name).toBe('greet_into');
    const odd = kind('odd', { floorHash: 6, methods: { parseHTTP: {} } });
    expect(() => describeKinds([odd])).toThrow(/odd.parseHTTP: its slot would be named parse_http/);
    expect(() => describeKinds([made, kind('plain', { floorHash: 9, methods: {} })])).toThrow(/two different kinds are called plain/);
  });
});
