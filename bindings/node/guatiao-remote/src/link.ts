// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A link to a peer speaking guatiao's remote protocol: a page, or any
 * JavaScript program, calls the providers a server serves. Everything runs
 * on the event loop: a call the peer makes while one of ours is open runs
 * as its frame arrives.
 *
 * @module
 */

import { ObjectRef, decodeAnswer, decodeArgs, encodeAnswer, encodeArgs, isTree } from './args.js';
import {
  type AnyFrame,
  Call,
  Cancel,
  Close,
  Frame,
  FrameReader,
  FrameWriter,
  Goodbye,
  HANDSHAKE_MAX_FRAME,
  Hello,
  ListCatalog,
  MalformedFrame,
  Open,
  Ping,
  Pong,
  Proof,
  Release,
  Reply,
  Target,
  Welcome,
} from './codec.js';
import { RemoteKind, type RemoteSlot } from './described.js';
import { BadValue, Gone, GuatiaoError, NotFound, Status, Unsupported, WrongKind, errorFor } from './errors.js';
import * as proof from './proof.js';
import { openWebSocket } from './socket.js';
import { define, isPlainObject } from './wire.js';

/** How far a link trusts its peer: the level both sides must name. */
export const RemoteTrust = {
  /** The same machine; the operating system's permissions are the gate. */
  LOCAL: 0,
  /** A network the owner isolates: a pre-shared key proved both ways. No encryption. */
  TRUSTED: 1,
  /** Any network, mutual TLS: both sides proven by certificate. */
  SECURE: 2,
  /** Any network, server TLS: the server proven by its certificate, the client by the key (when asked) or a token. */
  SERVER_TLS: 3,
} as const;
export type RemoteTrust = (typeof RemoteTrust)[keyof typeof RemoteTrust];

/**
 * A link's security, the same choice on every transport and client: `none`
 * (`ws://`: the key proved both ways, no encryption), `server-tls`
 * (`wss://`: TLS proves the server; this side proves the key when asked, or a
 * token) or `mutual-tls` (`wss://`: in a page, with a client certificate the
 * user installed in the browser).
 */
export type RemoteSecurity = 'none' | 'server-tls' | 'mutual-tls';

const TRUST_OF: Record<RemoteSecurity, RemoteTrust> = {
  none: RemoteTrust.TRUSTED,
  'server-tls': RemoteTrust.SERVER_TLS,
  'mutual-tls': RemoteTrust.SECURE,
};

/** Refuses, before anything is sent, a link with nothing to prove itself with at `trust`. */
function checkProof(trust: RemoteTrust, key: Uint8Array | undefined, token: string | undefined): void {
  const hasKey = key !== undefined && key.length > 0;
  if (trust === RemoteTrust.TRUSTED && !hasKey) throw new TypeError('the trusted level needs a pre-shared key');
  if (trust === RemoteTrust.SERVER_TLS && !hasKey && token === undefined) {
    throw new TypeError('at server TLS the client proves itself by a key or a token: name one');
  }
}

/**
 * A message transport: one frame's body per message. A WebSocket is one;
 * another plugs in through `RemoteLink.over`.
 */
export interface RemoteChannel {
  /** Starts delivering: each message to `message`, in order, then `closed` once. */
  start(message: (body: Uint8Array) => void, closed: (why: string) => void): void;
  /** Sends one message. */
  send(body: Uint8Array): void;
  /** Closes the channel; `closed` then runs. */
  close(): void;
}

/** A call the peer is making of an object this side serves, while its handler runs. */
export class InboundCall {
  /** @internal */
  constructor(
    /** @internal */ readonly link: RemoteLink,
    /** @internal */ readonly id: number,
  ) {}

  /** Whether the handler it was given to still runs. */
  get running(): boolean {
    return this.link.isRunning(this.id);
  }
}

/**
 * What runs when the peer calls a slot of an object this side serves:
 * the arguments as the slot's description types them (a lent map, list or
 * buffer is the one the answer writes back, to change in place), and the
 * call, to name as `during` for a call made while it runs. Answers what the
 * slot answers; throwing a `GuatiaoError` refuses with its status.
 */
export type Handler = (args: unknown[], call: InboundCall) => unknown;

/** An object this side serves: `slots` maps each slot's name to its handler. */
export class Served {
  readonly slots: Readonly<Record<string, Handler>>;

  constructor(
    /** The object kind it is, by name. */
    readonly kind: string,
    slots: Record<string, Handler>,
  ) {
    this.slots = Object.freeze({ ...slots });
  }

  toString(): string {
    return `Served(${this.kind})`;
  }
}

