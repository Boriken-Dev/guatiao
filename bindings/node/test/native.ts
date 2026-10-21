// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Builds the repository's C libraries into directories of their own, for
// the tests that call a library written in C, and finds the other builds
// the suite drives: the Rust caller and the guatiao-py bridge.

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { libraryFilename } from '../src/native.js';
import { REPO } from './setup.js';

const CANDIDATES = ['clang', 'C:\\Program Files\\LLVM\\bin\\clang.exe', 'cc', 'gcc'];

function works(cc: string): boolean {
  try {
    execFileSync(cc, ['--version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

let found: string | null | undefined;

/** The first C compiler on this machine, as the Rust tests look for one. */
export function compiler(): string | null {
  if (found === undefined) found = CANDIDATES.find(works) ?? null;
  return found;
}

/** A fresh directory under the system's temporary one. */
export function scratch(name: string): string {
  return mkdtempSync(path.join(tmpdir(), `guatiao-node-${name}-`));
}

/**
 * `source` (a path) built into `into` as the library `name`, or `null`
 * without a compiler. A compile error fails loudly: the header changed
 * under it.
 */
export function buildC(source: string, into: string, name: string): string | null {
  const cc = compiler();
  if (cc === null) return null;
  mkdirSync(into, { recursive: true });
  const out = path.join(into, libraryFilename(name));
  const args = ['-shared', '-std=c11', '-Wall', '-Wextra', '-Werror'];
  if (process.platform !== 'win32') args.push('-fPIC');
  args.push(
    '-I',
    path.join(REPO, 'crates', 'guatiao', 'include'),
    '-I',
    path.join(REPO, 'examples', 'greeter_kind', 'include'),
    '-o',
    out,
    source,
  );
  execFileSync(cc, args, { stdio: ['ignore', 'ignore', 'pipe'] });
  return out;
}

const C_CONSUMER = path.join(REPO, 'crates', 'guatiao', 'tests', 'c_consumer');

/** The greeter written in C, in a directory of its own; `null` without a compiler. */
export function buildCGreeter(): string | null {
  const dir = scratch('c-greeter');
  return buildC(path.join(C_CONSUMER, 'greeter_in_c.c'), dir, 'c_greeter') === null ? null : dir;
}

/** The source written in C (a shelf of counters), as a file; `null` without a compiler. */
export function buildCSource(name: string): string | null {
  return buildC(path.join(C_CONSUMER, 'source_in_c.c'), scratch('c-shelf'), name);
}

/** The workspace build of `base`, beside the guatiao library. */
export function workspaceLibrary(base: string): string {
  return path.join(process.env.GUATIAO_LIBRARY ?? path.join(REPO, 'target', 'debug'), libraryFilename(base));
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
    paths: [path.join(REPO, 'bindings', 'python', 'src'), path.join(REPO, 'examples', 'greeter_kind', 'python')],
  };
}
