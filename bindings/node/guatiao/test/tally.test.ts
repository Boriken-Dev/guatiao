// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A kind that exists only in TypeScript, written as interfaces: the files
// written from them are current, and a provider written in Node serves it.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { BadValue, Registry, type Value } from '../src/index.js';
import { describeInterfaces } from '../src/describe.js';
import { generate } from '../src/generate.js';
import { type KindsDocument, describeKinds, kindsOf } from '../src/kinds.js';
import { Provider, library } from '../src/provide.js';
import * as tally from '../../../../examples/tally_kind/node/tally_kind.js';
import { REPO, readText } from './helpers.js';

const TALLY = path.join(REPO, 'examples', 'tally_kind');
const TALLY_DOCUMENT = path.join(TALLY, 'include', 'tally_kind.kinds.json');

describe('the tally kind', () => {
  const command =
    'guatiao-kind ../tally_kind.ts --out tally_kind --document ../include/tally_kind.kinds.json --project ../tsconfig.json';

  it('is what its interfaces write', async () => {
    const written = await describeInterfaces(path.join(TALLY, 'tally_kind.ts'), { project: path.join(TALLY, 'tsconfig.json') });
    expect(`${JSON.stringify(written, null, 2)}\n`).toBe(readText(TALLY_DOCUMENT));
    const fresh = generate(written, { source: '../tally_kind.ts', command });
    expect(readText(path.join(TALLY, 'node', 'tally_kind.js'))).toBe(fresh.js);
    expect(readText(path.join(TALLY, 'node', 'tally_kind.d.ts'))).toBe(fresh.dts);
  });

  it('is checked by the command it names', (ctx) => {
    const cli = path.join(REPO, 'bindings', 'node', 'guatiao', 'dist', 'kind_cli.js');
    if (!existsSync(cli)) return ctx.skip('the package is not built: npm run build');
    const run = (...args: string[]): string =>
      execFileSync(process.execPath, [cli, ...args], { cwd: path.join(TALLY, 'node'), encoding: 'utf8' });
    expect(run(...command.split(' ').slice(1), '--check')).toBe('');
    expect(() => run('../tally_kind.ts', '--out', 'elsewhere', '--project', '../tsconfig.json', '--check')).toThrow(/not what/);
    expect(() => run('../tally_kind.ts')).toThrow(/say where with --out/);
  });

  class Counts extends Provider implements tally.Tally {
    static override readonly kinds = [tally.Tally];
    readonly #counts = new Map<string, bigint>();
    add(name: string, by: number): bigint {
      if (!name) throw new BadValue('a count has a name');
      const total = (this.#counts.get(name) ?? 0n) + BigInt(by);
      this.#counts.set(name, total);
      return total;
    }
    total(name: string): bigint {
      return this.#counts.get(name) ?? 0n;
    }
    all(): Record<string, bigint> {
      return Object.fromEntries(this.#counts);
    }
    known(name: string): boolean {
      return this.#counts.has(name);
    }
    checksum(data: Uint8Array): number {
      return data.reduce((sum, byte) => (sum + byte) >>> 0, 0);
    }
    report(watcher: tally.WatcherRemote): void {
      for (const [name, total] of this.#counts) watcher.changed(name, total);
    }
    load(counts: Value): number {
      using owned = counts;
      let held = 0;
      for (const [name, ref] of owned.asMap()) {
        this.#counts.set(name, (this.#counts.get(name) ?? 0n) + ref.intOr(0n));
        held += 1;
      }
      return held;
    }
  }

  it('is served from Node, every slot', () => {
    using reg = new Registry('tally', '0.1');
    reg.registerLibrary(library('counts', '1.0', { providers: [Counts] }));
    const counts = reg.bind('counts_counts', tally.Tally);
    expect(counts.kind.floorHash).toBe(2688319391);
    expect(counts.add('a', 2)).toBe(2n);
    expect(counts.add('a', 3)).toBe(5n);
    expect(counts.total('a')).toBe(5n);
    expect(counts.total('zzz')).toBe(0n);
    expect(() => counts.add('', 1)).toThrow(/a count has a name/);
    expect(counts.known('a')).toBe(true);
    expect(counts.known('zzz')).toBe(false);
    expect(counts.checksum(Uint8Array.of(1, 2, 3, 250))).toBe(256);
    using all = counts.all();
    expect(all.toJS({ numbers: 'bigint' })).toEqual({ a: 5n });
    expect(counts.load({ a: 1, b: 7 })).toBe(2);
    const heard: [string, bigint][] = [];
    counts.report({ changed: (name: string, total: bigint) => void heard.push([name, total]) });
    expect(heard).toEqual([
      ['a', 6n],
      ['b', 7n],
    ]);
  });

  it('is described again by the runtime that loaded it', () => {
    const document = JSON.parse(readText(TALLY_DOCUMENT)) as KindsDocument;
    expect(describeKinds(Object.values(kindsOf(document)))).toEqual(document);
    // The generated module loads the description without its docs.
    const bare = (doc: KindsDocument): unknown => JSON.parse(JSON.stringify(doc, (key, value) => (key === 'doc' ? undefined : value)));
    expect(bare(describeKinds([tally.Tally]))).toEqual(bare({ ...document, kinds: [document.kinds[1]!, document.kinds[0]!] }));
  });
});
