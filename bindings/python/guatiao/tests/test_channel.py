# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A host written in Python hands a sink to the Rust echo and receives on
the echo's own thread: the channel's rules, from Python. The sink is a
Python object the echo calls from a thread Python did not start, which
the GIL allows."""

from __future__ import annotations

import sys
import threading
import time
from typing import List, Optional

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native
from guatiao import kinds
from guatiao.errors import Gone
from guatiao.registry import Registry

_KIND = native.REPO / "examples" / "channel_echo" / "python"
sys.path.insert(0, str(_KIND))

from channel_echo_kind import Channel, ChannelEcho, ChannelSink  # noqa: E402

GONE = 12
GRACE = 0.3
WAIT = 30.0


class Sink(ChannelSink):
    """What the echo writes back on, seen from the test."""

    def __init__(self) -> None:
        self.cond = threading.Condition()
        self.received: List[bytes] = []
        self.closed_with: Optional[int] = None
        self.closed_reason: Optional[str] = None
        self.broken: List[str] = []
        self.in_call = False
        self.held = False
        self.away = False
        self.threads = set()

    def _enter(self) -> None:
        self.threads.add(threading.get_ident())
        if self.in_call:
            self.broken.append("a call came while another was running")
        self.in_call = True

    def receive(self, bytes: bytes) -> None:
        with self.cond:
            self._enter()
            if self.closed_with is not None:
                self.broken.append("receive after closed")
            while self.held:
                self.cond.wait()
            if self.away:
                self.in_call = False
                raise Gone("the host takes nothing more")
            self.received.append(bytes)
            self.in_call = False
            self.cond.notify_all()

    def closed(self, status: int, reason: str) -> None:
        with self.cond:
            self._enter()
            if self.closed_with is not None:
                self.broken.append("closed twice")
            self.closed_with = status
            self.closed_reason = reason
            self.in_call = False
            self.cond.notify_all()

    def wait_for(self, done) -> bool:
        deadline = time.monotonic() + WAIT
        with self.cond:
            while not done():
                left = deadline - time.monotonic()
                if left <= 0:
                    return False
                self.cond.wait(left)
        return True


@pytest.fixture()
def echo():
    with Registry("py-channel", "0.1") as registry:
        registry.scan_dir(native.library_dir(), rules="kind=channel_echo")
        yield registry.bind("channel_echo_echo", ChannelEcho.kind)


def test_messages_come_back_in_order_on_the_echos_own_thread(echo):
    sink = Sink()
    with echo.start({}, sink) as channel:
        sent = [f"m{i}".encode() for i in range(20)]
        for message in sent:
            channel.send(message)
        assert sink.wait_for(lambda: len(sink.received) == 20)
        assert sink.received == sent
        assert threading.get_ident() not in sink.threads, "called on the echo's thread"
        channel.close(0, "")
        assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == 0
    assert not sink.broken


def test_a_sink_that_takes_its_time_makes_the_sender_wait(echo):
    sink = Sink()
    sink.held = True
    channel = echo.start({}, sink)
    sent = [f"p{i}".encode() for i in range(24)]
    done = threading.Event()

    def sender() -> None:
        for message in sent:
            channel.send(message)
        done.set()

    thread = threading.Thread(target=sender)
    thread.start()
    time.sleep(1.0)
    assert not done.is_set(), "the sender waits for room, it is not refused"
    assert sink.closed_with is None
    with sink.cond:
        sink.held = False
        sink.cond.notify_all()
    thread.join(WAIT)
    assert sink.wait_for(lambda: len(sink.received) == 24)
    assert sink.received == sent
    channel.close(0, "")
    assert sink.wait_for(lambda: sink.closed_with is not None)


def test_the_hosts_close_ends_sends_and_closes_once_with_its_status_and_reason(echo):
    sink = Sink()
    channel = echo.start({}, sink)
    for text in (b"1", b"2", b"3"):
        channel.send(text)
    channel.close(5, "the host gave up")
    with pytest.raises(Gone):
        channel.send(b"4")
    assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == 5
    assert sink.closed_reason == "the host gave up"
    channel.destroy()
    time.sleep(GRACE)
    assert not sink.broken
    assert sink.received == [b"1", b"2", b"3"][: len(sink.received)]


def test_dropping_the_channel_is_close_with_ok_and_lets_the_sink_go(echo):
    sink = Sink()
    before = kinds.live_objects()
    channel = echo.start({}, sink)
    channel.send(b"1")
    assert sink.wait_for(lambda: len(sink.received) == 1)
    channel.destroy()
    assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == 0
    assert sink.closed_reason == ""
    deadline = time.monotonic() + WAIT
    while kinds.live_objects() != before and time.monotonic() < deadline:
        time.sleep(0.01)
    assert kinds.live_objects() == before, "the echo destroyed the sink"


def test_the_provider_closes_with_its_own_status_and_reason(echo):
    sink = Sink()
    channel = echo.start(
        {"close_after": 2, "close_status": 7, "close_reason": "the provider failed"}, sink
    )
    channel.send(b"1")
    channel.send(b"2")
    assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == 7
    assert sink.closed_reason == "the provider failed"
    with pytest.raises(Gone):
        channel.send(b"3")
    channel.destroy()
    assert sink.received == [b"1", b"2"]


def test_a_sink_that_has_gone_ends_the_stream_with_gone(echo):
    sink = Sink()
    channel = echo.start({}, sink)
    channel.send(b"1")
    assert sink.wait_for(lambda: len(sink.received) == 1)
    sink.away = True
    channel.send(b"2")
    assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == GONE
    with pytest.raises(Gone):
        channel.send(b"3")
    channel.destroy()
    assert sink.received == [b"1"]


def test_close_and_drop_do_not_wait_for_a_receive_still_running(echo):
    sink = Sink()
    sink.held = True
    channel = echo.start({}, sink)
    channel.send(b"x")
    time.sleep(GRACE)
    started = time.monotonic()
    channel.close(3, "closing while a receive runs")
    channel.destroy()
    took = time.monotonic() - started
    with sink.cond:
        sink.held = False
        sink.cond.notify_all()
    assert took < 3 * GRACE
    assert sink.wait_for(lambda: sink.closed_with is not None)
    assert sink.closed_with == 3
    assert not sink.broken
