// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// What the suites share, in Node and in a page alike: the checkout's
// fixtures arrive as text through Vite, never through the file system.

import kindsText from '../../../../examples/greeter_kind/include/greeter_kind.kinds.json?raw';
import vectorsText from '../../../../crates/guatiao-source-remote/tests/vectors/remote.json?raw';
import wireText from '../../../../crates/guatiao/tests/vectors/wire.json?raw';

import { RemoteKind } from '../src/described.js';

export function hex(text: string): Uint8Array {
  const out = new Uint8Array(text.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(text.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function toHex(bytes: Uint8Array): string {
  return [...bytes].map((b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * The file's text with every integer of 16 digits or more quoted:
 * `JSON.parse` keeps no integer past 2^53 exact, and the vectors hold the
 * 64-bit limits.
 */
function quoteWide(json: string): string {
  return json.replace(/(?<=[[,]\s*)(-?\d{16,})(?=\s*[\],])/g, '"$1"');
}

/** The shared vectors, wide integers as text. */
export const vectors = JSON.parse(quoteWide(vectorsText)) as Record<string, unknown>;

/** The wire encoding's shared vectors (a number is its text there). */
export const wireVectors = JSON.parse(wireText) as Record<string, unknown>;

/** The greeter kinds' description document. */
export const kindsDocument = JSON.parse(kindsText) as { kinds: unknown[] };

/** The greeter kinds, by name. */
export const kinds: ReadonlyMap<string, RemoteKind> = new Map(
  kindsDocument.kinds.map((k) => {
    const kind = RemoteKind.fromDescription(k);
    return [kind.name, kind] as const;
  }),
);
