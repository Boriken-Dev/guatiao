// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The example host runs as its README says, against greeters written in
// Rust, C, TypeScript and, through the bridge, Python.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

import { describe, expect, it } from 'vitest';

import { REPO } from './helpers.js';
import { libraryDir } from './setup.js';
import { bridge, bridgeConfig, cLibraries } from './native.js';

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
    const py = bridge();
    if (py !== null) {
      const config = bridgeConfig();
      args.push('--python-source', py, ...config.dirs!.flatMap((d) => ['--python-dir', d]));
      args.push(...config.paths!.flatMap((p) => ['--python-path', p]));
    }
    const out = execFileSync(process.execPath, args, { cwd: REPO, encoding: 'utf8', env: process.env });
    expect(out).toContain('registered node_greeter 1.0.0 from Node');
    expect(out).toMatch(/derived_greeter_hello[\s\S]*greet: \{"greeting":"hello, ana"\}/);
    expect(out).toMatch(/node_greeter_hello[\s\S]*conversation: 2 turns, transcript "hi\\nagain"/);
    expect(out).toMatch(/node_greeter_shouter[\s\S]*instance from \{"prefix":"hey"\}/);
    expect(out).toContain('not this greeter kind');
    if (cDir !== undefined) expect(out).toMatch(/c_greeter[\s\S]*greet: \{"greeting":"hello from C, ana"\}/);
    if (py !== null) {
      expect(out).toContain('served py_greeter from Python');
      expect(out).toMatch(/py_greeter_hello[\s\S]*greetInto: the lent listener heard \["hello, dee"\]/);
    }
    // Off Windows, CPython in this process keeps what a closed Python
    // conversation held (see greeters.test.ts).
    if (py === null || process.platform === 'win32') expect(out).toContain('objects a library can still reach: 0');
    else expect(out).toMatch(/objects a library can still reach: \d+/);
  });
});
