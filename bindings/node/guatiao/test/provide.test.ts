// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Providers written in TypeScript: what a provider class and a module
// library are checked for, configurations, availability, and the
// lifetime of what a host builds.

import { mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { describe, expect, it } from 'vitest';

import { BadValue, Internal, Registry, Value, WrongKind } from '../src/index.js';
import { LIBRARY_INFO, PROVIDER_INFO, SIZE, Status } from '../src/abi.js';
import { kind, liveObjects, t } from '../src/kinds.js';
import * as n from '../src/native.js';
import { Provider, library, libraryOf, s } from '../src/provide.js';
import { Counter, Greeter, Listener } from '../../../../examples/greeter_kind/node/greeter_kind.js';
import { REPO } from './helpers.js';
import { scratch } from './native.js';

const NODE_GREETER = path.join(REPO, 'examples', 'node_provider', 'node_greeter', 'greeters.ts');

class Plain extends Provider implements Counter {
  static override readonly kinds = [Counter];
  count(): bigint {
    return 1n;
  }
}

const Limits = s.object({
  low: s.integer(),
  high: s.optional(s.integer({ minimum: 0 }), { default: 10 }),
  label: s.optional(s.nullable(s.string())),
});

class Bounded extends Provider implements Counter {
  static override readonly kinds = [Counter];
  static override readonly named = 'bounded';
  static override readonly displayName = 'Bounded';
  static override readonly config = Limits;
  readonly #limits: s.Infer<typeof Limits>;
  #at: number;

  constructor(config: s.Infer<typeof Limits>) {
    super();
    this.#limits = config;
    this.#at = config.low;
  }

  count(): bigint {
    this.#at += 1;
    return BigInt(Math.min(this.#at, this.#limits.high));
  }
}

describe('a module is its library', () => {
  it('takes its id, version, providers and meta from its package', async () => {
    const lib = await libraryOf(NODE_GREETER);
    expect(await libraryOf(pathToFileURL(NODE_GREETER))).toBe(lib);
    expect([lib.id, lib.version]).toEqual(['node_greeter', '1.0.0']);
    expect(lib.providers.map((p) => p.name)).toEqual(['Hello', 'Shouter']);
    expect(lib.kinds).toEqual(['greeter', 'counter']);
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(lib);
    expect(reg.libraries().map((l) => [l.id, l.version])).toEqual([['node_greeter', '1.0.0']]);
    expect(lib.meta).toEqual({
      summary: 'Greeters written in TypeScript, served to any host in the process',
      license: 'MPL-2.0',
      authors: ['jose-pr'],
    });
  });

  it('names a module that is not its package entry by its path, and reads its licence', async () => {
    const dir = scratch('module-library');
    mkdirSync(path.join(dir, 'lib'));
    writeFileSync(
      path.join(dir, 'package.json'),
      JSON.stringify({ name: 'tally', version: '2.1.0', type: 'module', main: 'index.js', repository: { url: 'https://example.org/tally' } }),
    );
    writeFileSync(path.join(dir, 'LICENSE'), 'Use it well.\n');
    const kinds = pathToFileURL(path.join(REPO, 'bindings', 'node', 'guatiao', 'src', 'provide.ts')).href;
    const counter = pathToFileURL(path.join(REPO, 'examples', 'greeter_kind', 'node', 'greeter_kind.js')).href;
    writeFileSync(
      path.join(dir, 'lib', 'extra.ts'),
      [
        `import { Provider } from '${kinds}';`,
        `import { Counter } from '${counter}';`,
        'export class Tally extends Provider {',
        '  static kinds = [Counter];',
        '  count() { return 3n; }',
        '}',
        'export const notAProvider = 1;',
      ].join('\n'),
    );
    const lib = await libraryOf(path.join(dir, 'lib', 'extra.ts'));
    expect(lib.id).toBe('tally/lib/extra');
    expect(lib.version).toBe('2.1.0');
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(lib);
    const meta = lib.meta!;
    expect(meta.urls).toEqual({ Repository: 'https://example.org/tally' });
    expect(meta.notices).toEqual([{ component: 'tally', text: 'Use it well.\n', format: 'text' }]);
    expect(reg.bind('tally/lib/extra_tally', Counter).count()).toBe(3n);
  });

  it('refuses a module outside a package, or exporting no provider', async () => {
    const dir = scratch('no-provider');
    writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'empty', version: '1.0.0', type: 'module' }));
    writeFileSync(path.join(dir, 'index.ts'), 'export const x = 1;\n');
    await expect(libraryOf(path.join(dir, 'index.ts'))).rejects.toThrow(/exports no Provider/);
    writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'empty', type: 'module' }));
    writeFileSync(path.join(dir, 'again.ts'), 'export const x = 1;\n');
    await expect(libraryOf(path.join(dir, 'again.ts'))).rejects.toThrow(/version/);
  });
});