/** How a call is made. */
export interface CallOptions {
  /**
   * The peer's call whose handler makes this one: the peer runs it as
   * part of that call (on the thread waiting for it), which a call back
   * into an object busy with that call needs. Named only while the handler
   * still runs.
   */
  during?: InboundCall;
}

/** What a peer serves, as its catalog lists it. */
export class RemoteCatalog {
  /** @internal */
  constructor(
    /** Every kind a table or an object is of, by name. */
    readonly kinds: ReadonlyMap<string, RemoteKind>,
    /** Every library the peer serves. */
    readonly libraries: readonly RemoteLibrary[],
  ) {}

  /** Every provider, of every library. */
  get providers(): RemoteProvider[] {
    return this.libraries.flatMap((l) => [...l.providers]);
  }
}

/** A library a peer serves. Its providers answer only once `RemoteLink.open` has opened it. */
export class RemoteLibrary {
  /** @internal */
  constructor(
    readonly id: string,
    readonly version: string,
    /** Changes whenever the library does. */
    readonly stamp: string,
    readonly providers: readonly RemoteProvider[],
  ) {}

  toString(): string {
    return `RemoteLibrary(${this.id} ${this.version})`;
  }
}

/** A provider a peer serves: a table per kind. */
export class RemoteProvider {
  /** @internal */
  constructor(
    readonly id: string,
    readonly name: string,
    /** What else it declares, or an empty object. */
    readonly meta: Readonly<Record<string, unknown>>,
    /** Its tables, by kind name. */
    readonly tables: ReadonlyMap<string, RemoteTable>,
  ) {}

  /** Its table of `kind`, or `undefined` when it serves none. */
  table(kind: string): RemoteTable | undefined {
    return this.tables.get(kind);
  }

  toString(): string {
    return `RemoteProvider(${this.id})`;
  }
}

/** Something calls go to: a provider's table or a peer's object. */
export abstract class RemoteCallee {
  /** @internal */
  constructor(
    /** @internal */ readonly link: RemoteLink,
    /** What its slots are. */
    readonly kind: RemoteKind,
  ) {}

  /** @internal */
  abstract get target(): Target;

  /** Whether it fills `slot`. */
  abstract has(slot: string): boolean;

  /**
   * Calls `slot` with `args`, typed by the kind's description: a `Served`
   * or a `RemoteObject` where it takes an object, a map, list or
   * `Uint8Array` it writes back into where it lends one. Answers what the
   * slot answers: plain data for a tree, a `RemoteObject` for an object of
   * the peer's, the `Served` itself for one of this side's.
   */
  async call(slot: string, args: readonly unknown[] = [], options: CallOptions = {}): Promise<unknown> {
    const s = this.kind.slot(slot);
    if (!this.has(slot)) throw new Unsupported(`the far \`${this.kind.name}\` table does not fill \`${slot}\``);
    return this.link.callOn(this.target, s, args, options.during);
  }
}

/** A provider's table of one kind. */
export class RemoteTable extends RemoteCallee {
  /** @internal */
  constructor(
    link: RemoteLink,
    kind: RemoteKind,
    /** The number a CALL names it by. */
    readonly id: number,
    /** The slots the far table fills; a caller has no default for the others. */
    readonly slots: ReadonlySet<string>,
  ) {
    super(link, kind);
  }

  /** @internal */
  get target(): Target {
    return Target.table(this.id);
  }

  has(slot: string): boolean {
    return this.slots.has(slot);
  }
}

/** A handle on an object the peer holds. `release` lets it go; a handle lent for one call answers `Gone` once that call returns. */
export class RemoteObject extends RemoteCallee {
  /** @internal */
  gone = false;

  /** @internal */
  constructor(
    link: RemoteLink,
    kind: RemoteKind,
    /** Its id, in the peer's numbering. */
    readonly id: number,
    /** @internal */ readonly lent: boolean,
  ) {
    super(link, kind);
  }

  /** @internal */
  get target(): Target {
    return Target.object(this.id);
  }

  has(): boolean {
    return !this.gone;
  }

  /** Whether it can still be called. */
  get alive(): boolean {
    return !this.gone && this.link.alive;
  }

  override async call(slot: string, args: readonly unknown[] = [], options: CallOptions = {}): Promise<unknown> {
    if (this.gone) throw new Gone('the object has been let go');
    return super.call(slot, args, options);
  }

  /** Lets the object go; the peer drops it. Idempotent. A lent handle is the lender's to end: releasing it ends only this handle. */
  release(): void {
    if (this.gone) return;
    this.gone = true;
    this.link.forget(this);
    if (!this.lent) this.link.send(new Release(this.id));
  }

  override toString(): string {
    return `RemoteObject(${this.kind.name} ${this.id})`;
  }
}

/** An object of this side's, held for the peer. */
interface Export {
  served: Served;
  kind: RemoteKind;
  lent: boolean;
}

