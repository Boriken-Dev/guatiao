#!/usr/bin/env node
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * `guatiao-kind DOCUMENT [--out BASE] [--from MODULE] [--check]`: writes
 * `BASE.js` and `BASE.d.ts` for a document of kinds (the
 * `<crate>.kinds.json` a kind crate commits beside its C header). `BASE`
 * defaults to the document's own path without `.kinds.json`; `--from`
 * names the guatiao package as the module imports it (default
 * `guatiao`); `--check` writes nothing and fails when the files are not
 * what it would write.
 *
 * @module
 */

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';

import { generate } from './generate.js';
import type { KindsDocument } from './kinds.js';

function main(argv: string[]): number {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      out: { type: 'string' },
      from: { type: 'string' },
      check: { type: 'boolean', default: false },
    },
  });
  if (positionals.length !== 1) {
    console.error('usage: guatiao-kind DOCUMENT [--out BASE] [--from MODULE] [--check]');
    return 2;
  }
  const document = path.resolve(positionals[0]!);
  const base = path.resolve(values.out ?? document.replace(/(\.kinds)?\.json$/i, ''));
  const relative = (to: string): string => path.relative(path.dirname(base), to).split(path.sep).join('/') || '.';
  const source = relative(document);
  const command = [
    'guatiao-kind',
    source,
    '--out',
    path.basename(base),
    ...(values.from !== undefined ? ['--from', values.from] : []),
  ].join(' ');
  let generated;
  try {
    const parsed = JSON.parse(readFileSync(document, 'utf8')) as KindsDocument;
    generated = generate(parsed, { source, command, ...(values.from !== undefined ? { from: values.from } : {}) });
  } catch (e) {
    console.error(`guatiao-kind: ${document}: ${(e as Error).message}`);
    return 1;
  }
  const files: [string, string][] = [
    [`${base}.js`, generated.js],
    [`${base}.d.ts`, generated.dts],
  ];
  if (values.check) {
    const stale = files.filter(([file, text]) => !existsSync(file) || readFileSync(file, 'utf8') !== text).map(([file]) => file);
    if (stale.length) {
      console.error(`guatiao-kind: not what ${source} writes, run \`${command}\`: ${stale.join(', ')}`);
      return 1;
    }
    return 0;
  }
  for (const [file, text] of files) writeFileSync(file, text);
  return 0;
}

process.exitCode = main(process.argv.slice(2));