describe('a provider class is checked', () => {
  it('refuses what it cannot serve', () => {
    class Silent extends Provider {
      static override readonly kinds = [Greeter];
    }
    expect(() => library('x', '1', { providers: [Silent] })).toThrow(/does not implement greet/);
    class Kindless extends Provider {}
    expect(() => library('x', '1', { providers: [Kindless] })).toThrow(/no kind/);
    class Ear extends Provider {
      static override readonly kinds = [Listener as never];
      heard(): void {}
    }
    expect(() => library('x', '1', { providers: [Ear] })).toThrow(/object kind/);
    class Untyped extends Provider {
      static override readonly kinds = [Counter];
      static override readonly config = { type: 'object' } as never;
      count(): bigint {
        return 0n;
      }
    }
    expect(() => library('x', '1', { providers: [Untyped] })).toThrow(/configSchema/);
    class Again extends Provider {
      static override readonly kinds = [Counter];
      static override readonly named = 'plain';
      count(): bigint {
        return 2n;
      }
    }
    expect(() => library('dup', '1', { providers: [Plain, Again] })).toThrow(/share an id/);
    expect(() => library('x', '1', { providers: [class {} as never] })).toThrow(/not a class extending Provider/);
  });

  it('writes a configuration schema from s', () => {
    expect(Limits.json).toEqual({
      type: 'object',
      properties: {
        low: { type: 'integer' },
        high: { type: 'integer', minimum: 0, default: 10 },
        label: { anyOf: [{ type: 'string' }, { type: 'null' }] },
      },
      additionalProperties: false,
      required: ['low'],
    });
    expect(Limits.build({ low: 1 })).toEqual({ low: 1, high: 10 });
  });
});

describe('a library built here', () => {
  it('builds an instance per configuration the host checked', () => {
    const lib = library('node_bounded', '0.1.0', { providers: [Bounded] });
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(lib);
    expect(reg.providerConfig('node_bounded_bounded')!.toJS<{ required: string[] }>().required).toEqual(['low']);
    expect(() => reg.create('node_bounded_bounded', { low: 'one' })).toThrow(BadValue);
    expect(lib.liveInstances()).toBe(0);
    const instance = reg.create('node_bounded_bounded', { low: 8, high: 9 });
    const counter = instance.bind(Counter);
    expect([counter.count(), counter.count()]).toEqual([9n, 9n]);
    expect(lib.liveInstances()).toBe(1);
    expect(() => reg.unload('node_bounded')).toThrow(WrongKind);
    instance.close();
    expect(lib.liveInstances()).toBe(0);
  });

  it('builds nothing when create has nowhere to put the instance', () => {
    const lib = library('node_nowhere', '0.1.0', { providers: [Bounded] });
    const info = lib.infoAddress;
    const provider = n.ptrAt(info, LIBRARY_INFO.providers);
    const create = n.koffi().decode(
      n.ptrAt(provider, PROVIDER_INFO.create),
      n.koffi().proto('uint32_t', ['void *', 'void *', 'void *', 'void *']),
    ) as (ctx: bigint | null, config: bigint, out: null, err: bigint) => number;
    using config = new Value({ low: 1 });
    const err = n.alloc(SIZE.providerError);
    expect(create(null, config.requireNode(), null, err)).toBe(Status.ERR_NULL);
    n.check(Status.OK, err);
    n.release(err);
    expect(lib.liveInstances()).toBe(0);
    lib.close();
  });

  it('answers busy to unload while an instance lives, through its own slot', () => {
    const lib = library('node_busy', '0.1.0', { providers: [Bounded] });
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(lib);
    const unload = n.koffi().decode(n.ptrAt(lib.infoAddress, LIBRARY_INFO.unload), n.koffi().proto('uint32_t', [])) as () => number;
    expect(unload()).toBe(Status.OK);
    const instance = reg.create('node_busy_bounded', { low: 1 });
    expect(unload()).toBe(Status.ERR_BUSY);
    instance.close();
    expect(unload()).toBe(Status.OK);
  });

  it('is retired, not unloaded, and declines a host once closed', () => {
    const before = liveObjects();
    const lib = library('node_short_lived', '0.1.0', { providers: [Plain] });
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(lib);
    expect(liveObjects()).toBe(before + 1);
    expect(() => reg.unload('node_short_lived')).toThrow(WrongKind);
    reg.retire('node_short_lived');
    expect(reg.providers('counter').map((p) => p.id)).not.toContain('node_short_lived_plain');
    lib.close();
    lib.close();
    expect(liveObjects()).toBe(before);
    expect(reg.registerLibrary(lib)).toEqual({ skipped: 'declined-this-host' });
  });

  it('says why a provider cannot run here', () => {
    const reasons: (string | undefined)[] = [undefined, 'the device is gone'];
    class Flaky extends Provider implements Counter {
      static override readonly kinds = [Counter];
      static override available(): string | undefined {
        return reasons.shift();
      }
      count(): bigint {
        return 0n;
      }
    }
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(library('node_flaky', '0.1.0', { providers: [Flaky] }));
    expect(reg.providerAvailable('node_flaky_flaky')).toEqual({ available: true });
    expect(reg.providerAvailable('node_flaky_flaky')).toEqual({ available: false, reason: 'the device is gone' });
  });

  it('disposes an instance its host destroys', () => {
    const disposed: string[] = [];
    class Tidy extends Provider implements Counter {
      static override readonly kinds = [Counter];
      static override readonly config = s.object({ name: s.string() });
      readonly #name: string;
      constructor(config: { name: string }) {
        super();
        this.#name = config.name;
      }
      count(): bigint {
        return 1n;
      }
      [Symbol.dispose](): void {
        disposed.push(this.#name);
      }
    }
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(library('node_tidy', '0.1.0', { providers: [Tidy] }));
    reg.create('node_tidy_tidy', { name: 'a' }).close();
    expect(disposed).toEqual(['a']);
  });

  it('refuses to describe a kind inside a call a library is making', () => {
    class Maker extends Provider implements Counter {
      static override readonly kinds = [Counter];
      count(): bigint {
        kind('made_late', { floorHash: 1, methods: { x: { answer: t.bool } } });
        return 0n;
      }
    }
    using reg = new Registry('node-provide', '0.1');
    reg.registerLibrary(library('node_maker', '0.1.0', { providers: [Maker] }));
    // `count` cannot fail, so the failure crosses as its status alone.
    expect(() => reg.bind('node_maker_maker', Counter).count()).toThrow(Internal);
  });
});
