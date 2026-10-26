// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * The remote protocol's frames as bytes, with no I/O. A frame here is a
 * body: a message transport (a WebSocket) carries it as one message, with
 * no length in front.
 *
 * ```text
 * uint  := LEB128, shortest form, at most 64 bits   int := uint, zigzagged
 * text  := len:uint UTF-8     bytes := len:uint octets
 * value := len:uint wire-encoded value
 * ```
 *
 * An integer is a `number` while it is a safe integer, a `bigint` past it.
 *
 * @module
 */

import { BadValue } from './errors.js';
import { Bytes, fromWire, toWire } from './wire.js';

/** Why bytes are not a frame, named as the shared vectors name it. */
export type FrameFault =
  | 'unknown_frame'
  | 'not_this_protocol'
  | 'version'
  | 'overlong'
  | 'truncated'
  | 'trailing'
  | 'not_utf8'
  | 'field'
  | 'wire'
  | 'too_large';

/**
 * Bytes a peer sent that are not a frame of this protocol: `fault` says
 * why, `at` the byte of the body it went wrong at, when it says.
 */
export class MalformedFrame extends BadValue {
  readonly fault: FrameFault;
  readonly at: number | undefined;

  constructor(fault: FrameFault, message: string, at?: number) {
    super(message);
    this.fault = fault;
    this.at = at;
  }
}

/** The four bytes a handshake starts with: "GTRM". */
export const MAGIC = new Uint8Array([0x47, 0x54, 0x52, 0x4d]);

/** This protocol's version. */
export const VERSION = 1;

/** The largest body a peer reads before the handshake ends. */
export const HANDSHAKE_MAX_FRAME = 64 << 10;

/** Frame types. */
export const FrameType = {
  HELLO: 0x01,
  WELCOME: 0x02,
  PROOF: 0x03,
  LIST: 0x04,
  OPEN: 0x05,
  CLOSE: 0x06,
  CALL: 0x10,
  REPLY: 0x11,
  CANCEL: 0x12,
  RELEASE: 0x13,
  PING: 0x14,
  PONG: 0x15,
  GOODBYE: 0x16,
} as const;

/** An integer as the codec carries it. */
export type Int = number | bigint;

const U64_MAX = (1n << 64n) - 1n;
const I64_MIN = -(1n << 63n);
const I64_MAX = (1n << 63n) - 1n;

/** `big` as a `number` when it is a safe integer. */
export function narrow(big: bigint): Int {
  return big >= BigInt(Number.MIN_SAFE_INTEGER) && big <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(big) : big;
}

/** An integer as a `bigint`; `undefined` for anything else. */
export function bigOf(n: unknown): bigint | undefined {
  if (typeof n === 'bigint') return n;
  if (typeof n === 'number' && Number.isInteger(n)) return BigInt(n);
  return undefined;
}

const utf8 = new TextEncoder();
// Refuses what is not UTF-8, and keeps a leading U+FEFF: it is text.
const strict = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/** A body being written. */
export class FrameWriter {
  private readonly out = new Bytes();

  byte(b: number): this {
    this.out.byte(b);
    return this;
  }

  raw(bytes: Uint8Array): this {
    this.out.raw(bytes);
    return this;
  }

  /** An unsigned integer from 0 to 2^64 - 1. */
  uint(n: Int): this {
    if (typeof n === 'number' && Number.isSafeInteger(n) && n >= 0) {
      this.out.count(n);
      return this;
    }
    const big = bigOf(n);
    if (big === undefined || big < 0n || big > U64_MAX) {
      throw new RangeError(`${String(n)} is not an unsigned 64-bit integer`);
    }
    let v = big;
    while (v >= 128n) {
      this.out.byte(Number(v & 0x7fn) | 0x80);
      v >>= 7n;
    }
    this.out.byte(Number(v));
    return this;
  }

