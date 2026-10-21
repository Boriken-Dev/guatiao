// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    setupFiles: ['test/setup.ts'],
    // A process per file: an unload unmaps a library for the whole
    // process, and koffi's callbacks belong to the thread that made them.
    pool: 'forks',
    testTimeout: 30_000,
  },
});
