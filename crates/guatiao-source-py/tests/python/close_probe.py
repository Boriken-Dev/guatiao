# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A library whose kind answers objects with a slot named `close`: the
kinds declared here hash as `tests/hosts.rs` declares them in Rust."""

from __future__ import annotations

from typing import Protocol

from guatiao import provide
from guatiao.kinds import I64, KindProtocol, fallible, required

__version__ = "0.1.0"
__all__ = ["Dialer"]


class Session(KindProtocol, Protocol, kind="session", is_object=True):
    """A line a caller talks on and then closes."""

    @required
    @fallible
    def send(self, what: str) -> None: ...

    @required
    @fallible
    def close(self) -> None: ...

    @required
    def said(self) -> I64: ...


class Opener(KindProtocol, Protocol, kind="opener"):
    """Opens sessions."""

    @required
    @fallible
    def open(self) -> Session: ...


class Line(Session):
    def __init__(self) -> None:
        self.count = 0
        self.closed = False

    def send(self, what: str) -> None:
        if self.closed:
            raise ValueError("the line is closed")
        self.count += 1

    def close(self) -> None:
        self.closed = True

    def said(self) -> int:
        return self.count


class Dialer(provide.Provider, Opener):
    def open(self) -> Line:
        return Line()
