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
 * `guatiao-kind FILE.ts... --out BASE [--document FILE] [--project TSCONFIG]`
 * reads the kinds from TypeScript interfaces instead (the `typescript`
 * package, 7.0 or newer): it writes the document too (`BASE.kinds.json`, or
 * `--document`), and the files are read through the `tsconfig.json` above
 * them, or `--project`.
 *
 * @module
 */

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';

import { DescribeError, describeInterfaces } from './describe.js';
import { generate } from './generate.js';
import type { KindsDocument } from './kinds.js';

const USAGE =
  'usage: guatiao-kind DOCUMENT [--out BASE] [--from MODULE] [--check]\n' +
  '       guatiao-kind FILE.ts... --out BASE [--document FILE] [--project TSCONFIG] [--from MODULE] [--check]';

async function main(argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      out: { type: 'string' },
      from: { type: 'string' },
      project: { type: 'string' },
      document: { type: 'string' },
      check: { type: 'boolean', default: false },
    },
  });
  const sources = positionals.every((p) => /\.[cm]?ts$/i.test(p) && !/\.d\.[cm]?ts$/i.test(p));
  if (positionals.length === 0 || (!sources && positionals.length !== 1)) {
    console.error(USAGE);
    return 2;
  }
  if (sources && values.out === undefined) {
    console.error(`guatiao-kind: ${positionals.join(' ')} is TypeScript, which the kinds' files are written beside: say where with --out BASE\n${USAGE}`);
    return 2;
  }
  if (!sources && (values.project !== undefined || values.document !== undefined)) {
    console.error(`guatiao-kind: --project and --document are for reading TypeScript\n${USAGE}`);
    return 2;
  }
  const given = positionals.map((p) => path.resolve(p));
  const base = path.resolve(values.out ?? given[0]!.replace(/(\.kinds)?\.json$/i, ''));
  const relative = (to: string): string => path.relative(path.dirname(base), to).split(path.sep).join('/') || '.';
  const documentFile = sources ? path.resolve(values.document ?? `${base}.kinds.json`) : given[0]!;
  const source = sources ? given.map(relative).join(' ') : relative(documentFile);
  const command = [
    'guatiao-kind',
    source,
    '--out',
    relative(base),
    ...(values.document !== undefined ? ['--document', relative(documentFile)] : []),
    ...(values.project !== undefined ? ['--project', relative(path.resolve(values.project))] : []),
    ...(values.from !== undefined ? ['--from', values.from] : []),
  ].join(' ');
  let generated;
  let documentText: string | undefined;
  try {
    let parsed: KindsDocument;
    if (sources) {
      parsed = await describeInterfaces(given, values.project === undefined ? {} : { project: values.project });
      documentText = `${JSON.stringify(parsed, null, 2)}\n`;
    } else {
      parsed = JSON.parse(readFileSync(documentFile, 'utf8')) as KindsDocument;
    }
    generated = generate(parsed, {
      source: sources ? source : relative(documentFile),
      command,
      ...(values.from !== undefined ? { from: values.from } : {}),
    });
  } catch (e) {
    console.error(`guatiao-kind: ${e instanceof DescribeError ? '' : `${documentFile}: `}${(e as Error).message}`);
    return 1;
  }
  const files: [string, string][] = [
    ...(documentText === undefined ? [] : ([[documentFile, documentText]] as [string, string][])),
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

process.exitCode = await main(process.argv.slice(2));
