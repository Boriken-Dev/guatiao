// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The example host runs as its README says, against greeters written in
// Rust, C, TypeScript and, through the bridge, Python.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { describeKinds, kindsOf } from '../src/kinds.js';
import { Echo, Page } from '../../../../examples/node_js/echo_kind.js';
import { REPO, readText } from './helpers.js';
import { libraryDir } from './setup.js';
import { PYTHON_FREES_HERE, bridge, bridgeConfig, cLibraries } from './native.js';

const EXAMPLES = path.join(REPO, 'examples');

describe('examples/node_host', () => {
  it('calls every greeter it finds and closes what it opened', (ctx) => {
    if (!existsSync(path.join(EXAMPLES, 'node_modules', 'guatiao', 'dist', 'index.js'))) {
      return ctx.skip('the examples are not installed: npm run build here, then npm install in examples/');
    }
    const args = [path.join(EXAMPLES, 'node_host', 'host.ts'), libraryDir()];
    const cDir = cLibraries()?.greeterDir;
    if (cDir !== undefined) args.push(cDir);
    args.push('--node-library', path.join(EXAMPLES, 'node_provider', 'node_greeter', 'greeters.ts'));
    args.push('--node-library', path.join(EXAMPLES, 'node_js', 'greeters.js'));
    const py = bridge();
    if (py !== null) {
      const config = bridgeConfig();
      args.push('--python-source', py, ...config.dirs!.flatMap((d) => ['--python-dir', d]));
      args.push(...config.paths!.flatMap((p) => ['--python-path', p]));
    }
    const out = execFileSync(process.execPath, args, { cwd: REPO, encoding: 'utf8', env: process.env });
    expect(out).toContain('registered node_greeter 1.0.0 from Node');
    expect(out).toContain('registered js_greeter 1.0.0 from Node');
    expect(out).toMatch(/js_greeter_hello[\s\S]*greet: \{"greeting":"hello from JS, ana"\}/);
    expect(out).toMatch(/derived_greeter_hello[\s\S]*greet: \{"greeting":"hello, ana"\}/);
    expect(out).toMatch(/node_greeter_hello[\s\S]*conversation: 2 turns, transcript "hi\\nagain"/);
    expect(out).toMatch(/node_greeter_shouter[\s\S]*instance from \{"prefix":"hey"\}/);
    expect(out).toContain('not this greeter kind');
    if (cDir !== undefined) expect(out).toMatch(/c_greeter[\s\S]*greet: \{"greeting":"hello from C, ana"\}/);
    if (py !== null) {
      expect(out).toContain('served py_greeter from Python');
      expect(out).toMatch(/py_greeter_hello[\s\S]*greetInto: the lent listener heard \["hello, dee"\]/);
    }
    // On Linux and macOS, CPython in this process keeps what a closed
    // Python conversation held (see PYTHON_FREES_HERE).
    if (py === null || PYTHON_FREES_HERE) expect(out).toContain('objects a library can still reach: 0');
    else expect(out).toMatch(/objects a library can still reach: \d+/);
  });
});

describe('examples/node_js', () => {
  const installed = (): boolean => existsSync(path.join(EXAMPLES, 'node_modules', 'guatiao', 'dist', 'index.js'));

  it('runs as plain JavaScript, with no build step', (ctx) => {
    if (!installed()) return ctx.skip('the examples are not installed: npm run build here, then npm install in examples/');
    const out = execFileSync(process.execPath, [path.join(EXAMPLES, 'node_js', 'main.js'), libraryDir()], {
      cwd: REPO,
      encoding: 'utf8',
      env: process.env,
    });
    expect(out).toContain('registered js_greeter 1.0.0');
    expect(out).toContain('derived_greeter_hello: {"greeting":"hello, ana"}');
    expect(out).toContain('js_greeter_hello: {"greeting":"hello from JS, ana"}');
    expect(out).toContain('js_greeter_hello: refused, BadValue: nobody to greet');
    expect(out).toContain('echo: hi, hi');
    expect(out).toContain('the lent page: ["ho","ho, ho"]');
  });

  it('writes the kinds it defines as the description the other languages read', (ctx) => {
    if (!installed()) return ctx.skip('the examples are not installed: npm run build here, then npm install in examples/');
    // Plain JavaScript, as the README says; fails when the committed file is stale.
    execFileSync(process.execPath, [path.join(EXAMPLES, 'node_js', 'describe.js'), '--check'], {
      cwd: REPO,
      encoding: 'utf8',
      env: process.env,
    });
    const text = readText(path.join(EXAMPLES, 'node_js', 'echo_kind.kinds.json'));
    expect(describeKinds([Echo])).toEqual(JSON.parse(text));
    const loaded = kindsOf(text);
    expect([loaded.echo?.floorHash, loaded.page?.floorHash]).toEqual([Echo.floorHash, Page.floorHash]);
    expect(Object.keys(loaded.echo?.methods ?? {})).toEqual(['echo', 'copyOnto']);
    expect(describeKinds(Object.values(loaded))).toEqual(JSON.parse(text));
  });

  it('refuses a plain JavaScript provider missing a required method, naming it', (ctx) => {
    if (!installed()) return ctx.skip('the examples are not installed: npm run build here, then npm install in examples/');
    const script = [
      "import { Provider, library } from 'guatiao/provide';",
      "import { Greeter } from './greeter_kind/node/greeter_kind.js';",
      'class Silent extends Provider { static kinds = [Greeter]; shout(name) { return name; } }',
      "try { library('silent', '1', { providers: [Silent] }); } catch (e) { console.log(`${e.constructor.name}: ${e.message}`); }",
    ].join('\n');
    const out = execFileSync(process.execPath, ['--input-type=module', '-e', script], {
      cwd: EXAMPLES,
      encoding: 'utf8',
      env: process.env,
    });
    expect(out.trim()).toBe('TypeError: Silent does not implement greet, which the greeter kind requires');
  });
});
