// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The link against a peer the suite plays itself, for what a well-behaved
// server never does.

import { describe, expect, test, vi } from 'vitest';

import { ObjectRef, decodeArgs, encodeAnswer, encodeArgs } from '../src/args.js';
import {
  type AnyFrame,
  Call,
  Cancel,
  Frame,
  FrameWriter,
  Goodbye,
  Hello,
  ListCatalog,
  Ping,
  Pong,
  Proof,
  Release,
  Reply,
  Target,
  Welcome,
} from '../src/codec.js';
import { RemoteKind } from '../src/described.js';
import { Busy, Gone, GuatiaoError, NotFound, Unsupported, WrongKind } from '../src/errors.js';
import { type RemoteChannel, type RemoteProvider, RemoteLink, RemoteTrust, Served } from '../src/link.js';
import { CLIENT_LABEL, SERVER_LABEL, proofOf } from '../src/proof.js';
import { kindsDocument } from './support.js';

/** Both ends of a link the suite plays the server of. */
class Pipe implements RemoteChannel {
  private message: ((body: Uint8Array) => void) | undefined;
  private closed: ((why: string) => void) | undefined;
  private readonly sent: Uint8Array[] = [];
  private waiting: ((body: Uint8Array) => void) | undefined;
  ended = false;

  start(message: (body: Uint8Array) => void, closed: (why: string) => void): void {
    this.message = message;
    this.closed = closed;
  }

  send(body: Uint8Array): void {
    if (this.waiting !== undefined) {
      const w = this.waiting;
      this.waiting = undefined;
      w(body);
    } else this.sent.push(body);
  }

  close(): void {
    if (this.ended) return;
    this.ended = true;
    this.closed?.('closed');
  }

  /** The next frame the link sent. */
  async next(): Promise<AnyFrame> {
    const queued = this.sent.shift();
    const body =
      queued ??
      (await new Promise<Uint8Array>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('the link sent nothing')), 5_000);
        this.waiting = (b) => {
          clearTimeout(timer);
          resolve(b);
        };
      }));
    return Frame.decode(body);
  }

  reply(frame: Frame): void {
    this.raw(frame.encode());
  }

  raw(body: Uint8Array): void {
    this.message!(body);
  }
}

/**
 * Kinds of the suite's own: `maker` hands the peer an `echo` this side
 * serves, or two listeners, or lends it an `echo` and is answered one; an
 * `echo` takes listeners handed over.
 */
const OWN_KINDS = [
  {
    name: 'maker',
    object: false,
    slots: [
      { name: 'make', args: [{ name: 'e', type: 'object', kind: 'echo', pass: 'taken' }] },
      {
        name: 'pair',
        args: [
          { name: 'a', type: 'object', kind: 'listener', pass: 'taken' },
          { name: 'b', type: 'object', kind: 'listener', pass: 'taken' },
        ],
      },
      {
        name: 'wrap',
        args: [{ name: 'e', type: 'object', kind: 'echo', pass: 'lent' }],
        answer: { type: 'object', kind: 'echo' },
      },
    ],
  },
  {
    name: 'echo',
    object: true,
    slots: [
      { name: 'big', answer: { type: 'text' } },
      { name: 'fill', args: [{ name: 'dst', type: 'bytes_mut' }], answer: { type: 'i64' } },
      { name: 'take', args: [{ name: 'l', type: 'object', kind: 'listener', pass: 'taken' }] },
      {
        name: 'take2',
        args: [
          { name: 'a', type: 'object', kind: 'listener', pass: 'taken' },
          { name: 'b', type: 'object', kind: 'listener', pass: 'taken' },
        ],
      },
    ],
  },
];

const ECHO = RemoteKind.fromDescription(OWN_KINDS[1]);

/**
 * A link at the local level over a `Pipe`, its catalog listed: one
 * provider whose greeter table fills only `greet`, `start` and
 * `greet_into`, and whose `maker` table fills every slot. `welcome` is what
 * the peer offers, `maxCalls` how many of its calls this side runs.
 */
