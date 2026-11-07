// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// A host written in Node hands a sink to the Rust echo, whose own thread
// calls it. Node runs JavaScript on one thread: the echo's thread calls
// into the sink through koffi, which queues the call to Node's event loop
// and makes the echo's thread wait for the answer. So the sink hears
// while the event loop is free to run it (an `await`), and a host that
// blocks the loop inside a synchronous `send` hears nothing until it
// returns.

import path from 'node:path';

import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { Gone, Registry, Value } from '../src/index.js';
import { ChannelEcho } from '../../../../examples/channel_echo/node/channel_echo_kind.js';
import { libraryOf } from '../src/provide.js';
import { REPO } from './helpers.js';
import { libraryDir } from './setup.js';

const GONE = 12;

class Sink {
  readonly received: string[] = [];
  closedWith: number | null = null;
  away = false;
  destroyed = false;
  receive(bytes: Uint8Array): void {
    if (this.away) {
      throw new Gone('the host takes nothing more');
    }
    this.received.push(new TextDecoder().decode(bytes));
  }
  closed(status: number): void {
    this.closedWith = status;
  }
  [Symbol.dispose](): void {
    this.destroyed = true;
  }
}

/** Lets the event loop run until `done`, for at most `ms`. */
async function until(done: () => boolean, ms = 30000): Promise<boolean> {
  const end = Date.now() + ms;
  while (!done()) {
    if (Date.now() > end) return false;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  return true;
}

let reg: Registry;

beforeAll(() => {
  reg = new Registry('node-channel', '0.1');
  reg.scanDir(libraryDir(), { rules: 'kind=channel_echo' });
});

afterAll(() => {
  reg.close();
});

const bytes = (text: string): Uint8Array => new TextEncoder().encode(text);

// `send` is a synchronous call: while it waits for room in the echo's
// queue the event loop cannot run the sink the echo's thread is calling,
// so these tests send a few messages, then let the loop run.
const BATCH = 3;

describe('a Node host receives on the echo thread', () => {
  it('hears the messages in order, a batch at a time', async () => {
    const echo = reg.bind('channel_echo_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({});
    const channel = echo.start(config, sink);
    const sent: string[] = [];
    for (let i = 0; i < 20; i++) {
      sent.push(`m${i}`);
      channel.send(bytes(`m${i}`));
      if (sent.length % BATCH === 0) await until(() => sink.received.length === sent.length);
    }
    expect(await until(() => sink.received.length === 20)).toBe(true);
    expect(sink.received).toEqual(sent);
    channel.close(0);
    expect(await until(() => sink.closedWith !== null)).toBe(true);
    channel.destroy();
  });

  it('closes once with the host status, refuses later sends and lets the sink go', async () => {
    const echo = reg.bind('channel_echo_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({});
    const channel = echo.start(config, sink);
    for (const text of ['1', '2', '3']) channel.send(bytes(text));
    channel.close(5);
    expect(() => channel.send(bytes('4'))).toThrow(Gone);
    expect(await until(() => sink.closedWith !== null)).toBe(true);
    expect(sink.closedWith).toBe(5);
    channel.destroy();
    expect(await until(() => sink.destroyed)).toBe(true);
  });

  it('treats dropping the channel as close(0)', async () => {
    const echo = reg.bind('channel_echo_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({});
    const channel = echo.start(config, sink);
    channel.send(bytes('1'));
    expect(await until(() => sink.received.length === 1)).toBe(true);
    channel.destroy();
    expect(await until(() => sink.closedWith !== null)).toBe(true);
    expect(sink.closedWith).toBe(0);
    expect(await until(() => sink.destroyed)).toBe(true);
  });

  it('hears the provider close with its own status', async () => {
    const echo = reg.bind('channel_echo_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({ close_after: 2, close_status: 7 });
    const channel = echo.start(config, sink);
    channel.send(bytes('1'));
    channel.send(bytes('2'));
    expect(await until(() => sink.closedWith !== null)).toBe(true);
    expect(sink.closedWith).toBe(7);
    expect(() => channel.send(bytes('3'))).toThrow(Gone);
    channel.destroy();
    expect(sink.received).toEqual(['1', '2']);
  });

  it('ends the stream with GONE when its sink says the host has gone', async () => {
    const echo = reg.bind('channel_echo_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({});
    const channel = echo.start(config, sink);
    channel.send(bytes('1'));
    expect(await until(() => sink.received.length === 1)).toBe(true);
    sink.away = true;
    channel.send(bytes('2'));
    expect(await until(() => sink.closedWith !== null)).toBe(true);
    expect(sink.closedWith).toBe(GONE);
    expect(() => channel.send(bytes('3'))).toThrow(Gone);
    channel.destroy();
    expect(sink.received).toEqual(['1']);
  });
});

// The JavaScript echo under Node serves the channel from Node itself: its
// objects are called on Node's thread, so it answers inside `send`, as the
// QuickJS source's does.
describe('the JavaScript echo under Node', () => {
  const ECHO = path.join(REPO, 'examples', 'channel_echo', 'node', 'echo.js');
  let own: Registry;

  beforeAll(async () => {
    own = new Registry('node-own-echo', '0.1');
    own.registerLibrary(await libraryOf(ECHO));
  });

  afterAll(() => {
    own.close();
  });

  it('echoes in order, closes with each status, and lets the sink go', async () => {
    const echo = own.bind('channel_echo_js_echo', ChannelEcho);
    const sink = new Sink();
    using config = new Value({ close_after: 3, close_status: 7 });
    const channel = echo.start(config, sink);
    for (const text of ['a', 'b', 'c']) channel.send(bytes(text));
    expect(sink.received).toEqual(['a', 'b', 'c']);
    expect(sink.closedWith).toBe(7);
    expect(() => channel.send(bytes('d'))).toThrow(Gone);
    channel.destroy();
    expect(sink.destroyed).toBe(true);
  });

  it('closes with the host status, and dropping the channel is close(0)', async () => {
    const echo = own.bind('channel_echo_js_echo', ChannelEcho);
    using config = new Value({});
    const first = new Sink();
    const channel = echo.start(config, first);
    channel.send(bytes('1'));
    channel.close(5);
    expect(first.closedWith).toBe(5);
    expect(() => channel.send(bytes('2'))).toThrow(Gone);
    channel.destroy();
    const second = new Sink();
    echo.start(config, second).destroy();
    expect(second.closedWith).toBe(0);
    expect(second.destroyed).toBe(true);
  });

  it('ends the stream with GONE when the sink refuses', async () => {
    const echo = own.bind('channel_echo_js_echo', ChannelEcho);
    using config = new Value({});
    const sink = new Sink();
    sink.away = true;
    const channel = echo.start(config, sink);
    channel.send(bytes('1'));
    expect(sink.closedWith).toBe(GONE);
    expect(() => channel.send(bytes('2'))).toThrow(Gone);
    channel.destroy();
  });
});
