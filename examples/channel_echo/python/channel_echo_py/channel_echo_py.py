# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""An echo serving a channel from Python: every message sent comes back
through the host's sink, in order, from a thread this module starts for
the channel. It keeps every rule of `guatiao-channel`:

- `send` waits for room in a queue of a few messages, and `close` wakes a
  `send` that is waiting, which answers GONE;
- one thread calls the sink, in the order the messages were queued;
- `close` lets the queue drain, then the thread calls `closed` with the
  host's status and lets the sink go; dropping the channel is `close(0)`
  and does not wait for the thread;
- after `close_after` echoes the thread closes with `close_status`, and
  what was still queued is not delivered;
- an error from `receive` ends the stream: `closed` with that status (a
  Python exception without one is `GUATIAO_ERR_INTERNAL`, 100).

The library may be unmapped while this module's thread runs only if the
source lets it: guatiao-source-py keeps its interpreter and every module's
code for the life of the process, so the thread never runs unmapped code.
"""

from __future__ import annotations

import threading
from collections import deque
from typing import Deque, Optional

from channel_echo_kind import Channel, ChannelEcho, ChannelSink

from guatiao import Map, provide
from guatiao.errors import Gone

__version__ = "1.0.0"
__all__ = ["Echo"]

#: How many messages wait for the sink before `send` waits too.
QUEUE = 4


class _Queue:
    """What the channel and its thread share."""

    def __init__(self) -> None:
        self.cond = threading.Condition()
        self.items: Deque[bytes] = deque()
        #: Either side has ended the stream: `send` answers GONE.
        self.closed = False
        #: The status the host closed with, when it was the host.
        self.host_status: Optional[int] = None


class Echoing(Channel):
    """The host's end. The thread holds the queue, not this object, so
    dropping the channel is what closes it."""

    def __init__(self, queue: _Queue) -> None:
        self._queue = queue

    def send(self, bytes: bytes) -> None:
        q = self._queue
        with q.cond:
            while True:
                if q.closed:
                    raise Gone("the channel is closed")
                if len(q.items) < QUEUE:
                    break
                q.cond.wait()
            q.items.append(bytes)
            q.cond.notify_all()

    def close(self, status: int) -> None:
        q = self._queue
        with q.cond:
            if q.closed:
                return
            q.closed = True
            q.host_status = status
            q.cond.notify_all()

    def __del__(self) -> None:
        self.close(0)


def _deliver(
    sink: ChannelSink, q: _Queue, close_after: Optional[int], close_status: int
) -> None:
    """The channel's thread: the only caller of the sink."""
    echoed = 0
    while True:
        with q.cond:
            while True:
                if close_after is not None and echoed >= close_after:
                    message = None
                    break
                if q.items:
                    message = q.items.popleft()
                    q.cond.notify_all()
                    break
                if q.closed:
                    message = None
                    break
                q.cond.wait()
            if message is None:
                status = q.host_status if q.host_status is not None else close_status
                # What was queued and not delivered goes with the stream.
                q.closed = True
                q.items.clear()
                q.cond.notify_all()
                break
        try:
            sink.receive(message)
        except Exception as e:  # any refusal ends the stream, once
            status = int(getattr(e, "status", 100))
            with q.cond:
                q.closed = True
                q.items.clear()
                q.cond.notify_all()
            break
        echoed += 1
    try:
        sink.closed(status)
    finally:
        sink.destroy()


class Echo(provide.Provider, ChannelEcho, display_name="Echo, in Python"):
    """Starts an echoing channel per call."""

    def start(self, config: Map, sink: ChannelSink) -> Channel:
        close_after = config.get("close_after")
        close_status = config.get("close_status", 0)
        q = _Queue()
        threading.Thread(
            target=_deliver,
            args=(
                sink,
                q,
                None if close_after is None else int(close_after),
                int(close_status),
            ),
            name="channel_echo",
            daemon=True,
        ).start()
        return Echoing(q)