async function scripted(
  options: { timeout?: number | null; welcome?: Record<string, unknown>; maxCalls?: number } = {},
): Promise<[RemoteLink, Pipe, RemoteProvider]> {
  const pipe = new Pipe();
  const linking = RemoteLink.over(pipe, {
    trust: RemoteTrust.LOCAL,
    timeout: options.timeout ?? null,
    maxCalls: options.maxCalls ?? 64,
  });
  expect(await pipe.next()).toBeInstanceOf(Hello);
  pipe.reply(new Welcome(new Uint8Array(16), new Uint8Array(0), options.welcome ?? {}));
  const link = await linking;
  const listing = link.list();
  const ask = (await pipe.next()) as ListCatalog;
  const catalog = {
    kinds: [...kindsDocument.kinds, ...OWN_KINDS],
    libraries: [
      {
        id: 'scripted',
        version: '1',
        stamp: 's1',
        providers: [
          {
            id: 'p',
            name: 'P',
            tables: [
              { kind: 'greeter', table: 1, slots: ['greet', 'start', 'greet_into'] },
              { kind: 'maker', table: 2, slots: ['make', 'pair', 'wrap'] },
            ],
          },
        ],
      },
    ],
  };
  pipe.reply(new Reply(ask.call, 0, new FrameWriter().value(catalog).done()));
  return [link, pipe, (await listing).providers[0]!];
}

/** Hands the peer `echo` through `maker.make`; the id the peer holds it by. */
async function handEcho(pipe: Pipe, provider: RemoteProvider, echo: Served): Promise<number> {
  const maker = provider.table('maker')!;
  const making = maker.call('make', [echo]);
  const make = (await pipe.next()) as Call;
  const ref = decodeArgs(maker.kind.slot('make'), make.args)[0] as ObjectRef;
  pipe.reply(new Reply(make.call, 0, new Uint8Array(0)));
  await making;
  return ref.id;
}

/** The peer's `echo.take` (one ref) or `echo.take2` (two) on `echo`, handing `refs` over. */
function take(call: number, parent: number, echo: number, refs: ObjectRef[]): Call {
  const slot = ECHO.slot(refs.length === 1 ? 'take' : 'take2');
  return new Call(call, parent, Target.object(echo), slot.index, encodeArgs(slot, refs));
}

