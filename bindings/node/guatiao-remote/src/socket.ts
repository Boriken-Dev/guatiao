// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

/**
 * A WebSocket as a message transport: the browser's, or Node's own (22 and
 * later). Each frame's body is one binary message.
 *
 * @module
 */

import { Gone } from './errors.js';
import type { RemoteChannel } from './link.js';

/** Opens a WebSocket to `url` within `timeout` milliseconds. */
export function openWebSocket(url: string, timeout: number): Promise<RemoteChannel> {
  return new Promise((resolve, reject) => {
    let ws: WebSocket;
    try {
      ws = new globalThis.WebSocket(url);
    } catch (e) {
      reject(new Gone(`the WebSocket to ${url} did not open: ${String(e)}`));
      return;
    }
    ws.binaryType = 'arraybuffer';
    let opened = false;
    let message: ((body: Uint8Array) => void) | undefined;
    let closed: ((why: string) => void) | undefined;
    let ended: string | undefined;
    const early: Uint8Array[] = [];
    const end = (why: string): void => {
      if (ended !== undefined) return;
      ended = why;
      closed?.(why);
    };
    const channel: RemoteChannel = {
      start(onMessage, onClosed) {
        message = onMessage;
        closed = onClosed;
        for (const body of early.splice(0)) onMessage(body);
        if (ended !== undefined) onClosed(ended);
      },
      send(body) {
        ws.send(body as Uint8Array<ArrayBuffer>);
      },
      close() {
        ws.close(1000);
      },
    };
    const timer = setTimeout(() => {
      if (opened) return;
      ws.close();
      reject(new Gone(`the WebSocket to ${url} did not open in time`));
    }, timeout);
    ws.onopen = () => {
      clearTimeout(timer);
      opened = true;
      resolve(channel);
    };
    ws.onerror = () => {
      if (!opened) {
        clearTimeout(timer);
        reject(new Gone(`the WebSocket to ${url} did not open`));
      }
    };
    ws.onmessage = (event: MessageEvent) => {
      if (ended !== undefined) return;
      if (!(event.data instanceof ArrayBuffer)) {
        // A text message is no frame: the link ends. A page may close
        // with 1000 or 3000-4999 only.
        end('a text message: frames are binary');
        ws.close(1000);
        return;
      }
      const body = new Uint8Array(event.data);
      if (message !== undefined) message(body);
      else early.push(body);
    };
    ws.onclose = (event: CloseEvent) => {
      clearTimeout(timer);
      if (!opened) reject(new Gone(`the WebSocket to ${url} closed as it opened`));
      end(event.reason === '' ? `the WebSocket closed (${event.code})` : event.reason);
    };
  });
}
