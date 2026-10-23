// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Registry against a real --all-features build: scanning, answers,
// instances, retire, collect and unload, and sources.

import { copyFileSync, mkdirSync, renameSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { BadValue, Busy, NotFound, Registry, Tag, WrongKind } from '../src/index.js';
import * as n from '../src/native.js';
import { Counter } from '../../../../examples/greeter_kind/node/greeter_kind.js';
import { libraryName } from './helpers.js';
import { cLibraries, scratch, workspaceLibrary } from './native.js';
import { libraryDir } from './setup.js';

const TARGET = libraryDir();

function scanned(): Registry {
  const reg = new Registry('node-tests', '0.1');
  reg.scanDir(TARGET, { rules: 'kind=greeter' });
  return reg;
}

const ids = (list: Record<string, unknown>[]): unknown[] => list.map((p) => p.id);

describe('Registry', () => {
  it('scans by rule and lists the providers', () => {
    using reg = new Registry('node-tests', '0.1');
    const report = reg.scanDir(TARGET, { rules: 'kind=greeter' }) as { loaded: string[] };
    expect(report.loaded.some((p) => p.includes('derived_greeter'))).toBe(true);
    expect(report.loaded.some((p) => p.includes('hello_library'))).toBe(true);
    expect(ids(reg.providers('greeter'))).toEqual(expect.arrayContaining(['derived_greeter_hello', 'hello_library_greeter']));
  });

  it('answers the documented fields for the best provider', () => {
    using reg = scanned();
    const best = reg.best('greeter');
    for (const field of ['key', 'id', 'version', 'library', 'display_name', 'from', 'kinds', 'has_config', 'vtable_size']) {
      expect(best, field).toHaveProperty(field);
    }
    expect(best.kinds).toContain('greeter');
    expect(reg.whyNot('nothing-claims-this-kind')).toMatchObject({ available: false, why: 'nothing-claims-it' });
    expect(() => reg.provider('nonesuch')).toThrow(NotFound);
  });

  it('answers a single table, and a table per kind', () => {
    using reg = scanned();
    const single = reg.providerTable('hello_library_greeter');
    expect(single.table).not.toBe(0n);
    expect(single.size).toBeGreaterThan(0);
    expect(reg.providerTable('derived_greeter_hello').table).toBe(0n);
    expect(reg.providerTable('derived_greeter_hello', 'greeter').table).not.toBe(0n);
    expect(reg.providerTable('derived_greeter_hello', 'codec').table).toBe(0n);
    expect(() => reg.bind('derived_greeter_hello', Counter).count()).not.toThrow();
  });

  it('builds an instance, and lends its configuration schema', () => {
    using reg = scanned();
    const schema = reg.providerConfig('derived_greeter_shouter');
    expect(schema?.tag).toBe(Tag.MAP);
    expect(() => schema!.asMap().set('x', 1)).toThrow(TypeError);
    expect(reg.providerConfig('derived_greeter_hello')).toBe(undefined);
    const instance = reg.create('derived_greeter_shouter', { prefix: 'hey' });
    expect(instance.ctx).not.toBe(0n);
    instance.close();
    instance.close();
  });

  it("reads a provider's meta, a copy carrying its form", () => {
    using reg = scanned();
    using meta = reg.providerMeta('derived_greeter_shouter')!;
    const data = meta.asMap().get('form')!.toJS() as { sections: { id: string }[] };
    expect(data.sections.map((s) => s.id)).toEqual(['words', 'volume']);
    expect(reg.providerMeta('derived_greeter_hello')).toBe(undefined);
    expect(() => reg.providerMeta('nobody')).toThrow(NotFound);
  });

  it('keys, ranks and re-keys', () => {
    using reg = scanned();
    reg.setPriority('derived_greeter_hello', 7);
    expect(reg.priority('derived_greeter_hello')).toBe(7);
    reg.keyedBy('%id@%version');
    expect(reg.providers('greeter').map((p) => p.key)).toContain('derived_greeter_hello@0.0.0-alpha.1');
    expect(() => reg.keyedBy('%nonsense')).toThrow(BadValue);
  });

  it('retires a library and frees its key', () => {
    using reg = scanned();
    reg.retire('derived_greeter');
    expect(ids(reg.providers('greeter'))).not.toContain('derived_greeter_hello');
    expect(ids(reg.providers('greeter'))).toContain('hello_library_greeter');
    const again = reg.scanDir(TARGET, { rules: 'kind=greeter' }) as { loaded: string[] };
    expect(again.loaded.some((p) => p.includes('derived_greeter'))).toBe(true);
    expect(() => reg.retire('derived_greeter_hello')).toThrow(NotFound);
  });

  it('unloads only when the library agrees', () => {
    using reg = scanned();
    const instance = reg.create('derived_greeter_shouter', { prefix: 'hey' });
    expect(() => reg.unload('derived_greeter')).toThrow(Busy);
    instance.close();
    reg.unload('derived_greeter');
    expect(reg.libraries().map((l) => l.id)).not.toContain('derived_greeter');
    expect(() => reg.unload('derived_greeter')).toThrow(NotFound);
  });

  it('collects a retired library once nothing of it is alive', () => {
    using reg = scanned();
    expect(reg.collect()).toEqual([]);
    const instance = reg.create('derived_greeter_shouter', { prefix: 'hey' });
    reg.retire('derived_greeter');
    expect(reg.collect()).toEqual([]);
    instance.close();
    expect(reg.collect()).toEqual(['derived_greeter']);
    expect(reg.collect()).toEqual([]);
  });

  it('refuses use once closed, and closes twice quietly', () => {
    const reg = new Registry('node-tests', '0.1');
    reg.close();
    expect(() => reg.libraries()).toThrow(TypeError);
    reg.close();
  });
});

describe('sources', () => {
  it('serves libraries that come and go from a source file', (ctx) => {
    const shelf = cLibraries()?.shelfA;
    if (shelf === undefined) return ctx.skip('no C compiler: the source written in C is not built');
    const controls = n.koffi().load(shelf);
    const take = controls.func('void shelf_take(int i)');
    const put = controls.func('void shelf_put(int i)');
    const refuse = controls.func('void shelf_refuse(bool refuse)');
    const closed = controls.func('int shelf_closed(int i)');
    using reg = new Registry('node-sources', '0.1');
    const name = reg.addSourceFile(shelf, { ids: ['shelf_a', 'shelf_b'] });
    expect(name).toBe(shelf);
    expect(reg.libraries()).toEqual([]);
    const first = reg.refresh();
    expect(first.loaded.map((s) => s.id)).toEqual(['shelf_a', 'shelf_b']);
    expect(reg.bind('shelf_a_counter', Counter).count()).toBe(1n);
    expect(reg.refresh()).toEqual({ loaded: [], retired: [], skipped: [], refused: [] });
    take(1);
    expect(reg.refresh().retired.map((s) => s.id)).toEqual(['shelf_b']);
    reg.collect();
    expect(closed(1)).toBe(1);
    refuse(true);
    put(2);
    expect(reg.refresh().refused).toEqual([{ source: name, id: 'shelf_c', why: 'the shelf refuses, says C' }]);
  });

  it('refuses with the source own words, and a file that is no source', (ctx) => {
    const shelf = cLibraries()?.shelfB;
    if (shelf === undefined) return ctx.skip('no C compiler: the source written in C is not built');
    using reg = new Registry('node-sources', '0.1');
    expect(() => reg.addSourceFile(shelf, {})).toThrow(/`ids`/);
    reg.addSourceFile(shelf, { ids: [] });
    expect(() => reg.addSourceFile(shelf, { ids: [] })).toThrow(/already added/);
    expect(() => reg.addSourceFile(workspaceLibrary('derived_greeter'))).toThrow(WrongKind);
    expect(() => reg.addSourceFile(path.join(TARGET, 'no-such-file'))).toThrow(NotFound);
  });

  it('follows the files of a watched directory', () => {
    const dir = scratch('watched');
    const watched = path.join(dir, 'watched');
    const aside = path.join(dir, 'aside');
    mkdirSync(watched);
    mkdirSync(aside);
    using reg = new Registry('node-sources', '0.1');
    reg.watchDir(watched, { rules: 'kind=greeter' });
    expect(() => reg.watchDir(watched)).toThrow(BadValue);
    expect(reg.refresh().loaded).toEqual([]);
    const copy = path.join(watched, libraryName('hello_node_watched'));
    copyFileSync(workspaceLibrary('hello_library'), copy);
    expect(reg.refresh().loaded.map((s) => s.key)).toEqual(['hello_library']);
    // A mapped image cannot be deleted on Windows; moved aside, it is gone.
    renameSync(copy, path.join(aside, path.basename(copy)));
    expect(reg.refresh().retired.map((s) => s.key)).toEqual(['hello_library']);
    expect(reg.libraries()).toEqual([]);
  });
});
