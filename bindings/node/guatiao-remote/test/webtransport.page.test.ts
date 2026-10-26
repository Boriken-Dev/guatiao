// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// WebTransport from a page: the server's own certificate pinned by its hash
// (`serverCertificateHashes`), with no certificate authority and no browser
// flag. Chrome alone runs it.

import { expect, inject, test } from 'vitest';

import { BadValue, Gone } from '../src/errors.js';
import { RemoteLink, RemoteTrust } from '../src/link.js';

const key = inject('remoteKey');
const serverTls = inject('wtServerTls');
const none = inject('wtNone');
const mutual = inject('wtMutual');

/** A fingerprint no certificate has. */
const STRANGER = `sha256:${'ab'.repeat(32)}`;

async function greet(link: RemoteLink, name: string): Promise<unknown> {
  const catalog = await link.list();
  const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
  await link.open(library);
  return library.providers[0]!.table('greeter')!.call('greet', [name]);
}

test('server TLS over WebTransport: the certificate pinned by its hash, the page by the key', async () => {
  expect(serverTls.url.startsWith('https://')).toBe(true);
  expect(serverTls.hashes).toHaveLength(2);
  const link = await RemoteLink.connect(serverTls.url, { security: 'server-tls', key, pins: serverTls.hashes });
  expect(link.trust).toBe(RemoteTrust.SERVER_TLS);
  expect(await greet(link, 'webtransport')).toEqual({ greeting: 'hello, webtransport' });
  expect(await link.ping()).toBeGreaterThanOrEqual(0);
  link.close();
  await link.done;
  await expect(
    RemoteLink.connect(serverTls.url, { security: 'server-tls', key: 'not the key', pins: serverTls.hashes }),
  ).rejects.toBeInstanceOf(BadValue);
});

test('a hash the certificate does not have is refused by the browser', async () => {
  await expect(
    RemoteLink.connect(serverTls.url, { security: 'server-tls', key, pins: [STRANGER], handshakeTimeout: 5_000 }),
  ).rejects.toBeInstanceOf(Gone);
});

test('the next certificate alone is not the one presented', async () => {
  await expect(
    RemoteLink.connect(serverTls.url, { security: 'server-tls', key, pins: [serverTls.hashes[1]!], handshakeTimeout: 5_000 }),
  ).rejects.toBeInstanceOf(Gone);
});

test('no security over WebTransport: the key proved both ways inside QUIC', async () => {
  const link = await RemoteLink.connect(none.url, { security: 'none', key, pins: none.hashes });
  expect(link.trust).toBe(RemoteTrust.TRUSTED);
  expect(await greet(link, 'none')).toEqual({ greeting: 'hello, none' });
  link.close();
  await expect(
    RemoteLink.connect(none.url, { security: 'none', key: 'not the key', pins: none.hashes }),
  ).rejects.toBeInstanceOf(BadValue);
});

test('mutual TLS over WebTransport needs a client certificate the browser presents, which this profile holds none of', async () => {
  await expect(
    RemoteLink.connect(mutual.url, { security: 'mutual-tls', pins: mutual.hashes, handshakeTimeout: 5_000 }),
  ).rejects.toBeInstanceOf(Gone);
});

test('pins are refused on a WebSocket URL', async () => {
  await expect(RemoteLink.connect('wss://127.0.0.1:1/', { security: 'server-tls', key, pins: [STRANGER] })).rejects.toThrow(
    /WebTransport/,
  );
});