interface Pending {
  slot: RemoteSlot | undefined;
  resolve: (reply: Reply) => void;
  reject: (e: unknown) => void;
  timer: ReturnType<typeof setTimeout> | undefined;
}

/** Messages before the link routes them: what the handshake reads. */
class Inbox {
  private readonly queue: Uint8Array[] = [];
  private waiter: ((body: Uint8Array | string) => void) | undefined;
  private ended: string | undefined;
  private route: ((body: Uint8Array) => void) | undefined;
  private onEnd: ((why: string) => void) | undefined;

  push(body: Uint8Array): void {
    if (this.route !== undefined) this.route(body);
    else if (this.waiter !== undefined) {
      const w = this.waiter;
      this.waiter = undefined;
      w(body);
    } else this.queue.push(body);
  }

  close(why: string): void {
    if (this.ended !== undefined) return;
    this.ended = why;
    if (this.onEnd !== undefined) this.onEnd(why);
    else if (this.waiter !== undefined) {
      const w = this.waiter;
      this.waiter = undefined;
      w(why);
    }
  }

  /** The next message within `ms`; the channel's end or the timeout as an error. */
  next(ms: number): Promise<Uint8Array> {
    const queued = this.queue.shift();
    if (queued !== undefined) return Promise.resolve(queued);
    if (this.ended !== undefined) return Promise.reject(new Gone(`the server closed the link: ${this.ended}`));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiter = undefined;
        reject(new Gone('the server did not answer the handshake in time'));
      }, ms);
      this.waiter = (got) => {
        clearTimeout(timer);
        if (typeof got === 'string') reject(new Gone(`the server closed the link: ${got}`));
        else resolve(got);
      };
    });
  }

  /** Hands every message from now on, and those queued, to `route`. */
  forward(route: (body: Uint8Array) => void, onEnd: (why: string) => void): void {
    this.route = route;
    this.onEnd = onEnd;
    for (const body of this.queue.splice(0)) route(body);
    if (this.ended !== undefined) onEnd(this.ended);
  }
}

/** How a link is set up. Times are in milliseconds. */
export interface LinkOptions {
  /** The link's security: `mutual-tls` unless told; `ws://` runs `none` alone, `wss://` the TLS modes. */
  security?: RemoteSecurity;
  /** The pre-shared key: proved both ways with `none`, and at server TLS when the server asks for it. */
  key?: Uint8Array | string;
  /** What this side presents when the server requires a token (server TLS or mutual TLS). */
  token?: string;
  /** How long a call waits for its answer; `null` waits as long as the link lives. Default 30 000. */
  timeout?: number | null;
  /** How long the handshake may take. Default 10 000. */
  handshakeTimeout?: number;
  /** The largest message this side reads. Default 64 MiB. */
  maxFrame?: number;
  /** How many of the peer's calls this side runs at once. Default 64. */
  maxCalls?: number;
}

/** Calls given up on whose late answer may carry an object to let go: at most this many remembered. */
const MAX_CANCELLED = 4096;

/** A link to a peer speaking guatiao's remote protocol. */
export class RemoteLink {
  private why_: string | undefined;
  private readonly doneWith: Promise<string>;
  private resolveDone!: (why: string) => void;
  private nextCall = 1;
  private nextExport = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly cancelled = new Map<number, RemoteSlot>();
  private readonly running = new Map<number, boolean>();
  private runningTop = 0;
  private inFlight = 0;
  private readonly permits: { resolve: () => void; reject: (e: unknown) => void }[] = [];
  private readonly exports = new Map<number, Export>();
  private readonly kinds = new Map<string, RemoteKind>();
  private readonly finalizer = new FinalizationRegistry<number>((id) => {
    if (this.alive) this.send(new Release(id));
  });

  private constructor(
    private readonly channel: RemoteChannel,
    /** The level the link runs at. */
    readonly trust: RemoteTrust,
    private readonly timeout: number | null,
    private readonly maxFrame: number,
    private readonly maxCalls: number,
    private readonly sendMax: number,
    private readonly theirCalls: number,
  ) {
    this.doneWith = new Promise((resolve) => {
      this.resolveDone = resolve;
    });
  }

