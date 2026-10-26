// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A link over WebSocket to guatiao-remote-server on loopback, serving
// `derived_greeter`: in Node, and in Chrome as a page would.

import { expect, inject, test } from 'vitest';

import { BadValue, Gone } from '../src/errors.js';
import { type RemoteProvider, RemoteLink, RemoteObject, Served } from '../src/link.js';

const url = inject('remoteUrl');
const key = inject('remoteKey');

const connect = (withKey: string = key): Promise<RemoteLink> => RemoteLink.connect(url, { security: 'none', key: withKey });

/** A link with `derived_greeter` opened, and its `Hello` provider. */
async function hello(): Promise<[RemoteLink, RemoteProvider]> {
  const link = await connect();
  const catalog = await link.list();
  const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
  await link.open(library);
  return [link, library.providers.find((p) => p.id === 'derived_greeter_hello')!];
}

test('it runs where the project says', () => {
  // The chrome project runs in a page: no Node there.
  const inPage = typeof window !== 'undefined' && typeof document !== 'undefined';
  const inNode = typeof globalThis.process?.versions?.node === 'string';
  expect(inPage !== inNode).toBe(true);
  if (inPage) expect(navigator.userAgent).toContain('Chrome');
});

test('the catalog lists the greeter kinds and their providers', async () => {
  const link = await connect();
  const catalog = await link.list();
  expect([...catalog.kinds.keys()]).toEqual(
    expect.arrayContaining(['greeter', 'listener', 'conversation', 'counter', 'wide']),
  );
  expect(catalog.kinds.get('listener')!.object).toBe(true);
  const library = catalog.libraries.find((l) => l.id === 'derived_greeter')!;
  expect(library.stamp).not.toBe('');
  expect(library.providers.map((p) => p.id)).toContain('derived_greeter_hello');
  expect(typeof (await link.ping())).toBe('number');
  link.close();
});

test('a page greets through the greeter kind', async () => {
  const [link, provider] = await hello();
  const greeter = provider.table('greeter')!;
  expect(await greeter.call('greet', ['web'])).toEqual({ greeting: 'hello, web' });
  await expect(greeter.call('greet', [''])).rejects.toMatchObject({ name: 'BadValue', message: 'nobody to greet' });
  expect(await greeter.call('greet_from', [{ name: 'map' }])).toEqual({ greeting: 'hello, map' });
  const counter = provider.table('counter')!;
  // One counter for every link, the other project's among them.
  const first = (await counter.call('count')) as number;
  expect(await counter.call('count')).toBeGreaterThan(first);
  link.close();
});

test('a lent map comes back written in place', async () => {
  const [link, provider] = await hello();
  const config: Record<string, unknown> = {};
  expect(await provider.table('greeter')!.call('check', [config])).toBe(true);
  expect(config).toEqual({ name: 'world', greeting: 'hello, world' });
  link.close();
});

test('a lent listener is called during the call, and only then', async () => {
  const [link, provider] = await hello();
  const heard: string[] = [];
  const listener = new Served('listener', { heard: (args) => void heard.push(args[0] as string) });
  await provider.table('greeter')!.call('greet_into', ['lent', listener]);
  expect(heard).toEqual(['hello, lent']);
  expect(link.objectsHeld, 'lent for the call alone').toBe(0);
  link.close();
});

test('a handed listener is called back during calls on what it was handed to', async () => {
  const [link, provider] = await hello();
  const heard: string[] = [];
  const listener = new Served('listener', { heard: (args) => void heard.push(args[0] as string) });
  const chat = (await provider.table('greeter')!.call('start', [listener])) as RemoteObject;
  expect(chat).toBeInstanceOf(RemoteObject);
  expect(chat.kind.name).toBe('conversation');
  await chat.call('say', ['hi']);
  expect(heard, 'heard while `say` was open').toEqual(['hi']);
  await chat.call('say', ['there']);
  expect(heard).toEqual(['hi', 'there']);
  expect(await chat.call('turns')).toBe(2);
  const buffer = new Uint8Array(64);
  const n = (await chat.call('read', [buffer])) as number;
  expect(new TextDecoder().decode(buffer.subarray(0, n))).toBe('hi\nthere');
  expect(link.objectsHeld, 'the listener, handed over').toBe(1);
  chat.release();
  await expect(chat.call('turns')).rejects.toBeInstanceOf(Gone);
  // The conversation dropped, it lets the listener go.
  for (let i = 0; i < 100 && link.objectsHeld > 0; i++) await link.ping();
  expect(link.objectsHeld).toBe(0);
  link.close();
});

test('64-bit integers cross at their limits', async () => {
  const [link, provider] = await hello();
  const wide = provider.table('wide')!;
  expect(await wide.call('echo_i64', [-9223372036854775808n])).toBe(-9223372036854775808n);
  expect(await wide.call('echo_u64', [18446744073709551615n])).toBe(18446744073709551615n);
  expect(await wide.call('echo_i64', [-5])).toBe(-5);
  expect(await wide.call('echo_u64', [2 ** 53 - 1])).toBe(2 ** 53 - 1);
  await expect(wide.call('echo_u64', [-1]), 'below u64').rejects.toBeInstanceOf(TypeError);
  link.close();
});

test('a wrong key is refused before the link is up', async () => {
  const refused = connect('not the key');
  await expect(refused).rejects.toBeInstanceOf(BadValue);
  await expect(refused).rejects.toThrow("does not hold this link's key");
});

test('a closed link answers Gone', async () => {
  const [link, provider] = await hello();
  link.close();
  expect(link.alive).toBe(false);
  expect(await link.done).toContain('closed');
  await expect(provider.table('greeter')!.call('greet', ['late'])).rejects.toBeInstanceOf(Gone);
});

test('a server that is not there rejects Gone', async () => {
  // A port nothing listens on: the server's own, once it is closed, would race; 9 is discard.
  await expect(RemoteLink.connect('ws://127.0.0.1:9/', { security: 'none', key, handshakeTimeout: 5_000 })).rejects.toBeInstanceOf(Gone);
});
