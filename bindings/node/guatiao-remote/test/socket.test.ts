// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The WebSocket channel against a socket the suite plays, for what a
// server never sends.

import { afterEach, expect, test } from 'vitest';

import { openWebSocket } from '../src/socket.js';

/** A WebSocket that refuses a close code a page may not send, as the real one does. */
class FakeSocket {
  static last: FakeSocket | undefined;
  binaryType = 'blob';
  closedWith: number[] = [];
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;

  constructor(readonly url: string) {
    FakeSocket.last = this;
  }

  send(): void {}

  close(code?: number): void {
    if (code !== undefined && code !== 1000 && (code < 3000 || code > 4999)) {
      throw new DOMException(`close code ${code}`, 'InvalidAccessError');
    }
    this.closedWith.push(code ?? 1005);
  }
}

const real = globalThis.WebSocket;
afterEach(() => {
  globalThis.WebSocket = real;
});

test('a text message ends the channel once, closing as a page may', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
  const opening = openWebSocket('ws://example.invalid/', 5_000);
  const ws = FakeSocket.last!;
  expect(ws.binaryType).toBe('arraybuffer');
  ws.onopen!();
  const channel = await opening;
  const got: Uint8Array[] = [];
  const ends: string[] = [];
  channel.start(
    (body) => got.push(body),
    (why) => ends.push(why),
  );
  ws.onmessage!({ data: new Uint8Array([1, 2]).buffer });
  expect(() => ws.onmessage!({ data: 'hello' })).not.toThrow();
  expect(ends).toEqual(['a text message: frames are binary']);
  expect(ws.closedWith).toEqual([1000]);
  // Nothing after the end reaches the link, and the end is told once.
  ws.onmessage!({ data: new Uint8Array([3]).buffer });
  ws.onclose!({ code: 1000, reason: '' });
  expect(got.map((b) => [...b])).toEqual([[1, 2]]);
  expect(ends).toHaveLength(1);
});

test('messages before the link starts are kept for it', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
  const opening = openWebSocket('ws://example.invalid/', 5_000);
  const ws = FakeSocket.last!;
  ws.onopen!();
  const channel = await opening;
  ws.onmessage!({ data: new Uint8Array([7]).buffer });
  const got: number[][] = [];
  channel.start(
    (body) => got.push([...body]),
    () => undefined,
  );
  expect(got).toEqual([[7]]);
});

test('a socket that closes as it opens rejects Gone', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket;
  const opening = openWebSocket('ws://example.invalid/', 5_000);
  FakeSocket.last!.onclose!({ code: 1006, reason: '' });
  await expect(opening).rejects.toMatchObject({ name: 'Gone' });
});