  /**
   * Connects to a server's WebSocket URL at `options.security`, mutual TLS
   * unless told: `ws://host:port/path` runs `none` alone, `wss://` the TLS
   * modes, whose certificates the browser (or Node) checks as it checks any.
   */
  static async connect(url: string | URL, options: LinkOptions = {}): Promise<RemoteLink> {
    const text = String(url);
    const scheme = text.slice(0, text.indexOf(':')).toLowerCase();
    if (scheme !== 'ws' && scheme !== 'wss') throw new TypeError(`\`${text}\` is not a ws:// or wss:// URL`);
    const security = options.security ?? 'mutual-tls';
    if (!(security in TRUST_OF)) throw new TypeError(`\`${String(security)}\` is not none, server-tls or mutual-tls`);
    if ((scheme === 'wss') !== (security !== 'none')) {
      throw new TypeError(
        scheme === 'wss'
          ? "wss:// runs TLS: pass security 'server-tls' or 'mutual-tls'"
          : `ws:// carries no TLS, and the link is ${security}: pass security 'none', or connect to wss://`,
      );
    }
    const trust = TRUST_OF[security];
    const key = typeof options.key === 'string' ? new TextEncoder().encode(options.key) : options.key;
    checkProof(trust, key, options.token);
    const channel = await openWebSocket(text, options.handshakeTimeout ?? 10_000);
    return RemoteLink.over(channel, { ...options, trust });
  }

  /**
   * Runs the handshake over `channel` at `trust` and answers the link. The
   * trusted level needs `key`; server TLS needs `key` or `token`, and the
   * channel's own TLS must have proved the server.
   */
  static async over(channel: RemoteChannel, options: LinkOptions & { trust?: RemoteTrust }): Promise<RemoteLink> {
    const trust = options.trust ?? RemoteTrust.TRUSTED;
    const key = typeof options.key === 'string' ? new TextEncoder().encode(options.key) : options.key;
    const timeout = options.timeout === undefined ? 30_000 : options.timeout;
    const handshakeTimeout = options.handshakeTimeout ?? 10_000;
    const maxFrame = options.maxFrame ?? 64 << 20;
    const maxCalls = options.maxCalls ?? 64;
    try {
      checkProof(trust, key, options.token);
    } catch (e) {
      channel.close();
      throw e;
    }
    const inbox = new Inbox();
    channel.start(
      (body) => inbox.push(body),
      (why) => inbox.close(why),
    );
    const mine = proof.nonce();
    const offer: Record<string, unknown> = { max_frame: maxFrame, max_calls: maxCalls };
    if (options.token !== undefined) offer['token'] = options.token;
    const next = async (): Promise<Uint8Array> => {
      const body = await inbox.next(handshakeTimeout);
      if (body.length > HANDSHAKE_MAX_FRAME) {
        throw new MalformedFrame('too_large', `a handshake frame of ${body.length} bytes`);
      }
      return body;
    };
    try {
      channel.send(new Hello(trust, mine, offer).encode());
      const answer = Frame.decode(await next());
      if (answer instanceof Goodbye) throw errorFor(answer.status, answer.message);
      if (!(answer instanceof Welcome)) throw new MalformedFrame('field', 'the server sent no WELCOME');
      // At server TLS a WELCOME with a proof asks for the key; one without asks none.
      const asked = trust === RemoteTrust.TRUSTED || (trust === RemoteTrust.SERVER_TLS && answer.proof.length > 0);
      if (asked) {
        if (key === undefined || key.length === 0) {
          throw new BadValue('the server proves clients by a pre-shared key, and this client has none');
        }
        if (!proof.proves(key, proof.SERVER_LABEL, mine, answer.nonce, answer.proof)) {
          throw new BadValue("the server does not hold this link's key");
        }
        channel.send(new Proof(proof.proofOf(key, proof.CLIENT_LABEL, mine, answer.nonce)).encode());
      }
      const offered = answer.options;
      const number = (name: string, fallback: number): number => {
        const v = typeof offered === 'object' && offered !== null ? (offered as Record<string, unknown>)[name] : undefined;
        return typeof v === 'number' && Number.isSafeInteger(v) && v > 0 ? v : fallback;
      };
      const link = new RemoteLink(
        channel,
        trust,
        timeout,
        maxFrame,
        maxCalls,
        number('max_frame', maxFrame),
        number('max_calls', maxCalls),
      );
      inbox.forward(
        (body) => link.route(body),
        (why) => link.end(`the peer closed the link: ${why}`),
      );
      return link;
    } catch (e) {
      channel.close();
      throw e;
    }
  }

  /** Whether the link still runs. */
  get alive(): boolean {
    return this.why_ === undefined;
  }

  /** Why the link ended, once it has. */
  get why(): string | undefined {
    return this.why_;
  }

  /** Resolves with why the link ended. */
  get done(): Promise<string> {
    return this.doneWith;
  }

  /** How many objects of this side's the peer holds. */
  get objectsHeld(): number {
    return this.exports.size;
  }

  /** Ends the link: GOODBYE, then the channel closes. Every call waiting rejects `Gone`. */
  close(): void {
    if (!this.alive) return;
    this.send(new Goodbye(0, 'the host closed the link'));
    this.end('this side closed the link');
  }

