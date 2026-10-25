// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Finds the builds the suite drives: the C libraries built for this run,
// the Rust caller and the guatiao-source-py bridge.

import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { inject, onTestFinished } from 'vitest';

import { libraryFilename } from '../src/native.js';
import type { CLibraries } from './c_build.js';
import { REPO, libraryDir } from './setup.js';

/** The C libraries `global_setup.ts` built for this run, or `null` without a C compiler. */
export function cLibraries(): CLibraries | null {
  return inject('cLibraries');
}

/**
 * A fresh directory under the system's temporary one, removed when the
 * test ends. Kept on Windows, which refuses to delete a library a worker
 * still maps.
 */
export function scratch(name: string): string {
  const dir = mkdtempSync(path.join(tmpdir(), `guatiao-node-${name}-`));
  if (process.platform !== 'win32') {
    onTestFinished(() => rmSync(dir, { recursive: true, force: true }));
  }
  return dir;
}

/** The workspace build of `base`, beside the guatiao library. */
export function workspaceLibrary(base: string): string {
  return path.join(libraryDir(), libraryFilename(base));
}

/** The guatiao-source-py bridge: `GUATIAO_PY_BRIDGE`, else its debug build, else `null`. */
export function bridge(): string | null {
  const named =
    process.env.GUATIAO_PY_BRIDGE ?? path.join(REPO, 'crates', 'guatiao-source-py', 'target', 'debug', libraryFilename('guatiao_source_py'));
  return existsSync(named) ? named : null;
}

/**
 * Whether CPython, embedded through the guatiao-source-py bridge in this process,
 * frees what a call from here drops. On macOS with Python 3.14 it does not:
 * koffi runs the call on a stack of its own, CPython judges that stack too
 * deep, and an object dropped during the call is never deallocated (its
 * `__del__` never runs), so what a Python conversation holds outlives it.
 * On Linux the bridge points CPython at the stack the call runs on, and it
 * frees; Windows and FreeBSD free as they are. A Python host and a Rust
 * host do not see this.
 */
export const PYTHON_FREES_HERE = process.platform !== 'darwin';

/** What the bridge is told: where the Python greeters are, and what they import. */
export function bridgeConfig(): Record<string, string[]> {
  return {
    dirs: [path.join(REPO, 'examples', 'python_provider')],
    paths: [path.join(REPO, 'bindings', 'python', 'guatiao', 'src'), path.join(REPO, 'examples', 'greeter_kind', 'python')],
  };
}
