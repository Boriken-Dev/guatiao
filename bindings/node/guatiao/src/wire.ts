// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * Channels: a schema announced before its values, so a receiver knows what
 * each value means and refuses what does not fit it.
 *
 * A frame is `kind:u8 channel:LEB128 payload` (kind 0 a schema, 1 a value,
 * 2 the channel's end); the transport says where one ends. `announce`,
 * `send` and `close` write frames; a `Receiver` reads them and answers an
 * `Event`, or throws the refusal: `Malformed` and `Invalid` are
 * `BadValue`s, `NoSchema` a `NotFound`, `NotASchema` a `WrongKind`. A
 * refusal quotes nothing it was given. One value in the wire encoding is
 * `Value.fromWire(bytes)` and `value.toWire()`.
 *
 * @module
 */

import { SIZE, Status, Tag, VALUE, WIRE_EVENT } from './abi.js';
import { AllocFailed, BadValue, NotFound, WrongKind, raiseForStatus } from './errors.js';
import * as n from './native.js';
import { Borrowed, Ref, Value, adopt, withNodes } from './value.js';

/** A frame's kind byte: `GUATIAO_FRAME_*`. */
export const Frame = {
  SCHEMA: 0,
  VALUE: 1,
  CLOSE: 2,
} as const;
export type Frame = (typeof Frame)[keyof typeof Frame];

/** A `guatiao_wire_event`'s `refusal`: `GUATIAO_WIRE_*`. @internal */
export const Refusal = {
  ACCEPTED: 0,
  MALFORMED: 1,
  NO_SCHEMA: 2,
  NOT_A_SCHEMA: 3,
  INVALID: 4,
} as const;

/** A channel number: 64 bits, so a `bigint` past 2^53. */
export type Channel = number | bigint;

/**
 * What a frame said. `value` is the schema now the channel's, or the value
 * its schema accepted, as a `Value` the caller closes; `null` for a
 * channel's end.
 */
export interface Event {
  readonly kind: Frame;
  readonly channel: bigint;
  readonly value: Value | null;
}

/** Where a refused frame was: its channel (0 when it ended before one) and its kind byte. */
export interface Where {
  readonly channel: bigint;
  readonly kind: number;
}

/** Bytes that are not a frame; `at` is the byte of the frame it went wrong at. */
export class Malformed extends BadValue implements Where {
  readonly channel: bigint;
  readonly kind: number;
  readonly at: number;

  constructor(status: number, message: string, where: Where & { at: number }) {
    super(status, message);
    this.channel = where.channel;
    this.kind = where.kind;
    this.at = where.at;
  }
}

/** A value on a channel no schema was announced on. */
export class NoSchema extends NotFound implements Where {
  readonly channel: bigint;
  readonly kind: number;

  constructor(status: number, message: string, where: Where) {
    super(status, message);
    this.channel = where.channel;
    this.kind = where.kind;
  }
}

/** A schema frame whose value is not a schema. */
export class NotASchema extends WrongKind implements Where {
  readonly channel: bigint;
  readonly kind: number;

  constructor(status: number, message: string, where: Where) {
    super(status, message);
    this.channel = where.channel;
    this.kind = where.kind;
  }
}

/**
 * A value its channel's schema refuses: `key` is the field, and `expected`
 * what it accepts or `known` the fields there are.
 */
export class Invalid extends BadValue implements Where {
  readonly channel: bigint;
  readonly kind: number;
  readonly key: string;
  readonly expected: string | undefined;
  readonly known: readonly string[] | undefined;

  constructor(
    status: number,
    message: string,
    where: Where & { key: string; expected?: string | undefined; known?: readonly string[] | undefined },
  ) {
    super(status, message);
    this.channel = where.channel;
    this.kind = where.kind;
    this.key = where.key;
    this.expected = where.expected;
    this.known = where.known;
  }
}

/** Any refusal a `Receiver` throws. */
export type Refused = Malformed | NoSchema | NotASchema | Invalid;

/** A read-only `Ref` onto the node at `node`, which this module frees. */
const lent = (node: bigint): Ref => new Ref(new Borrowed(() => node), []);

function frame(build: (alloc: bigint, out: bigint) => number): Uint8Array {
  const core = n.core();
  const out = n.alloc(SIZE.value);
  try {
    raiseForStatus(build(core.alloc, out));
    return lent(out).toJS<Uint8Array>();
  } finally {
    n.freeNode(out);
    n.release(out);
  }
}

/**
 * The frame that announces `schema` (a `Value`, a `Ref` or plain data) on
 * `channel`. A later one replaces it.
 */
export function announce(channel: Channel, schema: unknown): Uint8Array {
  return withNodes([schema], ([node]) =>
    frame((alloc, out) => n.core().fn.guatiao_wire_announce!(alloc, BigInt(channel), node, out)),
  );
}

/** The frame that carries `value` on `channel`. */
export function send(channel: Channel, value: unknown): Uint8Array {
  return withNodes([value], ([node]) =>
    frame((alloc, out) => n.core().fn.guatiao_wire_send!(alloc, BigInt(channel), node, out)),
  );
}

/** The frame that ends `channel`: a receiver forgets its schema. */
export function close(channel: Channel): Uint8Array {
  return frame((alloc, out) => n.core().fn.guatiao_wire_close!(alloc, BigInt(channel), out));
}

const receivers = new FinalizationRegistry<bigint>((handle) => {
  n.core().fn.guatiao_wire_receiver_free!(handle);
});

/**
 * Keeps each channel's schema and checks every value against it. `close()`
 * (or `using`) releases it; a `FinalizationRegistry` is the backstop.
 */
export class Receiver implements Disposable {
  #handle: bigint;
  #closed = false;

  constructor() {
    const core = n.core();
    const handle = core.fn.guatiao_wire_receiver_new!(core.alloc) as bigint | null;
    if (!handle) throw new AllocFailed(Status.ERR_ALLOC, 'the allocator was refused');
    this.#handle = handle;
    receivers.register(this, handle, this);
  }

  /** Reads one frame: an `Event`, or the refusal thrown. */
  accept(data: Uint8Array): Event {
    if (this.#closed) throw new TypeError('this receiver is closed');
    const event = n.alloc(SIZE.wireEvent);
    try {
      const status = n.core().fn.guatiao_wire_receiver_accept!(this.#handle, n.bytesArg(data), event) as number;
      const value = event + BigInt(WIRE_EVENT.value);
      const message = event + BigInt(WIRE_EVENT.message);
      const detail = event + BigInt(WIRE_EVENT.detail);
      const words = lent(message).stringOr('');
      const found = lent(detail).toJS<Record<string, unknown> | undefined>({ numbers: 'text' });
      const channel = n.u64(event, WIRE_EVENT.channel);
      const kind = n.u32(event, WIRE_EVENT.kind);
      let taken: Value | null = null;
      if (status === Status.OK && n.u32(value, VALUE.tag) !== Tag.ABSENT) {
        const root = n.alloc(SIZE.value);
        n.copy(root, value, SIZE.value);
        n.zero(value, SIZE.value);
        taken = adopt(root, n.core().alloc);
      }
      for (const node of [value, message, detail]) n.freeNode(node);
      if (status === Status.OK) return { kind: kind as Frame, channel, value: taken };
      const where = { channel, kind };
      switch (n.u32(event, WIRE_EVENT.refusal)) {
        case Refusal.MALFORMED:
          throw new Malformed(status, words, { ...where, at: n.size(event, WIRE_EVENT.at) });
        case Refusal.NO_SCHEMA:
          throw new NoSchema(status, words, where);
        case Refusal.NOT_A_SCHEMA:
          throw new NotASchema(status, words, where);
        case Refusal.INVALID: {
          const known = found?.known;
          throw new Invalid(status, words, {
            ...where,
            key: typeof found?.key === 'string' ? found.key : '',
            expected: typeof found?.expected === 'string' ? found.expected : undefined,
            known: Array.isArray(known) ? (known as string[]) : undefined,
          });
        }
      }
      raiseForStatus(status, words);
      throw new Error('a refused status throws');
    } finally {
      n.release(event);
    }
  }

  get closed(): boolean {
    return this.#closed;
  }

  /** Frees the receiver and the schemas it keeps; a second call does nothing. */
  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    receivers.unregister(this);
    n.core().fn.guatiao_wire_receiver_free!(this.#handle);
  }

  [Symbol.dispose](): void {
    this.close();
  }
}
