// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

/** The checkout: the shared vectors and the kinds' description live there. */
const repo = fileURLToPath(new URL('../../../', import.meta.url));

/**
 * The Chrome the browser project drives: `CHROME_EXECUTABLE`, else where
 * the platform installs it. Missing, the launch fails naming the variable.
 */
function chrome(): string {
  const named = process.env['CHROME_EXECUTABLE'];
  if (named !== undefined && named !== '') return named;
  const usual =
    process.platform === 'win32'
      ? ['C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe']
      : process.platform === 'darwin'
        ? ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome']
        : ['/usr/bin/google-chrome', '/usr/bin/google-chrome-stable', '/usr/bin/chromium', '/usr/bin/chromium-browser'];
  return usual.find((p) => existsSync(p)) ?? 'no Chrome found: set CHROME_EXECUTABLE';
}

export default defineConfig({
  server: {
    fs: { allow: [repo] },
  },
  test: {
    // guatiao-remote-server, for the live suite of both projects.
    globalSetup: ['test/server.ts'],
    testTimeout: 30_000,
    projects: [
      {
        extends: true,
        // A page's TLS is its browser's: what only a page can do runs in Chrome.
        test: {
          name: 'node',
          include: ['test/**/*.test.ts'],
          exclude: ['test/**/*.page.test.ts'],
          environment: 'node',
        },
      },
      {
        extends: true,
        test: {
          name: 'chrome',
          include: ['test/**/*.test.ts'],
          browser: {
            enabled: true,
            headless: true,
            screenshotFailures: false,
            // The provider's browser context takes any certificate (vitest sets
            // ignoreHTTPSErrors), so the server-TLS test's own one passes.
            provider: playwright({ launchOptions: { executablePath: chrome() } }),
            instances: [{ browser: 'chromium' }],
          },
        },
      },
    ],
  },
});