  /** @internal */
  send(frame: Frame): void {
    this.sendBody(frame.encode());
  }

  private sendBody(body: Uint8Array): void {
    if (!this.alive) return;
    try {
      this.channel.send(body);
    } catch (e) {
      this.end(`a write failed: ${String(e)}`);
    }
  }

  private end(why: string): void {
    if (this.why_ !== undefined) return;
    this.why_ = why;
    for (const p of this.pending.values()) {
      if (p.timer !== undefined) clearTimeout(p.timer);
      p.reject(new Gone(why));
    }
    this.pending.clear();
    this.exports.clear();
    for (const waiting of this.permits.splice(0)) waiting.reject(new Gone(why));
    this.resolveDone(why);
    try {
      this.channel.close();
    } catch {
      // Closing an ended channel is nothing to report.
    }
  }

  private goodbye(status: number, why: string): void {
    this.send(new Goodbye(status, why));
    this.end(why);
  }

  /** The peer's catalog. Its kinds type every call after it. */
  async list(): Promise<RemoteCatalog> {
    const reply = await this.request((id) => new ListCatalog(id), undefined);
    const r = new FrameReader(reply.body);
    const value = r.value();
    r.end();
    return this.catalogOf(value);
  }

  /** Opens `library` on the peer: its providers answer until `closeLibrary` or the link's end. */
  async open(library: RemoteLibrary): Promise<void> {
    await this.request((id) => new Open(id, library.id, library.stamp), undefined);
  }

  /** Lets an opened library go. */
  closeLibrary(library: RemoteLibrary): void {
    this.send(new Close(library.id, library.stamp));
  }

  /** One round trip, in milliseconds. */
  async ping(): Promise<number> {
    const start = performance.now();
    await this.request((id) => new Ping(id), undefined);
    return performance.now() - start;
  }

  private catalogOf(value: unknown): RemoteCatalog {
    const bad = (why: string): BadValue => new BadValue(`the catalog: ${why}`);
    const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
    const isMap = (v: unknown): v is Record<string, unknown> =>
      typeof v === 'object' && v !== null && !Array.isArray(v) && !(v instanceof Uint8Array);
    if (!isMap(value)) throw bad('not a map');
    const kinds = new Map<string, RemoteKind>();
    for (const k of list(value['kinds'])) {
      const kind = RemoteKind.fromDescription(k);
      kinds.set(kind.name, kind);
    }
    this.kinds.clear();
    for (const [name, kind] of kinds) this.kinds.set(name, kind);
    const libraries: RemoteLibrary[] = [];
    for (const l of list(value['libraries'])) {
      if (!isMap(l)) throw bad('a library is not a map');
      const providers: RemoteProvider[] = [];
      for (const p of list(l['providers'])) {
        if (!isMap(p)) throw bad('a provider is not a map');
        const tables = new Map<string, RemoteTable>();
        for (const t of list(p['tables'])) {
          if (!isMap(t)) throw bad('a table is not a map');
          const kind = typeof t['kind'] === 'string' ? kinds.get(t['kind']) : undefined;
          const id = t['table'];
          if (kind === undefined || typeof id !== 'number') throw bad('a table names no kind the catalog describes');
          const slots = new Set(list(t['slots']).filter((s): s is string => typeof s === 'string'));
          tables.set(kind.name, new RemoteTable(this, kind, id, slots));
        }
        const meta = p['meta'];
        providers.push(
          new RemoteProvider(
            String(p['id']),
            String(p['name'] ?? p['id']),
            Object.freeze(isMap(meta) ? { ...meta } : {}),
            tables,
          ),
        );
      }
      libraries.push(
        new RemoteLibrary(
          String(l['id']),
          String(l['version'] ?? ''),
          String(l['stamp'] ?? ''),
          Object.freeze(providers),
        ),
      );
    }
    return new RemoteCatalog(kinds, Object.freeze(libraries));
  }

  /** @internal */
  isRunning(call: number): boolean {
    return this.running.has(call);
  }

  /** @internal */
  forget(handle: RemoteObject): void {
    this.finalizer.unregister(handle);
  }

  private async permit(): Promise<void> {
    if (this.inFlight < this.theirCalls) {
      this.inFlight++;
      return;
    }
    await new Promise<void>((resolve, reject) => this.permits.push({ resolve, reject }));
  }

  private unpermit(): void {
    const next = this.permits.shift();
    if (next !== undefined) next.resolve();
    else this.inFlight--;
  }

  /**
   * Sends the frame `build` makes for a new call id and answers its REPLY,
   * refusing a failed one with its status. `unsent` runs when the frame
   * never left.
   */
  private async request(
    build: (id: number) => Frame,
    slot: RemoteSlot | undefined,
    unsent?: () => void,
  ): Promise<Reply> {
    let body: Uint8Array;
    let id: number;
    try {
      if (!this.alive) throw new Gone(this.why_!);
      id = this.nextCall++;
      body = build(id).encode();
      if (body.length > this.sendMax) {
        throw new BadValue(`a frame of ${body.length} bytes is over the ${this.sendMax} the peer reads`);
      }
    } catch (e) {
      unsent?.();
      throw e;
    }
    const answered = new Promise<Reply>((resolve, reject) => {
      const pending: Pending = { slot, resolve, reject, timer: undefined };
      this.pending.set(id, pending);
      if (this.timeout !== null) {
        pending.timer = setTimeout(() => {
          if (!this.pending.delete(id)) return;
          if (slot !== undefined) {
            if (this.cancelled.size >= MAX_CANCELLED) this.cancelled.delete(this.cancelled.keys().next().value!);
            this.cancelled.set(id, slot);
          }
          this.send(new Cancel(id));
          reject(new Gone("no answer within the call's timeout"));
        }, this.timeout);
      }
    });
    try {
      this.channel.send(body);
    } catch (e) {
      const pending = this.pending.get(id);
      this.pending.delete(id);
      if (pending?.timer !== undefined) clearTimeout(pending.timer);
      unsent?.();
      // `end` rejects what was waiting; this call answers its own Gone.
      answered.catch(() => undefined);
      this.end(`a write failed: ${String(e)}`);
      throw new Gone(this.why_!);
    }
    const reply = await answered;
    if (reply.status !== 0) {
      let words: string;
      try {
        words = reply.words();
      } catch {
        words = 'the peer refused';
      }
      throw errorFor(reply.status, words);
    }
    return reply;
  }

  private kindNamed(name: string | undefined): RemoteKind {
    const kind = name === undefined ? undefined : this.kinds.get(name);
    if (kind === undefined) throw new NotFound(`no kind \`${name}\` in the catalog: list() first`);
    return kind;
  }

  private exportOf(served: Served, kind: RemoteKind, lent: boolean): number {
    if (served.kind !== kind.name) throw new WrongKind(`a \`${served.kind}\` where a \`${kind.name}\` is taken`);
    const id = this.nextExport++;
    this.exports.set(id, { served, kind, lent });
    return id;
  }

  /** The inbound call `during` names, if it is this link's and still runs; else 0. */
  private parentOf(during: InboundCall | undefined): number {
    return during !== undefined && during.link === this && this.running.has(during.id) ? during.id : 0;
  }

  /** @internal */
  async callOn(
    target: Target,
    slot: RemoteSlot,
    args: readonly unknown[],
    during: InboundCall | undefined,
  ): Promise<unknown> {
    if (!this.alive) throw new Gone(this.why_!);
    if (args.length !== slot.args.length) {
      throw new TypeError(`${args.length} arguments for \`${slot.name}\`, which takes ${slot.args.length}`);
    }
    const parent = this.parentOf(during);
    if (parent === 0) await this.permit();
    const lent: number[] = [];
    const handed: number[] = [];
    const givenBack: RemoteObject[] = [];
    try {
      const wire = [...args];
      try {
        this.objectsOut(slot, wire, lent, handed, givenBack);
      } catch (e) {
        for (const id of handed) this.exports.delete(id);
        throw e;
      }
      let sent = true;
      let reply: Reply;
      try {
        reply = await this.request(
          (id) => new Call(id, parent, target, slot.index, encodeArgs(slot, wire)),
          slot,
          () => {
            sent = false;
            for (const id of handed) this.exports.delete(id);
          },
        );
      } finally {
        // Handed back to its owner, who keeps it: nothing to release.
        if (sent) {
          for (const v of givenBack) {
            v.gone = true;
            this.forget(v);
          }
        }
      }
      const decoded = decodeAnswer(slot, reply.body);
      let w = 0;
      slot.args.forEach((a, i) => {
        if (isTree(a.type) && a.pass === 'lent') {
          writeBack(args[i], decoded.writebacks[w++]);
        } else if (a.type === 'bytes_mut') {
          const into = args[i] as Uint8Array;
          const from = decoded.writebacks[w++] as Uint8Array;
          into.set(from.subarray(0, Math.min(from.length, into.length)));
        }
      });
      return this.answerOf(slot, decoded.answer);
    } finally {
      for (const id of lent) this.exports.delete(id);
      if (parent === 0) this.unpermit();
    }
  }

