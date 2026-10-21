# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A second Python library: one counter, starting where its configuration
says. Its own manifest, its own path, its own entry point."""

from __future__ import annotations

import threading

from greeter_kind import COUNTER

from guatiao import provide


class Tally:
    """Counts from `start`, one instance per configuration."""

    def __init__(self, config: dict) -> None:
        self._next = int(config.get("start", 0))
        self._lock = threading.Lock()

    def count(self) -> int:
        with self._lock:
            self._next += 1
            return self._next


library = provide.library(
    "py_tally",
    "0.1.0",
    providers=[
        provide.Provider(
            "tally",
            Tally,
            kinds=[COUNTER],
            config={
                "type": "object",
                "properties": {"start": {"type": "integer"}},
                "additionalProperties": False,
            },
        ),
    ],
)
