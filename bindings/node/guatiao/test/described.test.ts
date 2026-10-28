// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind loaded from its description, the module guatiao-kind writes from
// one, and a kind defined in JavaScript as a class.

import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { describe, expect, it } from 'vitest';

import { WORD } from '../src/abi.js';
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
      expect(described.tableSize, table).toBe(8 + slots.length * WORD);
      expect(described.isObject, table).toBe(body.includes('(*destroy)'));
    }
    expect(Object.keys(kinds.greeter!.methods)).toEqual(['greet', 'shout', 'start', 'greetInto', 'greetFrom', 'check']);
    expect(kinds.greeter!.floor, 'every slot, optional ones too').toBe(kinds.greeter!.tableSize);
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
      ({ kinds: [{ name: 'k', floor_hash: 1, slots }, ...extra] }) as never;
    expect(() => kindsOf({ name: 'k' } as never)).toThrow(/\{ kinds: \[\.\.\.\] \}/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'u128' }] }]))).toThrow(/k\.x argument 0: u128/);
    expect(() => kindsOf(one([{ name: 'x', answer: { type: 'str' } }]))).toThrow(/k\.x answer: an answer cannot be str/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'list' }] }]))).toThrow(/a list is not borrowed/);
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'nowhere', pass: 'lent' }] }]))).toThrow(
      /no kind nowhere/,
    );
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'k', pass: 'lent' }] }]))).toThrow(
      /k is not an object kind/,
    );
    const provider = { name: 'p', floor_hash: 2, slots: [{ name: 'y' }] };
    expect(() => kindsOf(one([{ name: 'x', args: [{ type: 'object', kind: 'p', pass: 'taken' }] }], [provider]))).toThrow(
      /p is not an object kind/,
    );
    expect(() => kindsOf(one([{ name: 'a_b' }, { name: 'aB' }]))).toThrow(/two slots are aB/);
  });
});

describe('modules written from two documents', () => {
  it('import the object kinds the other one describes', async () => {
    const whole = document();
    const objects: KindsDocument = { kinds: whole.kinds.filter((k) => k.object) };
    const greeters: KindsDocument = { kinds: whole.kinds.filter((k) => !k.object) };
    const imports = { listener: './objects.js', conversation: './objects.js' };
    expect(() => generate(greeters)).toThrow(/the object kind listener is not in this document/);
    // Beside this file, so the modules resolve `guatiao` as the suite does.
    const dir = mkdtempSync(path.join(path.dirname(fileURLToPath(import.meta.url)), 'generated-'));
    try {
      const o = generate(objects);
      const g = generate(greeters, { imports });
      expect(g.js).toContain("import { Listener, Conversation } from './objects.js';");
      expect(g.js).toContain('{ known: [Listener, Conversation] }');
      expect(g.dts).toContain(
        "import type { Listener, ListenerRemote, Conversation, ConversationRemote } from './objects.js';",
      );
      expect(g.dts).toContain('greetInto?(name: string, listener: ListenerRemote): void;');
      writeFileSync(path.join(dir, 'objects.js'), o.js);
      writeFileSync(path.join(dir, 'greeters.js'), g.js);
      const loaded = (await import(pathToFileURL(path.join(dir, 'greeters.js')).href)) as Record<string, { tableSize: number; floorHash: number }>;
      const imported = (await import(pathToFileURL(path.join(dir, 'objects.js')).href)) as Record<string, unknown>;
      expect(loaded.Greeter!.tableSize).toBe(committed.Greeter.tableSize);
      expect(loaded.Greeter!.floorHash).toBe(committed.Greeter.floorHash);
      // One kind object, imported, not described again.
      expect(describeKinds([loaded.Greeter as never]).kinds.map((k) => k.name)).toEqual(['greeter', 'listener', 'conversation']);
      expect(imported.Listener).toBeDefined();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
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
      kinds: [{ name: 'session.backend_v2', floor_hash: 3, object: false, slots: [{ name: 'open', required: true, args: [{ name: 'default', type: 'u8' }] }] }],
    });
    expect(dts).toContain('export interface SessionBackendV2 {');
    expect(dts).toContain('  open(default_: number): void;');
    expect(() => generate({ kinds: [{ name: 'a', floor_hash: 1, slots: [] }, { name: 'A', floor_hash: 2, slots: [] }] })).toThrow(
      /two kinds are A/,
    );
  });
});

// ---- kinds that name themselves --------------------------------------------------------

/** A node answers its children, nodes too, and is lent another; a forest answers a root. */
const TREES: KindsDocument = {
  kinds: [
    {
      name: 'forest',
      floor_hash: 6,
      object: false,
      slots: [{ name: 'root', required: true, fallible: true, args: [], answer: { type: 'object', kind: 'node' } }],
    },
    {
      name: 'node',
      floor_hash: 5,
      object: true,
      slots: [
        { name: 'label', required: true, fallible: false, args: [], answer: { type: 'text' } },
        {
          name: 'child',
          required: true,
          fallible: true,
          args: [{ name: 'label', type: 'str' }],
          answer: { type: 'object', kind: 'node' },
        },
        {
          name: 'size_of',
          required: false,
          fallible: true,
          args: [{ name: 'other', type: 'object', kind: 'node', pass: 'lent' }],
          answer: { type: 'u32' },
        },
      ],
    },
  ],
};

class Leaf {
  constructor(readonly name: string) {}
  label(): string {
    return this.name;
  }
  child(label: string): Leaf {
    return new Leaf(`${this.name}/${label}`);
  }
  sizeOf(other: { label(): string }): number {
    return other.label().length;
  }
}

describe('a kind that names itself through its slots', () => {
  it('loads, is described as given, and its objects answer objects of itself', () => {
    const { forest, node } = kindsOf(TREES);
    expect(describeKinds([forest!, node!])).toEqual(TREES);
    class Woods extends Provider {
      static override readonly kinds = [forest as never];
      root(): Leaf {
        return new Leaf('root');
      }
    }
    using reg = new Registry('trees', '0.1');
    reg.registerLibrary(library('woods', '1.0', { providers: [Woods] }));
    const woods = reg.bind('woods_woods', forest!) as unknown as {
      root(): { label(): string; child(l: string): { label(): string; close(): void }; sizeOf(o: unknown): number; close(): void };
    };
    const root = woods.root();
    const twig = root.child('a');
    expect(twig.label()).toBe('root/a');
    // An object of the kind lent to an object of the same kind.
    expect(root.sizeOf(new Leaf('four'))).toBe(4);
    expect(root.sizeOf(twig)).toBe(6);
    twig.close();
    root.close();
  });

  it('or names one that names it back', () => {
    const cycle: KindsDocument = {
      kinds: [
        { name: 'ping', floor_hash: 1, object: true, slots: [{ name: 'pong', required: true, fallible: true, args: [], answer: { type: 'object', kind: 'pong' } }] },
        { name: 'pong', floor_hash: 2, object: true, slots: [{ name: 'ping', required: true, fallible: true, args: [], answer: { type: 'object', kind: 'ping' } }] },
      ],
    };
    const loaded = kindsOf(cycle);
    expect(describeKinds([loaded.ping!])).toEqual(cycle);
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
    open: slot().returns(Note).fallible(),
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
        open: { answer: Note, fallible: true },
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
    open: slot().returns(Memo).fallible(),
  };
}

describe('a kind described again', () => {
  const expected: KindsDocument = {
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
          { name: 'open', required: false, fallible: true, args: [], answer: { type: 'object', kind: 'memo' } },
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
    class Short extends Kind {
      static readonly floorHash = 7;
      static readonly slots = { f: slot(t.str, t.u32).named('only') };
    }
    expect(() => describeKinds([Short])).toThrow(/short\.f: a name for each of its 2 arguments, not \["only"\]/);
  });

  it('refuses what no trait declares, with no description to check and no library', () => {
    expect(() => kind('k', { floorHash: 1, methods: { header: {} } })).toThrow(/k\.header: the table's first field/);
    expect(() => kind('k', { floorHash: 1, methods: { read: { args: [t.str, t.lent(Memo)] } } })).toThrow(
      /k\.read: argument 1, an object, crosses on a fallible slot only/,
    );
    expect(() => kind('k', { floorHash: 1, methods: { give: { args: [t.taken(Memo)] } } })).toThrow(/on a fallible slot only/);
    expect(() => kind('k', { floorHash: 1, methods: { open: { answer: Memo } } })).toThrow(/k\.open: an object answer crosses on a fallible slot only/);
    expect(() => kind('k', { floorHash: 1, methods: { fill: { args: [t.bytesMut] } } })).toThrow(/on an object kind only/);
    expect(() => kind('k', { floorHash: 1, methods: { open: { args: [t.lent(Memo)], answer: Memo, fallible: true } } })).not.toThrow();
  });
});
