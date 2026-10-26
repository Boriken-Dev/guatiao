# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A kind's function table, from Python: calling one, and serving one.

A kind is a typed `Protocol` class, written from its C header: a method
per slot in the header's order, the shapes read from the annotations,
and the `FLOOR_HASH` the header defines.

```python
from typing import Protocol

from guatiao import Map
from guatiao.kinds import KindProtocol, Lent, MapAnswer, fallible, required

class Listener(KindProtocol, Protocol, kind="listener", floor_hash=1867921573, is_object=True):
    @required
    def heard(self, what: str) -> None: ...

class Greeter(KindProtocol, Protocol, kind="greeter", floor_hash=2695856491):
    @required
    @fallible
    def greet(self, name: str) -> MapAnswer: ...
    def shout(self, name: str) -> str: ...
    @fallible
    def greet_into(self, name: str, listener: Lent[Listener]) -> None: ...
    @fallible
    def check(self, config: Lent[Map]) -> bool: ...

greeter = reg.bind("acme_greeter", Greeter)
greeter.greet("ana").to_python()      # {"greeting": "hello, ana"}
```

A kind that has a description (the `<crate>.kinds.json` a kind crate
commits, or what `Registry.kind_descriptions` answers) is loaded from it
instead: `kinds_of(document)` answers each `Kind` by name, and `python -m
guatiao.kinds DOCUMENT -o module.py` writes the classes above from it.

The class calls a provider's table (`Registry.bind`, `Kind.bind`) with
Python values in and out, and a Python class serves it: an object handed
to a library, or a provider (`guatiao.provide`). `Greeter.kind` is the
`Kind` it builds; `Kind(...)` with `Method`s and `kinds.shapes` is the
same description written by hand.
"""

from __future__ import annotations

from . import shapes
from ._protocol import (
    F32,
    I8,
    I16,
    I32,
    I64,
    ISIZE,
    U8,
    U16,
    U32,
    U64,
    USIZE,
    KindProtocol,
    Lent,
    ListAnswer,
    MapAnswer,
    Taken,
    ValueAnswer,
    fallible,
    kind_of,
    required,
)
from ._description import generate, kinds_of
from ._spec import Kind, Method, Object, Remote, live_objects
from ._table import FloorMismatch, table

__all__ = [
    "KindProtocol",
    "Lent",
    "Taken",
    "required",
    "fallible",
    "kind_of",
    "MapAnswer",
    "ListAnswer",
    "ValueAnswer",
    "I8",
    "I16",
    "I32",
    "I64",
    "U8",
    "U16",
    "U32",
    "U64",
    "USIZE",
    "ISIZE",
    "F32",
    "Kind",
    "Method",
    "Remote",
    "Object",
    "FloorMismatch",
    "table",
    "live_objects",
    "kinds_of",
    "generate",
    "shapes",
]
