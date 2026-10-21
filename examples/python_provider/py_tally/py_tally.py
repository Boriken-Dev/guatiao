# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A second Python library: one counter, starting where its configuration
says. Its own manifest, its own path, its own module."""

from __future__ import annotations

import threading
from typing import TypedDict

from greeter_kind import Counter

from guatiao import provide

__version__ = "0.1.0"
__all__ = ["Tally"]


class TallyConfig(TypedDict, total=False):
    """Where counting starts; left out, at zero."""

    start: int


class Tally(provide.Provider, Counter):
    """Counts from `start`, one instance per configuration."""

    def __init__(self, config: TallyConfig) -> None:
        self._next = config.get("start", 0)
        self._lock = threading.Lock()

    def count(self) -> int:
        with self._lock:
            self._next += 1
            return self._next
