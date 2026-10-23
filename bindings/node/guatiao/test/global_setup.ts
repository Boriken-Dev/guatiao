// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Builds the suite's C libraries once, before any test file runs, so no
// test's budget carries a compile: the first C compile on a fresh Windows
// runner can take a minute, and a synchronous one also stops the event loop.
// The directory stays in the system's temporary one, as the suite's other
// scratch directories do: on Windows a library a worker loaded can still be
// mapped when the run ends, and deleting it fails.

import type { TestProject } from 'vitest/node';

import { buildCLibraries, type CLibraries } from './c_build.js';

declare module 'vitest' {
  export interface ProvidedContext {
    cLibraries: CLibraries | null;
  }
}

export default function setup(project: TestProject): void {
  project.provide('cLibraries', buildCLibraries());
}
