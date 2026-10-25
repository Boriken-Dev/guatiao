// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Writes the kinds echo_kind.js defines as a description,
// echo_kind.kinds.json: the document `kindsOf()` loads, and that Dart's
// `dart run guatiao:kind` and the other generators write code from.
//
//     node describe.js [--check]
//
// `--check` writes nothing and fails when the file is not what it would write.

import { readFileSync, writeFileSync } from 'node:fs';

import { describeKinds } from 'guatiao/kinds';

import { Echo } from './echo_kind.js';

const file = new URL('./echo_kind.kinds.json', import.meta.url);
const text = `${JSON.stringify(describeKinds([Echo]), null, 2)}\n`;
if (process.argv.includes('--check')) {
  let committed = '';
  try {
    committed = readFileSync(file, 'utf8');
  } catch {
    // A missing file is stale too.
  }
  if (committed !== text) {
    console.error('echo_kind.kinds.json is not what echo_kind.js describes: run node describe.js');
    process.exitCode = 1;
  }
} else {
  writeFileSync(file, text);
}
