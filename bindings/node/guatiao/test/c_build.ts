// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Builds the C libraries the suite calls, each into a directory of its own.
// `global_setup.ts` runs it once per run, before any test file starts.

import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync } from 'node:fs';
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

/** The first C compiler on this machine, as the Rust tests look for one. */
function compiler(): string | null {
  return CANDIDATES.find(works) ?? null;
}

/**
 * `source` (a path) built with `cc` into `into` as the library `name`. A
 * compile error fails loudly: the header changed under it.
 */
function buildC(cc: string, source: string, into: string, name: string, defines: readonly string[] = []): string {
  mkdirSync(into, { recursive: true });
  const out = path.join(into, libraryFilename(name));
  const args = ['-shared', '-std=c11', '-Wall', '-Wextra', '-Werror', ...defines.map((d) => `-D${d}`)];
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

/** Where each C library the suite calls was built. */
export interface CLibraries {
  /** The directory holding the greeter written in C, and nothing else. */
  greeterDir: string;
  /** The same greeter built to ignore the carrier (its answers are built in its own allocator), in a directory of its own. */
  carelessGreeterDir: string;
  /** The stock greeter again, in a directory of its own: a test unloads it, so nothing else may keep it mapped. */
  lentGreeterDir: string;
  /** Two builds of the source written in C (a shelf of counters), one per test that changes its state. */
  shelfA: string;
  shelfB: string;
  /** `test/fixtures/foreign_thread.c`, which calls a slot from a thread of its own. */
  foreignThread: string;
}

/** Every C library the suite calls, or `null` without a C compiler. */
export function buildCLibraries(): CLibraries | null {
  const cc = compiler();
  if (cc === null) return null;
  const root = mkdtempSync(path.join(tmpdir(), 'guatiao-node-c-'));
  const C_CONSUMER = path.join(REPO, 'crates', 'guatiao', 'tests', 'c_consumer');
  const greeterDir = path.join(root, 'greeter');
  buildC(cc, path.join(C_CONSUMER, 'greeter_in_c.c'), greeterDir, 'c_greeter');
  const carelessGreeterDir = path.join(root, 'careless');
  buildC(cc, path.join(C_CONSUMER, 'greeter_in_c.c'), carelessGreeterDir, 'c_greeter', ['GREETER_IGNORES_CARRIER']);
  const lentGreeterDir = path.join(root, 'lent');
  buildC(cc, path.join(C_CONSUMER, 'greeter_in_c.c'), lentGreeterDir, 'c_greeter');
  return {
    greeterDir,
    carelessGreeterDir,
    lentGreeterDir,
    shelfA: buildC(cc, path.join(C_CONSUMER, 'source_in_c.c'), path.join(root, 'shelf_a'), 'node_shelf_a'),
    shelfB: buildC(cc, path.join(C_CONSUMER, 'source_in_c.c'), path.join(root, 'shelf_b'), 'node_shelf_b'),
    foreignThread: buildC(
      cc,
      path.join(REPO, 'bindings', 'node', 'guatiao', 'test', 'fixtures', 'foreign_thread.c'),
      path.join(root, 'foreign'),
      'foreign_thread',
    ),
  };
}