  /**
   * Puts each object argument in `wire` as it crosses: a `Served`
   * exported (its id in `lent` or `handed`), a handle on the peer's object
   * as the peer's own (in `givenBack` when taken).
   */
  private objectsOut(
    slot: RemoteSlot,
    wire: unknown[],
    lent: number[],
    handed: number[],
    givenBack: RemoteObject[],
  ): void {
    wire.forEach((v, i) => {
      const a = slot.args[i]!;
      if (a.type !== 'object') return;
      if (v instanceof Served) {
        const id = this.exportOf(v, this.kindNamed(a.kind), a.pass === 'lent');
        (a.pass === 'lent' ? lent : handed).push(id);
        wire[i] = new ObjectRef(id);
      } else if (v instanceof RemoteObject && v.link === this) {
        if (v.gone) throw new Gone('the object has been let go');
        wire[i] = new ObjectRef(v.id, true);
        if (a.pass === 'taken') givenBack.push(v);
      }
    });
  }

  private answerOf(slot: RemoteSlot, answer: unknown): unknown {
    if (!(answer instanceof ObjectRef)) return answer;
    if (answer.receivers) {
      const own = this.exports.get(answer.id);
      this.exports.delete(answer.id);
      if (own === undefined) throw new Gone('the object has been let go');
      return own.served;
    }
    return this.handle(this.kindNamed(slot.answer?.kind), answer.id, false);
  }

  private handle(kind: RemoteKind, id: number, lent: boolean): RemoteObject {
    const handle = new RemoteObject(this, kind, id, lent);
    if (!lent) this.finalizer.register(handle, id, handle);
    return handle;
  }