  /** A signed integer within 64 bits, zigzagged. */
  int(n: Int): this {
    if (typeof n === 'number' && Number.isInteger(n) && Math.abs(n) < 2 ** 52) {
      return this.uint(n >= 0 ? n * 2 : -n * 2 - 1);
    }
    const big = bigOf(n);
    if (big === undefined || big < I64_MIN || big > I64_MAX) {
      throw new RangeError(`${String(n)} is not a signed 64-bit integer`);
    }
    return this.uint(big < 0n ? (-big << 1n) - 1n : big << 1n);
  }

  bytes(data: Uint8Array): this {
    this.uint(data.length);
    return this.raw(data);
  }

  text(s: string): this {
    return this.bytes(utf8.encode(s));
  }

  /** A value in the wire encoding, behind its length. */
  value(v: unknown): this {
    return this.bytes(toWire(v));
  }

  done(): Uint8Array {
    return this.out.done();
  }
}

/** A body being read. Every refusal names the byte it went wrong at. */
export class FrameReader {
  at = 0;
  constructor(readonly data: Uint8Array) {}

  get isEmpty(): boolean {
    return this.at === this.data.length;
  }

  /** What is left, unread. */
  rest(): Uint8Array {
    const out = this.data.slice(this.at);
    this.at = this.data.length;
    return out;
  }

  end(): void {
    if (!this.isEmpty) {
      throw new MalformedFrame('trailing', `bytes follow the last field, from ${this.at}`, this.at);
    }
  }

  private truncated(where: number): MalformedFrame {
    return new MalformedFrame('truncated', `the frame ends inside a field at ${where}`, where);
  }

  byte(): number {
    if (this.at >= this.data.length) throw this.truncated(this.at);
    return this.data[this.at++]!;
  }

  fixed(n: number): Uint8Array {
    if (this.at + n > this.data.length) throw this.truncated(this.at);
    const out = this.data.slice(this.at, this.at + n);
    this.at += n;
    return out;
  }

  private overlong(start: number): MalformedFrame {
    return new MalformedFrame('overlong', `an integer at ${start} is not in its shortest form`, start);
  }

  /**
   * An unsigned LEB128. Refused when longer than its shortest form or
   * wider than 64 bits.
   */
  uint(): Int {
    const start = this.at;
    let n = 0;
    let big: bigint | undefined;
    for (let shift = 0; shift < 64; shift += 7) {
      const b = this.byte();
      const bits = b & 0x7f;
      if (shift === 63 && bits > 1) throw this.overlong(start);
      if (big === undefined && shift <= 42) {
        n += bits * 2 ** shift;
      } else {
        big = (big ?? BigInt(n)) + (BigInt(bits) << BigInt(shift));
      }
      if ((b & 0x80) === 0) {
        if (b === 0 && shift > 0) throw this.overlong(start);
        return big === undefined ? n : narrow(big);
      }
    }
    throw this.overlong(start);
  }

  /** An unsigned LEB128 that must be a safe integer: an id, a count, a status. */
  small(what: string): number {
    const start = this.at;
    const n = this.uint();
    if (typeof n === 'number') return n;
    throw new MalformedFrame('field', `the field at ${start} is not ${what}`, start);
  }

  /** A zigzagged LEB128. */
  int(): Int {
    const z = this.uint();
    if (typeof z === 'number') return z % 2 === 0 ? z / 2 : -(z - 1) / 2 - 1;
    return narrow(z % 2n === 0n ? z >> 1n : -((z + 1n) >> 1n));
  }

  private len(): number {
    const start = this.at;
    const n = this.uint();
    if (typeof n !== 'number' || n > this.data.length - this.at) throw this.truncated(start);
    return n;
  }

  bytes(): Uint8Array {
    const n = this.len();
    const out = this.data.slice(this.at, this.at + n);
    this.at += n;
    return out;
  }

  text(): string {
    const start = this.at;
    const raw = this.bytes();
    try {
      return strict.decode(raw);
    } catch {
      throw new MalformedFrame('not_utf8', `text at ${start} is not UTF-8`, start);
    }
  }

