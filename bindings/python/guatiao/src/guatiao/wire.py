# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Channels: a schema announced before its values, so a receiver knows
what each value means and refuses what does not fit it.

A frame is ``kind:u8 channel:LEB128 payload`` (kind 0 a schema, 1 a
value, 2 the channel's end); where one ends is the transport's to say.
`announce`, `send` and `close` write frames; a `Receiver` reads them and
answers an `Event`, or raises the refusal: `Malformed` and `Invalid` are
`BadValue`s, `NoSchema` a `NotFound`, `NotASchema` a `WrongKind`. A
refusal quotes nothing it was given. A single value in the wire encoding
is `Value.from_wire(data)` and `value.to_wire()`.
"""

from __future__ import annotations

import ctypes
import enum
import threading
from typing import Any, Callable, NamedTuple, Optional

from . import _abi, _lib
from .errors import BadValue, GuatiaoError, NotFound, WrongKind, raise_for_status
from .value import Ref, Value, _buffer_view, _free, _make_bytes, _node_to_python

__all__ = [
    "Frame",
    "Event",
    "Receiver",
    "announce",
    "send",
    "close",
    "Refused",
    "Malformed",
    "NoSchema",
    "NotASchema",
    "Invalid",
]


class Frame(enum.IntEnum):
    """A frame's kind byte: `GUATIAO_FRAME_*`."""

    SCHEMA = 0
    VALUE = 1
    CLOSE = 2


# A `guatiao_wire_event`'s `refusal`: `GUATIAO_WIRE_*`.
_ACCEPTED = 0
_MALFORMED = 1
_NO_SCHEMA = 2
_NOT_A_SCHEMA = 3
_INVALID = 4


class Event(NamedTuple):
    """What a frame said. `value` is the schema now the channel's, or the
    value its schema accepted, as a `Value` the caller closes; `None` for
    the end of a channel."""

    kind: Frame
    channel: int
    value: Optional[Value]


class Refused(GuatiaoError):
    """A frame the receiver refused. `channel` is the channel it named (0
    when it ended before one) and `kind` its kind byte."""

    def __init__(
        self,
        status: "int | str | None" = None,
        message: str = "",
        *,
        channel: int = 0,
        kind: int = 0,
    ) -> None:
        super().__init__(status, message)
        self.channel = channel
        self.kind = kind


class Malformed(Refused, BadValue):
    """Bytes that are not a frame; `at` is the offset of the byte in the
    frame it went wrong at."""

    def __init__(self, status: "int | str | None" = None, message: str = "", *, at: int = 0, **kw: Any) -> None:
        super().__init__(status, message, **kw)
        self.at = at


class NoSchema(Refused, NotFound):
    """A value on a channel no schema was announced on."""


class NotASchema(Refused, WrongKind):
    """A schema frame whose value is not a schema."""


class Invalid(Refused, BadValue):
    """A value its channel's schema refuses: `key` is the field, and
    `expected` what it accepts or `known` the fields there are."""

    def __init__(
        self,
        status: "int | str | None" = None,
        message: str = "",
        *,
        key: str = "",
        expected: Optional[str] = None,
        known: "Optional[list[str]]" = None,
        **kw: Any,
    ) -> None:
        super().__init__(status, message, **kw)
        self.key = key
        self.expected = expected
        self.known = known


def _frame(build: Any, *args: Any) -> bytes:
    lib = _lib.core()
    out = _abi.Value()
    status = build(ctypes.byref(lib.alloc), *args, ctypes.byref(out))
    try:
        raise_for_status(status)
        return _buffer_view(out.payload.bytes)
    finally:
        _free(out)


def _with_node(value: Any, call: Callable[[Any], bytes]) -> bytes:
    """Calls `call` with a pointer to `value`'s node, converting a plain
    Python object into a temporary `Value` first."""
    if isinstance(value, Ref):
        return call(value._ptr())
    with Value(value) as temporary:
        return call(temporary._ptr())


def announce(channel: int, schema: Any) -> bytes:
    """The frame that announces `schema` (a `Value`, a `Ref` or what
    `Value(...)` accepts) on `channel`. A later one replaces it."""
    lib = _lib.core()
    return _with_node(schema, lambda node: _frame(lib.guatiao_wire_announce, channel, node))


def send(channel: int, value: Any) -> bytes:
    """The frame that carries `value` on `channel`."""
    lib = _lib.core()
    return _with_node(value, lambda node: _frame(lib.guatiao_wire_send, channel, node))


def close(channel: int) -> bytes:
    """The frame that ends `channel`: a receiver forgets its schema."""
    return _frame(_lib.core().guatiao_wire_close, channel)


def _text(node: "_abi.Value") -> str:
    found = _node_to_python(node, "str")
    return found if isinstance(found, str) else ""


class Receiver:
    """Keeps each channel's schema and checks every value against it.

    Close it with `close()` or a ``with`` block. One receiver serves one
    conversation; calls on it are serialised."""

    def __init__(self) -> None:
        lib = _lib.core()
        self._lock = threading.Lock()
        self._handle: Optional[int] = lib.guatiao_wire_receiver_new(ctypes.byref(lib.alloc))
        if not self._handle:
            raise BadValue("the allocator was refused")

    def accept(self, frame: bytes) -> Event:
        """Reads one frame: an `Event`, or the refusal raised."""
        lib = _lib.core()
        view, _buf = _make_bytes(bytes(frame))
        event = _abi.WireEvent()
        with self._lock:
            if not self._handle:
                raise ValueError("this receiver is closed")
            status = lib.guatiao_wire_receiver_accept(self._handle, view, ctypes.byref(event))
        try:
            message = _text(event.message)
            detail = _node_to_python(event.detail, "str")
            value = None
            if status == 0 and event.value.tag != _abi.Tag.ABSENT:
                value = Value(_raw=_abi.Value.from_buffer_copy(event.value))
                event.value = _abi.Value()
        finally:
            _free(event.value)
            _free(event.message)
            _free(event.detail)
        if status == 0:
            return Event(Frame(event.kind), int(event.channel), value)
        where = {"channel": int(event.channel), "kind": int(event.kind)}
        if event.refusal == _MALFORMED:
            raise Malformed(status, message, at=int(event.at), **where)
        if event.refusal == _NO_SCHEMA:
            raise NoSchema(status, message, **where)
        if event.refusal == _NOT_A_SCHEMA:
            raise NotASchema(status, message, **where)
        if event.refusal == _INVALID:
            detail = detail if isinstance(detail, dict) else {}
            raise Invalid(
                status,
                message,
                key=detail.get("key", ""),
                expected=detail.get("expected"),
                known=detail.get("known"),
                **where,
            )
        raise_for_status(status, message)
        raise AssertionError("unreachable: a refused status raises")

    def close(self) -> None:
        with self._lock:
            handle, self._handle = self._handle, None
        if handle:
            _lib.core().guatiao_wire_receiver_free(handle)

    def __enter__(self) -> "Receiver":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass
