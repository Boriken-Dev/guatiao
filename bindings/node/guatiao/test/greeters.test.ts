// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// One behaviour, four languages: a Node host calls the greeters written in
// Rust, C, Python and TypeScript through one description of the kind, and
// a Rust caller calls each of them through the kind's Rust proxy.

import path from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { AllocFailed, BadValue, Internal, Registry, Unsupported, Value } from '../src/index.js';
import * as n from '../src/native.js';
import { liveObjects, type Remote } from '../src/kinds.js';
import { libraryOf } from '../src/provide.js';
import { adopt } from '../src/value.js';
import { Counter, Greeter } from '../../../../examples/greeter_kind/node/greeter_kind.js';
import { Counting, REPO } from './helpers.js';
import { PYTHON_FREES_HERE, bridge, bridgeConfig, cLibraries, workspaceLibrary } from './native.js';
import { libraryDir } from './setup.js';

const NODE_GREETER = path.join(REPO, 'examples', 'node_provider', 'node_greeter', 'greeters.ts');

/** Each greeter: where it comes from, its key, the greeting it answers "ana" with. */
const GREETERS = {
  rust: { key: 'derived_greeter_hello', greeting: 'hello, ana' },
  c: { key: 'c_greeter', greeting: 'hello from C, ana' },
  python: { key: 'py_greeter_hello', greeting: 'hello, ana' },
  node: { key: 'node_greeter_hello', greeting: 'hello, ana' },
} as const;
type Which = keyof typeof GREETERS;

class Ear {
  readonly heardSoFar: string[] = [];
  heard(what: string): void {
    this.heardSoFar.push(what);
  }
}

let reg: Registry;
const present = new Set<Which>();

beforeAll(async () => {
  reg = new Registry('node-greeters', '0.1');
  reg.scanDir(libraryDir(), { rules: 'kind=greeter' });
  present.add('rust');
  const cDir = cLibraries()?.greeterDir;
  if (cDir !== undefined) {
    reg.scanDir(cDir, { rules: 'kind=greeter' });
    present.add('c');
  }
  const py = bridge();
  if (py !== null) {
    reg.addSourceFile(py, bridgeConfig());
    const changed = reg.refresh();
    expect(changed.refused).toEqual([]);
    present.add('python');
  }
  reg.registerLibrary(await libraryOf(NODE_GREETER));
  present.add('node');
});

afterAll(() => {
  reg.close();
});

const each = it.for(Object.keys(GREETERS) as Which[]);
function greeter(which: Which, skip: (note: string) => void): Remote<typeof Greeter> {
  if (!present.has(which)) {
    skip(which === 'c' ? 'no C compiler: the greeter written in C is not built' : 'no guatiao-source-py build: set GUATIAO_PY_BRIDGE');
  }
  return reg.bind(GREETERS[which].key, Greeter);
}

describe('a Node host calls every greeter', () => {
  each('%s greets', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    using answer = remote.greet('ana');
    expect(answer.toJS()).toEqual({ greeting: GREETERS[which].greeting });
  });

  each('%s refuses with its own words', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    expect(() => remote.greet('')).toThrow(BadValue);
    expect(() => remote.greet('')).toThrow(/nobody to greet/);
  });

  each('%s turns a failure inside the method into Internal', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    // The greeter written in C greets anyone, panic included.
    if (which === 'c') expect(remote.greet('panic').close()).toBe(undefined);
    else expect(() => remote.greet('panic')).toThrow(Internal);
  });

  each('%s fills a lent map in place', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    using config = new Value({ name: 'bo' });
    expect(remote.check(config)).toBe(true);
    const filled = config.toJS<Record<string, unknown>>();
    expect(filled.name).toBe('bo');
    expect(Object.keys(filled)).toHaveLength(2);
  });

  each('%s greets from a map handed over', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    const config = new Value({ name: 'ana' });
    using answer = remote.greetFrom(config);
    expect(answer.toJS()).toEqual({ greeting: GREETERS[which].greeting });
    expect(config.isAbsent()).toBe(true);
    config.close();
  });

  each('%s tells a lent listener', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    const ear = new Ear();
    remote.greetInto('ana', ear);
    expect(ear.heardSoFar).toEqual([GREETERS[which].greeting]);
  });

  each('%s holds a conversation told to the listener handed in', (which, ctx) => {
    const remote = greeter(which, (note) => ctx.skip(note));
    const before = liveObjects();
    const ear = new Ear();
    if (which === 'c') {
      expect(remote.has('start')).toBe(false);
      expect(() => remote.start(ear)).toThrow(Unsupported);
      return;
    }
    const conversation = remote.start(ear);
    conversation.say('hi');
    conversation.say('again');
    expect(conversation.turns()).toBe(2n);
    const transcript = new Uint8Array(16);
    const n = conversation.read(transcript);
    expect(Buffer.from(transcript.subarray(0, Number(n))).toString()).toBe('hi\nagain');
    expect(() => conversation.say('')).toThrow(BadValue);
    conversation.close();
    expect(conversation.closed).toBe(true);
    expect(ear.heardSoFar).toEqual(['hi', 'again']);
    if (which !== 'python') expect(liveObjects()).toBe(before);
  });

  (PYTHON_FREES_HERE ? it : it.fails)('python lets go of the listener a closed conversation held', (ctx) => {
    const remote = greeter('python', (note) => ctx.skip(note));
    const before = liveObjects();
    remote.start(new Ear()).close();
    expect(liveObjects()).toBe(before);
  });

  it('counts through a provider that serves two kinds', () => {
    const counter = reg.bind('node_greeter_hello', Counter);
    const first = counter.count();
    expect(counter.count()).toBe(first + 1n);
    expect(reg.provider('node_greeter_hello').kinds).toEqual(['greeter', 'counter']);
  });
});

describe('a tree built in the library\'s allocator outlives the unload', () => {
  // Each test unloads a library of its own: no other registry keeps it mapped.
  it('answered by a greeter that ignores the carrier', (ctx) => {
    const dir = cLibraries()?.carelessGreeterDir;
    if (dir === undefined) return ctx.skip('no C compiler: the greeter written in C is not built');
    using own = new Registry('node-careless', '0.1');
    own.scanDir(dir, { rules: 'kind=greeter' });
    const answer = own.bind('c_greeter', Greeter).greet('hal');
    // The C greeter has no `unload` function: nothing it handed out is
    // alive, which the caller states.
    own.unload('c_greeter_library', { unchecked: true });
    expect(own.libraries()).toEqual([]);
    expect(answer.toJS()).toEqual({ greeting: 'hello from C, hal' });
    answer.close();
  });

  it('grown into a lent map by a slot that rebuilt it in its own allocator', (ctx) => {
    const dir = cLibraries()?.lentGreeterDir;
    if (dir === undefined) return ctx.skip('no C compiler: the greeter written in C is not built');
    using own = new Registry('node-lent', '0.1');
    own.scanDir(dir, { rules: 'kind=greeter' });
    using config = new Value({ name: 'ivy' });
    expect(own.bind('c_greeter', Greeter).check(config)).toBe(true);
    own.unload('c_greeter_library', { unchecked: true });
    expect(config.toJS()).toEqual({ name: 'ivy', checked_by: 'C' });
  });

  it("refused when moving a lent map home fails, as the call's error", (ctx) => {
    // `check` rebuilds the lent map in the C file's own allocator; moving
    // it back into the map's, which refuses, fails the call and leaves the
    // map whole where it stands.
    const dir = cLibraries()?.carelessGreeterDir;
    if (dir === undefined) return ctx.skip('no C compiler: the greeter written in C is not built');
    using counting = new Counting();
    using own = new Registry('node-refused', '0.1');
    own.scanDir(dir, { rules: 'kind=greeter' });
    const greeter = own.bind('c_greeter', Greeter);
    {
      using config = new Value({ name: 'ivy' }, { alloc: counting.alloc });
      counting.refusing = true;
      try {
        let refused: unknown;
        try {
          greeter.check(config);
        } catch (e) {
          refused = e;
        }
        expect(refused).toBeInstanceOf(AllocFailed);
        expect((refused as Error).message).toMatch(/lent tree/);
      } finally {
        counting.refusing = false;
      }
      expect(config.toJS()).toEqual({ name: 'ivy', checked_by: 'C' });
    }
    expect(counting.live).toBe(0);
  });

  it("removed when the slot wrote a key that is not UTF-8 into a lent map, as the call's error", (ctx) => {
    const dir = cLibraries()?.badKeyGreeterDir;
    if (dir === undefined) return ctx.skip('no C compiler: the greeter written in C is not built');
    using counting = new Counting();
    using own = new Registry('node-bad-key', '0.1');
    own.scanDir(dir, { rules: 'kind=greeter' });
    {
      using config = new Value({ name: 'ivy' }, { alloc: counting.alloc });
      let refused: unknown;
      try {
        own.bind('c_greeter', Greeter).check(config);
      } catch (e) {
        refused = e;
      }
      expect(refused).toBeInstanceOf(BadValue);
      expect((refused as Error).message).toMatch(/not UTF-8/);
      expect(config.toJS()).toEqual({ name: 'ivy' });
    }
    expect(counting.live).toBe(0);
  });
});

describe('a Rust caller calls every greeter through its proxy', () => {
  let run: (host: bigint, key: n.StrArg, name: n.StrArg, out: bigint) => number;
  beforeAll(() => {
    const lib = n.koffi().load(workspaceLibrary('greeter_caller'));
    run = lib.func('uint32_t greeter_caller_run(void *host, guatiao_str key, guatiao_str name, void *out)') as typeof run;
  });

  each('%s, called from Rust', (which, ctx) => {
    greeter(which, (note) => ctx.skip(note));
    const out = n.alloc(40);
    expect(run(reg.host(), n.str(GREETERS[which].key), n.str('ana'), out)).toBe(0);
    using report = adopt(out);
    const got = report.toJS<Record<string, unknown>>();
    const greeting = GREETERS[which].greeting;
    expect(got.greet).toBe(greeting);
    expect(got.refusal).toEqual({ status: 7, message: expect.stringContaining('nobody to greet') });
    expect(got.failure).toBe(which === 'c' ? 0 : 100);
    expect(got.shout).toBe(greeting.toUpperCase());
    expect(got.checked).toBe(true);
    expect(got.greet_from).toBe(greeting);
    expect(got.heard).toEqual([greeting]);
    if (which === 'c') {
      // No `start` in its table: the Rust proxy runs the kind's default.
      expect(got.conversation).toEqual({ status: 11, message: 'this greeter holds no conversations' });
    } else {
      expect(got.conversation).toEqual({
        turns: 2,
        transcript: 'hi\nagain',
        refused: {},
        heard: ['hi', 'again'],
        dropped: which !== 'python' || PYTHON_FREES_HERE,
      });
    }
  });
});
