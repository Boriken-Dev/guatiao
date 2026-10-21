# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The kinds of `include/greeter_kind.h`, as typed Protocol classes.

Written by hand from the header: each method in the order its table
declares it, its shapes in the annotations, each `floor_hash` copied from
the `#define` beside the table. A test compares the literals with the
header, so a kind that changes shape fails there rather than in a call.
A host calls a greeter through `Greeter`; a Python greeter inherits it.
"""

from __future__ import annotations

from typing import Protocol

from guatiao import Map
from guatiao.kinds import I64, KindProtocol, Lent, MapAnswer, Taken, fallible, required


class Listener(KindProtocol, Protocol, kind="listener", floor_hash=1867921573, is_object=True):
    """`listener_vtable`: what a conversation tells, implemented by the host."""

    @required
    def heard(self, what: str) -> None: ...


class Conversation(KindProtocol, Protocol, kind="conversation", floor_hash=2769624503, is_object=True):
    """`conversation_vtable`: one conversation a greeter started."""

    @required
    @fallible
    def say(self, what: str) -> None: ...

    @required
    def read(self, dst: bytearray) -> I64: ...

    @required
    def turns(self) -> I64: ...


class Greeter(KindProtocol, Protocol, kind="greeter", floor_hash=2695856491):
    """`greeter_vtable`: greets by name. `greet` is the one slot every table
    has; the rest were appended, and an older table may lack them."""

    @required
    @fallible
    def greet(self, name: str) -> MapAnswer: ...

    def shout(self, name: str) -> str: ...

    @fallible
    def start(self, listener: Taken[Listener]) -> Conversation: ...

    @fallible
    def greet_into(self, name: str, listener: Lent[Listener]) -> None: ...

    @fallible
    def greet_from(self, config: Taken[Map]) -> MapAnswer: ...

    @fallible
    def check(self, config: Lent[Map]) -> bool: ...


class Counter(KindProtocol, Protocol, kind="counter", floor_hash=1227528937):
    """`counter_vtable`: counts how often it was asked."""

    @required
    def count(self) -> I64: ...


__all__ = ["Listener", "Conversation", "Greeter", "Counter"]
