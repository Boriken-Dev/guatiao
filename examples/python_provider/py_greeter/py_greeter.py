# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Greeters written in Python: the same kinds the Rust and C greeters
serve, offered by a library any host can load.

The module is the library: its id is its name, its version `__version__`,
its providers the `provide.Provider` classes in `__all__`. A Python host
registers it with `Registry.register_library(py_greeter)`; any host loads
it through the guatiao-source-py source, which reads `py_greeter.guatiao.toml`
beside this file and imports it.
"""

from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Dict, List

from greeter_kind import Conversation, Counter, Greeter, Listener

from guatiao import BadValue, Map, provide

__version__ = "1.0.0"
__all__ = ["Hello", "Shouter"]

#: Changed by the examples to show a reload serving new code.
GREETING = "hello"
MARK = ""


class Chat(Conversation):
    """One conversation `start` hands back, holding the listener it was
    handed. Inheriting `Conversation` is for the type checker; any object
    with these methods would serve."""

    def __init__(self, listener: Listener) -> None:
        self.listener = listener
        self.transcript: List[str] = []

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


class Hello(provide.Provider, Greeter, Counter, display_name="Hello, in Python"):
    """One provider serving two kinds, its own one instance."""

    def __init__(self) -> None:
        self._asked = 0
        self._lock = threading.Lock()

    def greet(self, name: str) -> Dict[str, str]:
        if name == "panic":
            raise RuntimeError("a provider bug, on request")
        if not name:
            raise BadValue("nobody to greet")
        return {"greeting": f"{GREETING}, {name}"}

    def shout(self, name: str) -> str:
        return self.greet(name)["greeting"].upper()

    def start(self, listener: Listener) -> Chat:
        return Chat(listener)

    def greet_into(self, name: str, listener: Listener) -> None:
        listener.heard(self.greet(name)["greeting"])

    def greet_from(self, config: Map) -> Dict[str, str]:
        return self.greet(str(config.get("name", "")))

    def check(self, config: Map) -> bool:
        name = str(config.get("name", "world"))
        config["name"] = name
        config["greeting"] = f"{GREETING}, {name}"
        return bool(name)

    def count(self) -> int:
        with self._lock:
            self._asked += 1
            return self._asked


@dataclass(frozen=True)
class ShouterConfig:
    """What a shouter is built from; its schema is read from these fields."""

    prefix: str


class Shouter(provide.Provider, Greeter):
    """Built from a configuration: one instance per configuration."""

    def __init__(self, config: ShouterConfig) -> None:
        self.prefix = config.prefix

    def greet(self, name: str) -> Dict[str, str]:
        return {"greeting": f"{self.prefix} {name.upper()}{MARK}"}
