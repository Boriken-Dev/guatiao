# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Greeters written in Python: the same kinds the Rust and C greeters
serve, offered by a library any host can load.

`library` is what a host registers: in a Python host,
`Registry.register_library(py_greeter.library)`; in any host, through the
guatiao-py source, which reads `py_greeter.guatiao-py.toml` beside this
file and imports `library` from here.
"""

from __future__ import annotations

import threading

from greeter_kind import COUNTER, GREETER

from guatiao import BadValue, provide

#: Changed by the examples to show a reload serving new code.
GREETING = "hello"


class Conversation:
    """One conversation `start` hands back: an object of the
    conversation kind, holding the listener it was handed."""

    def __init__(self, listener) -> None:
        self.listener = listener
        self.transcript: list = []

    def say(self, what: str) -> None:
        if not what:
            raise BadValue("nothing to say")
        self.transcript.append(what)
        self.listener.heard(what)

    def read(self, dst: bytearray) -> int:
        text = "\n".join(self.transcript).encode("utf-8")
        n = min(len(text), len(dst))
        dst[:n] = text[:n]
        return n

    def turns(self) -> int:
        return len(self.transcript)


class Hello:
    """One provider serving two kinds: a greeter and a counter."""

    def __init__(self) -> None:
        self._asked = 0
        self._lock = threading.Lock()

    def greet(self, name: str) -> dict:
        if name == "panic":
            raise RuntimeError("a provider bug, on request")
        if not name:
            raise BadValue("nobody to greet")
        return {"greeting": f"{GREETING}, {name}"}

    def shout(self, name: str) -> str:
        return self.greet(name)["greeting"].upper()

    def start(self, listener):
        return Conversation(listener)

    def greet_into(self, name: str, listener) -> None:
        listener.heard(self.greet(name)["greeting"])

    def greet_from(self, config):
        name = config.as_map()["name"].str_or("") if "name" in config.as_map() else ""
        config.close()
        return self.greet(name)

    def check(self, config) -> bool:
        if "name" not in config:
            config["name"] = "world"
        name = config["name"].str_or("")
        config["greeting"] = f"{GREETING}, {name}"
        return bool(name)

    def count(self) -> int:
        with self._lock:
            self._asked += 1
            return self._asked


class Shouter:
    """Built from a configuration: one instance per configuration."""

    def __init__(self, config: dict) -> None:
        self.prefix = config["prefix"]

    def greet(self, name: str) -> dict:
        return {"greeting": f"{self.prefix} {name.upper()}"}


library = provide.library(
    "py_greeter",
    "1.0.0",
    providers=[
        provide.Provider("hello", Hello, kinds=[GREETER, COUNTER], display_name="Hello, in Python"),
        provide.Provider(
            "shouter",
            Shouter,
            kinds=[GREETER],
            config={
                "type": "object",
                "properties": {"prefix": {"type": "string"}},
                "required": ["prefix"],
                "additionalProperties": False,
            },
        ),
    ],
)