  /** A value in the wire encoding, read into plain data. */
  value(): unknown {
    const start = this.at;
    const raw = this.bytes();
    try {
      return fromWire(raw);
    } catch (e) {
      if (e instanceof BadValue) throw new MalformedFrame('wire', `the value at ${start}: ${e.message}`, start);
      throw e;
    }
  }
}

/** Who a call is for, in the receiver's numbering: a provider's table or an object it holds. */
export class Target {
  private constructor(
    readonly id: number,
    readonly object: boolean,
  ) {}

  static table(id: number): Target {
    return new Target(id, false);
  }

  static object(id: number): Target {
    return new Target(id, true);
  }

  get code(): number {
    return this.object ? this.id * 2 + 1 : this.id * 2;
  }

  static of(code: number): Target {
    return code % 2 === 1 ? Target.object((code - 1) / 2) : Target.table(code / 2);
  }

  toString(): string {
    return this.object ? `object ${this.id}` : `table ${this.id}`;
  }
}

/** One frame's body. A call's arguments and a reply's answer stay bytes: they are read against the slot's description. */
export abstract class Frame {
  abstract readonly type: number;
  protected abstract fields(out: FrameWriter): void;

  /** The body: the type byte, then the fields. */
  encode(): Uint8Array {
    const out = new FrameWriter().byte(this.type);
    this.fields(out);
    return out.done();
  }

  /** Reads one body. */
  static decode(body: Uint8Array): AnyFrame {
    const r = new FrameReader(body);
    const type = r.byte();
    let frame: AnyFrame;
    switch (type) {
      case FrameType.HELLO:
      case FrameType.WELCOME: {
        const m = r.fixed(4);
        for (let i = 0; i < 4; i++) {
          if (m[i] !== MAGIC[i]) {
            throw new MalformedFrame('not_this_protocol', "the peer does not speak guatiao's remote protocol");
          }
        }
        const v = r.byte();
        if (v !== VERSION) throw new MalformedFrame('version', `the peer speaks version ${v}, this side ${VERSION}`);
        if (type === FrameType.HELLO) {
          const at = r.at;
          const trust = r.byte();
          if (trust > 3) throw new MalformedFrame('field', `the field at ${at} is not a trust level`, at);
          const nonce = r.fixed(16);
          frame = new Hello(trust, nonce, r.value());
        } else {
          const nonce = r.fixed(16);
          const proof = r.bytes();
          frame = new Welcome(nonce, proof, r.value());
        }
        break;
      }
      case FrameType.PROOF:
        frame = new Proof(r.bytes());
        break;
      case FrameType.LIST:
        frame = new ListCatalog(r.small('a call id'));
        break;
      case FrameType.OPEN:
        frame = new Open(r.small('a call id'), r.text(), r.text());
        break;
      case FrameType.CLOSE:
        frame = new Close(r.text(), r.text());
        break;
      case FrameType.CALL: {
        const call = r.small('a call id');
        const parent = r.small('a call id');
        const target = Target.of(r.small('a target'));
        const slot = r.small('a slot');
        return new Call(call, parent, target, slot, r.rest());
      }
      case FrameType.REPLY: {
        const call = r.small('a call id');
        const status = status32(r);
        return new Reply(call, status, r.rest());
      }
      case FrameType.CANCEL:
        frame = new Cancel(r.small('a call id'));
        break;
      case FrameType.RELEASE:
        frame = new Release(r.small('an object id'));
        break;
      case FrameType.PING:
        frame = new Ping(r.small('a ping number'));
        break;
      case FrameType.PONG:
        frame = new Pong(r.small('a ping number'));
        break;
      case FrameType.GOODBYE: {
        const status = status32(r);
        frame = new Goodbye(status, r.text());
        break;
      }
      default:
        throw new MalformedFrame(
          'unknown_frame',
          `frame type 0x${type.toString(16).padStart(2, '0')} is not one this version defines`,
        );
    }
    r.end();
    return frame;
  }
}

