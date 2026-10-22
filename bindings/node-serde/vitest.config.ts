// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

const core = fileURLToPath(new URL('../node/src/', import.meta.url));

export default defineConfig({
  resolve: {
    // The core package from its sources beside this one, as its own suite
    // runs it: one module instance for this package and the tests.
    alias: [
      { find: /^guatiao$/, replacement: `${core}index.ts` },
      { find: /^guatiao\/(.*)$/, replacement: `${core}$1.ts` },
    ],
  },
  test: {
    include: ['test/**/*.test.ts'],
    setupFiles: ['test/setup.ts'],
    // A process per file, as in the core: koffi's callbacks belong to the
    // thread that made them.
    pool: 'forks',
    testTimeout: 30_000,
  },
});
