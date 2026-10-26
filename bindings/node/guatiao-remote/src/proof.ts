// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The trusted level's proof of a pre-shared key: HMAC-SHA256 over a label
 * and both nonces, each side proving it to the other. `@noble/hashes`,
 * not WebCrypto: `crypto.subtle` exists only in a secure context, and a
 * page that may open `ws://` is one only on localhost.
 *
 * @module
 */

import { hmac } from '@noble/hashes/hmac.js';
import { sha256 } from '@noble/hashes/sha2.js';

/** What a server proves with. Protocol bytes, never renamed. */
export const SERVER_LABEL = 'guatiao-remote server';

/** What a client proves with. */
export const CLIENT_LABEL = 'guatiao-remote client';

const utf8 = new TextEncoder();

/** The proof of `key` for `label` over the client's nonce and the server's. */
export function proofOf(key: Uint8Array, label: string, client: Uint8Array, server: Uint8Array): Uint8Array {
  const label8 = utf8.encode(label);
  const message = new Uint8Array(label8.length + client.length + server.length);
  message.set(label8, 0);
  message.set(client, label8.length);
  message.set(server, label8.length + client.length);
  return hmac(sha256, key, message);
}

/** Whether `given` is the proof, compared in time that does not depend on where it first differs. */
export function proves(
  key: Uint8Array,
  label: string,
  client: Uint8Array,
  server: Uint8Array,
  given: Uint8Array,
): boolean {
  const expected = proofOf(key, label, client, server);
  if (given.length !== expected.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) diff |= expected[i]! ^ given[i]!;
  return diff === 0;
}

/** Sixteen bytes from the platform's secure random source (any context, secure or not). */
export function nonce(): Uint8Array {
  return globalThis.crypto.getRandomValues(new Uint8Array(16));
}
