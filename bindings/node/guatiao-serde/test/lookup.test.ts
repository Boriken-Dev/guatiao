// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Where a surface's library is found: its own variable, then
// GUATIAO_LIBRARY as a directory, a file carrying the surface, or a file
// beside it.

import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { LibraryNotFound, libraryFilename, openSurface } from 'guatiao/ffi';

import { libraryDir } from './setup.js';

const NAMED = process.env.GUATIAO_LIBRARY!;
const DIR = libraryDir();
const file = (basename: string): string => path.join(DIR, libraryFilename(basename));
const found = (): string =>
  openSurface('guatiao_serde', [['uint32_t guatiao_alloc_default(void *out)']], {
    variable: 'GUATIAO_SERDE_LIBRARY',
    probe: 'guatiao_json_parse_ex',
  }).path;

describe('the serde library is found', () => {
  afterEach(() => {
    delete process.env.GUATIAO_SERDE_LIBRARY;
    process.env.GUATIAO_LIBRARY = NAMED;
  });

  it('through its own variable naming a file, whatever it is called', () => {
    process.env.GUATIAO_SERDE_LIBRARY = file('guatiao_intake');
    expect(found()).toBe(file('guatiao_intake'));
  });

  it('through its own variable naming a directory', () => {
    process.env.GUATIAO_SERDE_LIBRARY = DIR;
    process.env.GUATIAO_LIBRARY = path.join(DIR, 'nowhere');
    expect(found()).toBe(file('guatiao_serde'));
  });

  it('in a GUATIAO_LIBRARY file that carries it', () => {
    process.env.GUATIAO_LIBRARY = file('guatiao_serde');
    expect(found()).toBe(file('guatiao_serde'));
  });

  it('beside a GUATIAO_LIBRARY file that does not carry it', () => {
    process.env.GUATIAO_LIBRARY = file('guatiao');
    expect(found()).toBe(file('guatiao_serde'));
  });

  it('or not at all, naming every place it looked', () => {
    process.env.GUATIAO_SERDE_LIBRARY = path.join(DIR, 'nowhere');
    process.env.GUATIAO_LIBRARY = path.join(DIR, 'nowhere');
    let caught: unknown;
    try {
      openSurface('not_a_real_guatiao_library', [], { variable: 'GUATIAO_SERDE_LIBRARY', probe: 'x' });
    } catch (e) {
      caught = e;
    }
    expect(caught).toBeInstanceOf(LibraryNotFound);
    expect(String(caught)).toContain('GUATIAO_SERDE_LIBRARY');
    expect(String(caught)).toContain('GUATIAO_LIBRARY');
  });
});
