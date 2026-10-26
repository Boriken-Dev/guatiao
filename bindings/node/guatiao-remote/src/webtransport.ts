// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * WebTransport as a message transport: a browser's session to an
 * `https://` URL, one bidirectional stream carrying each frame's body behind
 * its `u32` little-endian length, as the server's `quic://` does.
 *
 * @module
 */

import { BadValue, Gone, Unsupported } from './errors.js';
import type { RemoteChannel } from './link.js';

/** How long closing waits for what was written to leave, in milliseconds. */
const CLOSE_WAIT = 2_000;

/** A fingerprint's 32 bytes: what `serverCertificateHashes` names. */
function hashOf(pin: string): Uint8Array<ArrayBuffer> {
  const m = /^sha256:([0-9a-fA-F]{64})$/.exec(pin);
  if (m === null) throw new TypeError(`\`${pin}\` is not a certificate fingerprint, sha256:<64 hex digits>`);
  const hex = m[1]!;
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) out[i] = parseInt(hex.slice(2 * i, 2 * i + 2), 16);
  return out;
}

/** Bytes read and not yet a whole frame. */
class Pending {
  private buf = new Uint8Array(0);
  private len = 0;

  push(chunk: Uint8Array): void {
    if (this.len + chunk.length > this.buf.length) {
      const grown = new Uint8Array(Math.max(this.len + chunk.length, this.buf.length * 2, 1024));
      grown.set(this.buf.subarray(0, this.len));
      this.buf = grown;
    }
    this.buf.set(chunk, this.len);
    this.len += chunk.length;
  }

  /** The next whole frame's body, `undefined` while it has not all come, or a body too large. */
  next(maxFrame: number): Uint8Array | undefined {
    if (this.len < 4) return undefined;
    const size = new DataView(this.buf.buffer, this.buf.byteOffset, 4).getUint32(0, true);
    if (size > maxFrame) throw new BadValue(`a frame of ${size} bytes is larger than this side reads (${maxFrame})`);
    if (this.len < 4 + size) return undefined;
    const body = this.buf.slice(4, 4 + size);
    this.buf.copyWithin(0, 4 + size, this.len);
    this.len -= 4 + size;
    return body;
  }
}

/**
 * Opens a WebTransport session to `url` (`https://host:port/path`) within
 * `timeout` milliseconds, the server's certificate trusted by its
 * fingerprint when `pins` names any (`serverCertificateHashes`), else as the
 * browser trusts any. Frames over `maxFrame` bytes end the channel.
 */
export async function openWebTransport(
  url: string,
  timeout: number,
  pins: readonly string[],
  maxFrame: number,
): Promise<RemoteChannel> {
  const WT = (globalThis as { WebTransport?: typeof WebTransport }).WebTransport;
  if (WT === undefined) {
    throw new Unsupported("WebTransport is a browser's, and this runtime has none: connect over ws:// or wss://");
  }
  const hashes = pins.map((pin) => ({ algorithm: 'sha-256', value: hashOf(pin) }));
  let transport: WebTransport;
  try {
    transport = new WT(url, hashes.length > 0 ? { serverCertificateHashes: hashes } : {});
  } catch (e) {
    throw new Gone(`the WebTransport session to ${url} did not open: ${String(e)}`);
  }
  // A session that fails rejects both; `closed` is read below once open.
  transport.closed.catch(() => {});
  let timer: ReturnType<typeof setTimeout> | undefined;
  const late = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Gone(`the WebTransport session to ${url} did not open in time`)), timeout);
  });
  let reader: ReadableStreamDefaultReader<Uint8Array>;
  let writer: WritableStreamDefaultWriter<Uint8Array>;
  try {
    const stream = await Promise.race([
      transport.ready.then(() => transport.createBidirectionalStream()),
      late,
    ]);
    reader = stream.readable.getReader() as ReadableStreamDefaultReader<Uint8Array>;
    writer = stream.writable.getWriter();
  } catch (e) {
    try {
      transport.close();
    } catch {
      // Already failed.
    }
    if (e instanceof Gone) throw e;
    throw new Gone(`the WebTransport session to ${url} did not open: ${String(e)}`);
  } finally {
    clearTimeout(timer);
  }

  let message: ((body: Uint8Array) => void) | undefined;
  let closed: ((why: string) => void) | undefined;
  let ended: string | undefined;
  const early: Uint8Array[] = [];
  const end = (why: string): void => {
    if (ended !== undefined) return;
    ended = why;
    closed?.(why);
  };
  const shut = (why: string): void => {
    end(why);
    try {
      transport.close({ closeCode: 0, reason: '' });
    } catch {
      // Already closed.
    }
  };
  const deliver = (body: Uint8Array): void => {
    if (message !== undefined) message(body);
    else early.push(body);
  };

  void (async () => {
    const pending = new Pending();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        pending.push(value);
        for (let body = pending.next(maxFrame); body !== undefined; body = pending.next(maxFrame)) {
          if (ended !== undefined) return;
          deliver(body);
        }
      }
      end('the WebTransport stream ended');
    } catch (e) {
      if (e instanceof BadValue) shut(e.message);
      else end(`the WebTransport session ended: ${String(e)}`);
    }
  })();
  transport.closed.then(
    (info) =>
      end(
        info.reason !== undefined && info.reason !== ''
          ? info.reason
          : `the WebTransport session closed (${info.closeCode ?? 0})`,
      ),
    (e: unknown) => end(`the WebTransport session was lost: ${String(e)}`),
  );

  return {
    start(onMessage, onClosed) {
      message = onMessage;
      closed = onClosed;
      for (const body of early.splice(0)) onMessage(body);
      if (ended !== undefined) onClosed(ended);
    },
    send(body) {
      if (ended !== undefined) return;
      const frame = new Uint8Array(4 + body.length);
      new DataView(frame.buffer).setUint32(0, body.length, true);
      frame.set(body, 4);
      writer.write(frame).catch((e: unknown) => end(`the WebTransport stream refused a write: ${String(e)}`));
    },
    close() {
      if (ended !== undefined) return;
      end('this side closed the link');
      // What was written leaves before the session ends, unless the peer
      // stops reading.
      let done = false;
      const finish = (): void => {
        if (done) return;
        done = true;
        try {
          transport.close({ closeCode: 0, reason: '' });
        } catch {
          // Already closed.
        }
      };
      setTimeout(finish, CLOSE_WAIT);
      writer.close().then(finish, finish);
    },
  };
}
