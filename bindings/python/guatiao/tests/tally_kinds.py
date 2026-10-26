# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The tally kinds, born in Python: written as classes with no floor hash,
the way examples/tally_kind declares them in TypeScript. Described, they
are that example's committed document, byte for byte."""

from __future__ import annotations

from typing import Protocol

from guatiao import Map
from guatiao.kinds import I64, U32, KindProtocol, Lent, MapAnswer, Taken, fallible, required


class Watcher(KindProtocol, Protocol, kind="watcher", is_object=True):
    """What a tally tells whoever is watching it. Implemented by the host."""

    @required
    def changed(self, name: str, total: I64) -> None:
        """`name` now stands at `total`."""


class Tally(KindProtocol, Protocol, kind="tally"):
    """Counts by name."""

    @required
    @fallible
    def add(self, name: str, by: U32) -> I64:
        """Adds `by` to the count under `name`: the new count, or why not."""

    @required
    def total(self, name: str) -> I64:
        """The count under `name`: 0 when there is none."""

    def all(self) -> MapAnswer:
        """Appended: every count, as `{name: count}`."""

    def known(self, name: str) -> bool:
        """Appended: whether `name` has a count."""

    def checksum(self, data: bytes) -> U32:
        """Appended: the sum of the bytes, modulo 2^32."""

    @fallible
    def report(self, watcher: Lent[Watcher]) -> None:
        """Appended: tells `watcher` each count, for this call alone."""

    @fallible
    def load(self, counts: Taken[Map]) -> U32:
        """Appended: adds the counts of `counts` (`{name: count}`), which the
        tally owns from the call on; answers how many names it held."""