describe('against a scripted peer', () => {
  test('a slot the far table leaves out is Unsupported, and nothing is sent', async () => {
    const [link, pipe, provider] = await scripted();
    const table = provider.table('greeter')!;
    expect(table.has('shout')).toBe(false);
    await expect(table.call('shout', ['x'])).rejects.toBeInstanceOf(Unsupported);
    const ping = link.ping();
    const frame = await pipe.next();
    expect(frame, 'the refused call sent nothing').toBeInstanceOf(Ping);
    pipe.reply(new Pong((frame as Ping).n));
    await ping;
    link.close();
  });

  test('a call past its timeout is cancelled, and an object its late answer carries is released', async () => {
    const [link, pipe, provider] = await scripted({ timeout: 200 });
    const table = provider.table('greeter')!;
    const listener = new Served('listener', { heard: () => undefined });
    const starting = table.call('start', [listener]);
    const call = (await pipe.next()) as Call;
    await expect(starting).rejects.toBeInstanceOf(Gone);
    const cancel = (await pipe.next()) as Cancel;
    expect(cancel.call).toBe(call.call);
    expect(link.objectsHeld, 'the listener went out: the peer has it').toBe(1);
    // The late answer: a conversation of the peer's, nobody to take it.
    const slot = table.kind.slot('start');
    pipe.reply(new Reply(call.call, 0, encodeAnswer(slot, new ObjectRef(5), [undefined])));
    const release = (await pipe.next()) as Release;
    expect(release).toBeInstanceOf(Release);
    expect(release.object).toBe(5);

    // Another, whose late answer names this side's own object.
    const again = table.call('start', [listener]);
    const second = (await pipe.next()) as Call;
    await expect(again).rejects.toBeInstanceOf(Gone);
    expect(await pipe.next()).toBeInstanceOf(Cancel);
    expect(link.objectsHeld).toBe(2);
    const mine = decodeArgs(slot, second.args)[0] as ObjectRef;
    pipe.reply(new Reply(second.call, 0, encodeAnswer(slot, new ObjectRef(mine.id, true), [undefined])));
    const ping = link.ping();
    const asked = (await pipe.next()) as Ping;
    pipe.reply(new Pong(asked.n));
    await ping;
    expect(link.objectsHeld, 'the one answered back is dropped').toBe(1);
    link.close();
  });

  test('a call made during a handler names it as its parent, and only while it runs', async () => {
    const [link, pipe, provider] = await scripted();
    const table = provider.table('greeter')!;
    const greet = table.kind.slot('greet');
    let later: Promise<unknown> | undefined;
    let resolveAfterwards!: () => void;
    const afterwards = new Promise<void>((resolve) => {
      resolveAfterwards = resolve;
    });
    const listener = new Served('listener', {
      heard: async (_args, call) => {
        await table.call('greet', ['inside'], { during: call });
        setTimeout(() => {
          later = table.call('greet', ['after'], { during: call });
          resolveAfterwards();
        }, 0);
        return undefined;
      },
    });
    const starting = table.call('start', [listener]);
    const start = (await pipe.next()) as Call;
    const slot = table.kind.slot('start');
    const mine = decodeArgs(slot, start.args)[0] as ObjectRef;
    pipe.reply(new Reply(start.call, 0, encodeAnswer(slot, new ObjectRef(5), [undefined])));
    await starting;
    // The peer tells the listener something, during no call of ours.
    pipe.reply(new Call(40, 0, Target.object(mine.id), 0, new FrameWriter().text('x').done()));
    const inside = (await pipe.next()) as Call;
    expect(inside.parent, 'made while `heard` runs').toBe(40);
    pipe.reply(new Reply(inside.call, 0, encodeAnswer(greet, { g: 1 }, [undefined])));
    const heard = (await pipe.next()) as Reply;
    expect([heard.call, heard.status]).toEqual([40, 0]);
    await afterwards;
    const after = (await pipe.next()) as Call;
    expect(after.parent, '`heard` has returned').toBe(0);
    pipe.reply(new Reply(after.call, 0, encodeAnswer(greet, { g: 2 }, [undefined])));
    expect(await later).toEqual({ g: 2 });
    link.close();
  });

  test("an answer over the peer's max_frame is refused, not sent", async () => {
    const [link, pipe, provider] = await scripted({ welcome: { max_frame: 64 } });
    const maker = provider.table('maker')!;
    const echo = new Served('echo', {
      big: () => 'x'.repeat(200),
      // Breaks the buffer the answer writes back.
      fill: (args) => {
        args[0] = null;
        return 0;
      },
    });
    const making = maker.call('make', [echo]);
    const make = (await pipe.next()) as Call;
    const ref = decodeArgs(maker.kind.slot('make'), make.args)[0] as ObjectRef;
    pipe.reply(new Reply(make.call, 0, new Uint8Array(0)));
    await making;
    pipe.reply(new Call(60, 0, Target.object(ref.id), 0, new Uint8Array(0)));
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status]).toEqual([60, 7]);
    expect(refused.words()).toContain('over the 64');
    pipe.reply(new Call(61, 0, Target.object(ref.id), 1, new FrameWriter().uint(8).done()));
    const broken = (await pipe.next()) as Reply;
    expect([broken.call, broken.status], 'still answered').toEqual([61, 7]);
    expect(link.alive).toBe(true);
    link.close();
  });

  test('a handed object is taken back when a later argument fails', async () => {
    const [link, , provider] = await scripted();
    await expect(
      provider.table('maker')!.call('pair', [new Served('listener', {}), new Served('echo', {})]),
    ).rejects.toBeInstanceOf(WrongKind);
    expect(link.objectsHeld).toBe(0);
    link.close();
  });

  test('a slot the kind does not have rejects', async () => {
    const [link, , provider] = await scripted();
    await expect(provider.table('greeter')!.call('nope')).rejects.toBeInstanceOf(NotFound);
    link.close();
  });

  test('a malformed frame ends the link with GOODBYE', async () => {
    const [link, pipe] = await scripted();
    pipe.raw(new Uint8Array([0x7f]));
    const bye = (await pipe.next()) as Goodbye;
    expect(bye).toBeInstanceOf(Goodbye);
    expect(bye.status).toBe(7);
    expect(await link.done).toContain('malformed');
    expect(link.alive).toBe(false);
  });

  test('a value outside the wire grammar in a frame ends the link with GOODBYE, never a throw', async () => {
    const [link, pipe] = await scripted();
    // A WELCOME whose options hold a number written `abc`.
    const body = new FrameWriter()
      .byte(0x02)
      .raw(new Uint8Array([0x47, 0x54, 0x52, 0x4d, 1]))
      .raw(new Uint8Array(16))
      .bytes(new Uint8Array(0))
      .bytes(new Uint8Array([3, 3, 0x61, 0x62, 0x63]))
      .done();
    expect(() => pipe.raw(body)).not.toThrow();
    const bye = (await pipe.next()) as Goodbye;
    expect(bye).toBeInstanceOf(Goodbye);
    expect(bye.status).toBe(7);
    expect(await link.done).toContain('malformed');
  });

  test('a handler refusing with a status that is none goes as INTERNAL', async () => {
    const [link, pipe, provider] = await scripted();
    const maker = provider.table('maker')!;
    const echo = new Served('echo', {
      big: () => {
        throw new GuatiaoError(-1, 'not a status');
      },
    });
    const making = maker.call('make', [echo]);
    const make = (await pipe.next()) as Call;
    const ref = decodeArgs(maker.kind.slot('make'), make.args)[0] as ObjectRef;
    pipe.reply(new Reply(make.call, 0, new Uint8Array(0)));
    await making;
    pipe.reply(new Call(80, 0, Target.object(ref.id), 0, new Uint8Array(0)));
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status, refused.words()]).toEqual([80, 100, 'not a status']);
    expect(link.alive).toBe(true);
    link.close();
  });

  test('a call refused before it runs lets go of what it handed over', async () => {
    const [link, pipe, provider] = await scripted({ maxCalls: 1 });
    const echo = await handEcho(
      pipe,
      provider,
      new Served('echo', { big: () => new Promise(() => undefined), take: () => undefined }),
    );
    // Two listeners of this side's the peer holds.
    const maker = provider.table('maker')!;
    const pairing = maker.call('pair', [new Served('listener', {}), new Served('listener', {})]);
    const pair = (await pipe.next()) as Call;
    const mine = decodeArgs(maker.kind.slot('pair'), pair.args) as ObjectRef[];
    pipe.reply(new Reply(pair.call, 0, new Uint8Array(0)));
    await pairing;
    expect(link.objectsHeld).toBe(3);

    // Made during a call that has returned.
    pipe.reply(take(70, 4, echo, [new ObjectRef(5)]));
    expect(((await pipe.next()) as Release).object).toBe(5);
    expect(((await pipe.next()) as Reply).call).toBe(70);
    // A slot the echo does not fill.
    pipe.reply(take(71, 0, echo, [new ObjectRef(6), new ObjectRef(7)]));
    expect(((await pipe.next()) as Release).object).toBe(6);
    expect(((await pipe.next()) as Release).object).toBe(7);
    const unfilled = (await pipe.next()) as Reply;
    expect([unfilled.call, unfilled.status]).toEqual([71, 13]);
    // Past this side's maxCalls: the one place runs `big`, forever.
    pipe.reply(new Call(72, 0, Target.object(echo), 0, new Uint8Array(0)));
    pipe.reply(take(73, 0, echo, [new ObjectRef(8)]));
    expect(((await pipe.next()) as Release).object).toBe(8);
    const busy = (await pipe.next()) as Reply;
    expect([busy.call, busy.status]).toEqual([73, 14]);
    // This side's own, given back in a refused call, is dropped.
    pipe.reply(take(74, 4, echo, [new ObjectRef(mine[0]!.id, true)]));
    expect(((await pipe.next()) as Reply).call).toBe(74);
    expect(link.objectsHeld).toBe(2);
    link.close();
  });

  test('a call refused while its arguments are received lets each go', async () => {
    const [link, pipe, provider] = await scripted();
    const echo = await handEcho(pipe, provider, new Served('echo', { take2: () => undefined }));
    // The first is no object of this side's; the second is the peer's.
    pipe.reply(take(80, 0, echo, [new ObjectRef(99, true), new ObjectRef(9)]));
    expect(((await pipe.next()) as Release).object).toBe(9);
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status]).toEqual([80, 12]);
    link.close();
  });

  test('an object lent for a call cannot come back handed over', async () => {
    const [link, pipe, provider] = await scripted();
    const echo = await handEcho(pipe, provider, new Served('echo', { take: () => undefined }));
    const table = provider.table('greeter')!;
    const heard: unknown[] = [];
    const listener = new Served('listener', { heard: (args) => void heard.push(args[0]) });
    const greeting = table.call('greet_into', ['x', listener]);
    const greet = (await pipe.next()) as Call;
    const lent = decodeArgs(table.kind.slot('greet_into'), greet.args)[1] as ObjectRef;
    pipe.reply(take(90, greet.call, echo, [new ObjectRef(lent.id, true)]));
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status]).toEqual([90, 9]);
    // Still lent: the call it was lent for can call it.
    pipe.reply(new Call(91, greet.call, Target.object(lent.id), 0, new FrameWriter().text('still').done()));
    const called = (await pipe.next()) as Reply;
    expect([called.call, called.status]).toEqual([91, 0]);
    expect(heard).toEqual(['still']);
    pipe.reply(new Reply(greet.call, 0, new Uint8Array(0)));
    await greeting;

    // Nor come back as an answer.
    const maker = provider.table('maker')!;
    const wrapSlot = maker.kind.slot('wrap');
    const wrapping = maker.call('wrap', [new Served('echo', {})]);
    const wrap = (await pipe.next()) as Call;
    const mine = decodeArgs(wrapSlot, wrap.args)[0] as ObjectRef;
    pipe.reply(new Reply(wrap.call, 0, encodeAnswer(wrapSlot, new ObjectRef(mine.id, true), [undefined])));
    await expect(wrapping).rejects.toBeInstanceOf(WrongKind);
    link.close();
  });

  test("a call waiting for a place under the peer's max_calls gives up Busy at its timeout", async () => {
    const [link, pipe, provider] = await scripted({ timeout: 300, welcome: { max_calls: 1 } });
    // Every timer of one instant runs in the order it was set, so which
    // call gives up first is the clock's to say, not the machine's load.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] });
    try {
      const table = provider.table('greeter')!;
      const greet = table.kind.slot('greet');
      const first = expect(table.call('greet', ['a'])).rejects.toBeInstanceOf(Gone);
      const a = (await pipe.next()) as Call;
      // Both wait for the one place the first holds.
      const second = expect(table.call('greet', ['b'])).rejects.toBeInstanceOf(Gone);
      const third = expect(table.call('greet', ['c'])).rejects.toBeInstanceOf(Busy);
      await vi.advanceTimersByTimeAsync(300);
      // The first times out and its place goes to the second, with none of
      // its time left; the third has waited its whole timeout by then.
      expect(((await pipe.next()) as Cancel).call).toBe(a.call);
      await first;
      const b = (await pipe.next()) as Call;
      expect(decodeArgs(greet, b.args)).toEqual(['b']);
      await third;
      // A timer set for no time while the clock ticks runs a millisecond on.
      await vi.advanceTimersByTimeAsync(1);
      expect(((await pipe.next()) as Cancel).call, 'the wait counts against the call').toBe(b.call);
      await second;
    } finally {
      vi.useRealTimers();
    }
    const ping = link.ping();
    const asked = await pipe.next();
    expect(asked, 'the third was never sent').toBeInstanceOf(Ping);
    pipe.reply(new Pong((asked as Ping).n));
    await ping;
    link.close();
  });

  test('a call made during a call that returned is refused Gone', async () => {
    const [link, pipe] = await scripted();
    pipe.reply(new Call(9, 4, Target.object(1), 0, new Uint8Array(0)));
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status]).toEqual([9, 12]);
    link.close();
  });

  test('a call on an object this side never served is refused Gone', async () => {
    const [link, pipe] = await scripted();
    pipe.reply(new Call(3, 0, Target.object(77), 0, new Uint8Array(0)));
    const refused = (await pipe.next()) as Reply;
    expect([refused.call, refused.status]).toEqual([3, 12]);
    expect(refused.words()).toContain('let go');
    link.close();
  });

  test('past maxCalls a call of the peer is refused BUSY', async () => {
    const pipe = new Pipe();
    const linking = RemoteLink.over(pipe, { trust: RemoteTrust.LOCAL, timeout: null, maxCalls: 1 });
    await pipe.next();
    pipe.reply(new Welcome(new Uint8Array(16), new Uint8Array(0), {}));
    const link = await linking;
    // An object of this side's the peer holds, whose handler never returns.
    const listing = link.list();
    const ask = (await pipe.next()) as ListCatalog;
    pipe.reply(
      new Reply(
        ask.call,
        0,
        new FrameWriter()
          .value({
            kinds: [...kindsDocument.kinds, ...OWN_KINDS],
            libraries: [{ id: 'l', providers: [{ id: 'p', tables: [{ kind: 'maker', table: 2, slots: ['make'] }] }] }],
          })
          .done(),
      ),
    );
    const maker = (await listing).providers[0]!.table('maker')!;
    const echo = new Served('echo', { big: () => new Promise(() => undefined) });
    const making = maker.call('make', [echo]);
    const make = (await pipe.next()) as Call;
    const ref = decodeArgs(maker.kind.slot('make'), make.args)[0] as ObjectRef;
    pipe.reply(new Reply(make.call, 0, new Uint8Array(0)));
    await making;
    pipe.reply(new Call(70, 0, Target.object(ref.id), 0, new Uint8Array(0)));
    pipe.reply(new Call(71, 0, Target.object(ref.id), 0, new Uint8Array(0)));
    const busy = (await pipe.next()) as Reply;
    expect([busy.call, busy.status]).toEqual([71, 14]);
    link.close();
  });

  test('a GOODBYE in the handshake rejects with its status', async () => {
    const pipe = new Pipe();
    const linking = RemoteLink.over(pipe, { trust: RemoteTrust.LOCAL });
    await pipe.next();
    pipe.reply(new Goodbye(14, 'too many handshakes'));
    await expect(linking).rejects.toMatchObject({ name: 'Busy', status: 14, message: 'too many handshakes' });
    expect(pipe.ended).toBe(true);
  });
});