  private route(body: Uint8Array): void {
    if (!this.alive) return;
    if (body.length > this.maxFrame) {
      this.goodbye(Status.ERR_BAD_VALUE, `a frame of ${body.length} bytes is over the ${this.maxFrame} allowed`);
      return;
    }
    let frame: AnyFrame;
    try {
      frame = Frame.decode(body);
    } catch (e) {
      // Whatever the bytes broke, the link ends: nothing escapes into the transport.
      this.goodbye(Status.ERR_BAD_VALUE, `the peer sent a malformed frame: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    if (frame instanceof Reply) {
      const pending = this.pending.get(frame.call);
      this.pending.delete(frame.call);
      if (pending !== undefined) {
        if (pending.timer !== undefined) clearTimeout(pending.timer);
        pending.resolve(frame);
      } else {
        const slot = this.cancelled.get(frame.call);
        this.cancelled.delete(frame.call);
        if (slot !== undefined) this.settleCancelled(slot, frame);
      }
    } else if (frame instanceof Pong) {
      const pending = this.pending.get(frame.n);
      this.pending.delete(frame.n);
      if (pending !== undefined) {
        if (pending.timer !== undefined) clearTimeout(pending.timer);
        pending.resolve(new Reply(frame.n, 0, new Uint8Array(0)));
      }
    } else if (frame instanceof Call) {
      this.inbound(frame);
    } else if (frame instanceof ListCatalog || frame instanceof Open) {
      this.refuse(frame.call, Status.ERR_UNSUPPORTED, 'this side serves no catalog');
    } else if (frame instanceof Cancel) {
      if (this.running.has(frame.call)) this.running.set(frame.call, true);
    } else if (frame instanceof Release) {
      this.exports.delete(frame.object);
    } else if (frame instanceof Ping) {
      this.send(new Pong(frame.n));
    } else if (frame instanceof Goodbye) {
      this.end(`the peer ended the link: ${frame.message}`);
    } else if (frame instanceof Hello || frame instanceof Welcome || frame instanceof Proof) {
      this.goodbye(Status.ERR_BAD_VALUE, 'a handshake frame after the handshake');
    }
    // CLOSE: this side serves no library.
  }

  /** A cancelled call's late answer: an object in it is let go, the peer's released and one of this side's dropped. */
  private settleCancelled(slot: RemoteSlot, reply: Reply): void {
    if (reply.status !== 0 || slot.answer?.type !== 'object') return;
    try {
      const answer = decodeAnswer(slot, reply.body).answer;
      if (!(answer instanceof ObjectRef)) return;
      if (answer.receivers) this.exports.delete(answer.id);
      else this.send(new Release(answer.id));
    } catch {
      // Nobody waits for it.
    }
  }

  /** A REPLY refusing `call`; a status that is no `guatiao_status` (not a u32) goes as INTERNAL. */
  private refuse(call: number, status: number, why: string): void {
    const code = Number.isInteger(status) && status > 0 && status <= 0xffffffff ? status : Status.ERR_INTERNAL;
    this.send(new Reply(call, code, new FrameWriter().text(why).done()));
  }

  private inbound(c: Call): void {
    if (c.parent !== 0 && !this.pending.has(c.parent)) {
      this.refuse(c.call, Status.ERR_GONE, 'the call it was made during has returned');
      return;
    }
    const top = c.parent === 0;
    if (top && this.runningTop >= this.maxCalls) {
      this.refuse(c.call, Status.ERR_BUSY, 'the peer is running as many calls as it allows');
      return;
    }
    if (top) this.runningTop++;
    this.running.set(c.call, false);
    void this.serve(c)
      .catch((e: unknown) => this.end(`serving a call of the peer failed: ${String(e)}`))
      .finally(() => {
        this.running.delete(c.call);
        if (top) this.runningTop--;
      });
  }

  private async serve(c: Call): Promise<void> {
    const target = c.target;
    const exported = target.object ? this.exports.get(target.id) : undefined;
    if (exported === undefined) {
      this.refuse(
        c.call,
        target.object ? Status.ERR_GONE : Status.ERR_NOT_FOUND,
        target.object ? 'the object has been let go' : 'no such table on this link: this side serves no providers',
      );
      return;
    }
    const slot = exported.kind.slots[c.slot];
    if (slot === undefined) {
      this.refuse(c.call, Status.ERR_NOT_FOUND, 'the kind has no such slot');
      return;
    }
    const handler = exported.served.slots[slot.name];
    if (handler === undefined) {
      this.refuse(
        c.call,
        Status.ERR_UNSUPPORTED,
        `the \`${exported.kind.name}\` this side serves does not fill \`${slot.name}\``,
      );
      return;
    }
    let args: unknown[];
    const lentHandles: RemoteObject[] = [];
    try {
      args = decodeArgs(slot, c.args);
      args.forEach((ref, i) => {
        if (!(ref instanceof ObjectRef)) return;
        const a = slot.args[i]!;
        if (ref.receivers) {
          const own = this.exports.get(ref.id);
          if (a.pass !== 'lent') this.exports.delete(ref.id);
          if (own === undefined) throw new Gone('the object has been let go');
          args[i] = own.served;
        } else {
          const handle = this.handle(this.kindNamed(a.kind), ref.id, a.pass === 'lent');
          if (a.pass === 'lent') lentHandles.push(handle);
          args[i] = handle;
        }
      });
    } catch (e) {
      if (e instanceof GuatiaoError) this.refuse(c.call, e.status, e.message);
      else this.refuse(c.call, Status.ERR_INTERNAL, String(e));
      return;
    }
    let answer: unknown;
    try {
      answer = await handler(args, new InboundCall(this, c.call));
    } catch (e) {
      if (this.running.get(c.call) !== true) {
        if (e instanceof GuatiaoError) this.refuse(c.call, e.status, e.message);
        else this.refuse(c.call, Status.ERR_INTERNAL, String(e));
      }
      return;
    } finally {
      for (const h of lentHandles) h.gone = true;
    }
    if (this.running.get(c.call) === true || !this.alive) return;
    let exportedId: number | undefined;
    try {
      let wire = answer;
      const givenBack = answer instanceof RemoteObject && answer.link === this ? answer : undefined;
      if (answer instanceof Served) {
        exportedId = this.exportOf(answer, this.kindNamed(slot.answer?.kind), false);
        wire = new ObjectRef(exportedId);
      } else if (givenBack !== undefined) {
        wire = new ObjectRef(givenBack.id, true);
      }
      const reply = new Reply(c.call, 0, encodeAnswer(slot, wire, args)).encode();
      if (reply.length > this.sendMax) {
        throw new BadValue(`an answer of ${reply.length} bytes is over the ${this.sendMax} the peer reads`);
      }
      if (givenBack !== undefined) {
        givenBack.gone = true;
        this.forget(givenBack);
      }
      this.sendBody(reply);
    } catch (e) {
      if (exportedId !== undefined) this.exports.delete(exportedId);
      if (e instanceof GuatiaoError) this.refuse(c.call, e.status, e.message);
      else if (e instanceof TypeError || e instanceof RangeError) this.refuse(c.call, Status.ERR_BAD_VALUE, e.message);
      else this.refuse(c.call, Status.ERR_INTERNAL, String(e));
    }
  }
}

/** What a lent tree came back as, written into the one the caller lent. */
function writeBack(into: unknown, from: unknown): void {
  if (Array.isArray(into) && Array.isArray(from)) {
    into.length = 0;
    into.push(...(from as unknown[]));
  } else if (into instanceof Map && typeof from === 'object' && from !== null) {
    into.clear();
    for (const [k, v] of Object.entries(from)) into.set(k, v);
  } else if (typeof into === 'object' && into !== null && isPlainObject(into) && typeof from === 'object' && from !== null) {
    const target = into as Record<string, unknown>;
    for (const k of Object.keys(target)) delete target[k];
    for (const [k, v] of Object.entries(from)) define(target, k, v);
  }
}
