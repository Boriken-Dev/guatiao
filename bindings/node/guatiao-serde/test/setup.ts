// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Points the suite at a workspace build when nothing else does, and finds
// the directory of that build when `GUATIAO_LIBRARY` names one file in it.
import { statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..', '..');

process.env.GUATIAO_LIBRARY ??= path.join(REPO, 'target', 'debug');

/**
 * The directory holding the workspace's libraries: `GUATIAO_LIBRARY` when it
 * names one, else the directory of the file it names.
 */
export function libraryDir(): string {
  const named = process.env.GUATIAO_LIBRARY!;
  return statSync(named, { throwIfNoEntry: false })?.isFile() ? path.dirname(named) : named;
}
