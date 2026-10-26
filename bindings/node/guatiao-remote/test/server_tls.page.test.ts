// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Server TLS from a page: `wss://` whose certificate the browser judges,
// this side proving the key inside the link. Chrome alone runs it.

import { expect, inject, test } from 'vitest';

import { BadValue } from '../src/errors.js';
import { RemoteLink, RemoteTrust } from '../src/link.js';

const url = inject('serverTlsUrl');
const key = inject('remoteKey');

test('server TLS over wss: the server by its certificate, the page by the key', async () => {
  expect(url.startsWith('wss://')).toBe(true);
  const link = await RemoteLink.connect(url, { security: 'server-tls', key });
  expect(link.trust).toBe(RemoteTrust.SERVER_TLS);
  const catalog = await link.list();
  const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
  await link.open(library);
  const greeter = library.providers[0]!.table('greeter')!;
  expect(await greeter.call('greet', ['tls'])).toEqual({ greeting: 'hello, tls' });
  link.close();
  await expect(RemoteLink.connect(url, { security: 'server-tls', key: 'not the key' })).rejects.toBeInstanceOf(
    BadValue,
  );
});
