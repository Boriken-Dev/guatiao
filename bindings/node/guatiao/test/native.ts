// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Finds the builds the suite drives: the C libraries built for this run,
// the Rust caller and the guatiao-py bridge.

import { existsSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { inject } from 'vitest';

import { libraryFilename } from '../src/native.js';
import type { CLibraries } from './c_build.js';
import { REPO, libraryDir } from './setup.js';

/** The C libraries `global_setup.ts` built for this run, or `null` without a C compiler. */
export function cLibraries(): CLibraries | null {
  return inject('cLibraries');
}

/** A fresh directory under the system's temporary one. */
export function scratch(name: string): string {
  return mkdtempSync(path.join(tmpdir(), `guatiao-node-${name}-`));
}

/** The workspace build of `base`, beside the guatiao library. */
export function workspaceLibrary(base: string): string {
  return path.join(libraryDir(), libraryFilename(base));
}

/** The guatiao-py bridge: `GUATIAO_PY_BRIDGE`, else its debug build, else `null`. */
export function bridge(): string | null {
  const named =
    process.env.GUATIAO_PY_BRIDGE ?? path.join(REPO, 'crates', 'guatiao-py', 'target', 'debug', libraryFilename('guatiao_py'));
  return existsSync(named) ? named : null;
}

/** What the bridge is told: where the Python greeters are, and what they import. */
export function bridgeConfig(): Record<string, string[]> {
  return {
    dirs: [path.join(REPO, 'examples', 'python_provider')],
    paths: [path.join(REPO, 'bindings', 'python', 'guatiao', 'src'), path.join(REPO, 'examples', 'greeter_kind', 'python')],
  };
}
