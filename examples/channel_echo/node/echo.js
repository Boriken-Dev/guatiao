// @ts-check
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// An echo serving a channel, in the shape the Node binding gives
// providers: the same file runs under Node and under the
// guatiao-source-js source (QuickJS). Every message sent comes back
// through the host's sink inside the `send` that took it: neither engine
// calls a provider's object from a thread of its own.

import { Gone } from 'guatiao';
import { Provider } from 'guatiao/provide';

import { ChannelEcho } from './channel_echo_kind.js';

/** A count from the configuration, by the name it is spelt with: a plain
 *  object under QuickJS, a `MapRef` of `Ref`s under Node. */
function count(config, name) {
  const camel = name.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
  const value = typeof config.get === 'function' ? config.get(name) : (config[name] ?? config[camel]);
  if (value === undefined || value === null) return undefined;
  return Number(typeof value.toJS === 'function' ? value.toJS() : value);
}

/** The channel `start` hands back: the sink it was given, until the stream ends. */
class Channel {
  #sink;
  #echoed = 0;
  #closeAfter;
  #closeStatus;

  constructor(sink, closeAfter, closeStatus) {
    this.#sink = sink;
    this.#closeAfter = closeAfter;
    this.#closeStatus = closeStatus;
  }

  /** Ends the stream once: the sink hears `closed(status)`, then is let go. */
  #finish(status) {
    const sink = this.#sink;
    if (sink === null) return;
    this.#sink = null;
    try {
      sink.closed(status);
    } finally {
      sink.close();
    }
  }

  /** @param {Uint8Array} bytes */
  send(bytes) {
    if (this.#sink === null) throw new Gone('the channel is closed');
    try {
      this.#sink.receive(bytes);
    } catch (e) {
      // Any refusal ends the stream; the message was taken all the same.
      this.#finish(Number(e.status ?? 0));
      return;
    }
    this.#echoed += 1;
    if (this.#closeAfter !== undefined && this.#echoed >= this.#closeAfter) {
      this.#finish(this.#closeStatus);
    }
  }

  /** @param {number} status */
  close(status) {
    this.#finish(status);
  }

  // Dropping the channel is `close(0)`.
  [Symbol.dispose]() {
    this.#finish(0);
  }
}

export class Echo extends Provider {
  static kinds = [ChannelEcho];
  static displayName = 'Echo, in JavaScript';

  start(config, sink) {
    return new Channel(sink, count(config, 'close_after'), count(config, 'close_status') ?? 0);
  }
}