function status32(r: FrameReader): number {
  const at = r.at;
  const status = r.uint();
  if (typeof status !== 'number' || status > 0xffffffff) {
    throw new MalformedFrame('field', `the field at ${at} is not a status`, at);
  }
  return status;
}

/** The client's first frame: `trust` 0 local, 1 trusted, 2 secure (mutual TLS), 3 server TLS. */
export class Hello extends Frame {
  readonly type = FrameType.HELLO;
  constructor(
    readonly trust: number,
    readonly nonce: Uint8Array,
    readonly options: unknown,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.raw(MAGIC).byte(VERSION).byte(this.trust).raw(this.nonce).value(this.options);
  }
}

/** The server's answer to a HELLO. */
export class Welcome extends Frame {
  readonly type = FrameType.WELCOME;
  constructor(
    readonly nonce: Uint8Array,
    readonly proof: Uint8Array,
    readonly options: unknown,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.raw(MAGIC).byte(VERSION).raw(this.nonce).bytes(this.proof).value(this.options);
  }
}

/** The client's proof of the key. */
export class Proof extends Frame {
  readonly type = FrameType.PROOF;
  constructor(readonly proof: Uint8Array) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.bytes(this.proof);
  }
}

/** Asks the peer's catalog. */
export class ListCatalog extends Frame {
  readonly type = FrameType.LIST;
  constructor(readonly call: number) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.call);
  }
}

/** Opens a listed library. */
export class Open extends Frame {
  readonly type = FrameType.OPEN;
  constructor(
    readonly call: number,
    readonly library: string,
    readonly stamp: string,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.call).text(this.library).text(this.stamp);
  }
}

/** Lets an opened library go. */
export class Close extends Frame {
  readonly type = FrameType.CLOSE;
  constructor(
    readonly library: string,
    readonly stamp: string,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.text(this.library).text(this.stamp);
  }
}

/** A call of a slot; `args` are typed by the slot's description. */
export class Call extends Frame {
  readonly type = FrameType.CALL;
  constructor(
    readonly call: number,
    readonly parent: number,
    readonly target: Target,
    readonly slot: number,
    readonly args: Uint8Array,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.call).uint(this.parent).uint(this.target.code).uint(this.slot).raw(this.args);
  }
}

/** The answer to a call, a list or an open: the answer (status 0) or the message. */
export class Reply extends Frame {
  readonly type = FrameType.REPLY;
  constructor(
    readonly call: number,
    readonly status: number,
    readonly body: Uint8Array,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.call).uint(this.status).raw(this.body);
  }

  /** A refusal's words. */
  words(): string {
    const r = new FrameReader(this.body);
    const words = r.text();
    r.end();
    return words;
  }
}

/** The caller no longer waits for a call. */
export class Cancel extends Frame {
  readonly type = FrameType.CANCEL;
  constructor(readonly call: number) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.call);
  }
}

/** An object the receiver gave is let go. */
export class Release extends Frame {
  readonly type = FrameType.RELEASE;
  constructor(readonly object: number) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.object);
  }
}

/** Asks for a PONG. */
export class Ping extends Frame {
  readonly type = FrameType.PING;
  constructor(readonly n: number) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.n);
  }
}

/** The answer to a PING. */
export class Pong extends Frame {
  readonly type = FrameType.PONG;
  constructor(readonly n: number) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.n);
  }
}

/** The link is ending. */
export class Goodbye extends Frame {
  readonly type = FrameType.GOODBYE;
  constructor(
    readonly status: number,
    readonly message: string,
  ) {
    super();
  }
  protected fields(out: FrameWriter): void {
    out.uint(this.status).text(this.message);
  }
}

/** Every frame this version defines. */
export type AnyFrame =
  | Hello
  | Welcome
  | Proof
  | ListCatalog
  | Open
  | Close
  | Call
  | Reply
  | Cancel
  | Release
  | Ping
  | Pong
  | Goodbye;

/** A REPLY refusing `call` with `status` and `message`. */
export function refusal(call: number, status: number, message: string): Reply {
  return new Reply(call, status, new FrameWriter().text(message).done());
}