describe('security', () => {
  const enc = new TextEncoder();

  test('a URL must agree with the security asked', async () => {
    await expect(RemoteLink.connect('ws://127.0.0.1:9/', { key: 'k' })).rejects.toThrow(/security 'none'/);
    await expect(RemoteLink.connect('wss://127.0.0.1:9/', { security: 'none', key: 'k' })).rejects.toThrow(
      /wss:\/\/ runs TLS/,
    );
    await expect(RemoteLink.connect('wss://127.0.0.1:9/', { security: 'server-tls' })).rejects.toThrow(
      /a key or a token/,
    );
    await expect(RemoteLink.connect('http://127.0.0.1:9/', { security: 'none', key: 'k' })).rejects.toThrow(
      /not a ws:\/\/, wss:\/\/ or https:\/\//,
    );
  });

  test('an https:// URL is WebTransport: unsupported where there is none, gone where nothing answers', async () => {
    const linking = RemoteLink.connect('https://127.0.0.1:9/', { security: 'server-tls', key: 'k', handshakeTimeout: 3_000 });
    await expect(linking).rejects.toBeInstanceOf('WebTransport' in globalThis ? Gone : Unsupported);
  });

  test('server TLS proves the key when the WELCOME asks for it', async () => {
    const key = enc.encode('k');
    const pipe = new Pipe();
    const linking = RemoteLink.over(pipe, { trust: RemoteTrust.SERVER_TLS, key, timeout: null });
    const hello = (await pipe.next()) as Hello;
    expect(hello.trust).toBe(3);
    const server = new Uint8Array(16).fill(7);
    pipe.reply(new Welcome(server, proofOf(key, SERVER_LABEL, hello.nonce, server), {}));
    const answered = (await pipe.next()) as Proof;
    expect(answered.proof).toEqual(proofOf(key, CLIENT_LABEL, hello.nonce, server));
    const link = await linking;
    expect(link.trust).toBe(RemoteTrust.SERVER_TLS);
    link.close();
  });

  test('server TLS with a token sends no proof when none is asked', async () => {
    const pipe = new Pipe();
    const linking = RemoteLink.over(pipe, { trust: RemoteTrust.SERVER_TLS, token: 't0k', timeout: null });
    const hello = (await pipe.next()) as Hello;
    expect((hello.options as Record<string, unknown>)['token']).toBe('t0k');
    pipe.reply(new Welcome(new Uint8Array(16), new Uint8Array(0), {}));
    const link = await linking;
    expect(link.alive).toBe(true);
    link.close();
  });

  test('a server asking for a key this client lacks is refused', async () => {
    const pipe = new Pipe();
    const linking = RemoteLink.over(pipe, { trust: RemoteTrust.SERVER_TLS, token: 't0k' });
    const hello = (await pipe.next()) as Hello;
    const server = new Uint8Array(16);
    pipe.reply(new Welcome(server, proofOf(enc.encode('k'), SERVER_LABEL, hello.nonce, server), {}));
    await expect(linking).rejects.toThrow(/has none/);
  });
});
