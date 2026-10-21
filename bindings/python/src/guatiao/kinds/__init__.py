# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A kind's function table, from Python: calling one, and serving one.

A kind is described once, by hand, from its C header: the table's
`ctypes.Structure`, its `FLOOR_HASH`, and a `Method` per slot saying what
it takes and answers. That description calls a provider's table
(`Kind.bind`, or `Registry.bind`) with Python values in and out, and
builds a table over a Python object, so a Python class can be an object
a library is handed, or a provider (`guatiao.provide`).

```python
from guatiao import kinds
from guatiao.kinds import MAP, STR, TEXT, Method, lent

GREETER = kinds.Kind(
    "greeter", GreeterVtable, floor_hash=2695856491,
    methods=[
        Method("greet", [STR], MAP, fallible=True, required=True),
        Method("shout", [STR], TEXT),
        Method("check", [lent(MAP)], kinds.BOOL, fallible=True),
    ],
)
greeter = reg.bind("acme_greeter", GREETER)
greeter.greet("ana").to_python()      # {"greeting": "hello, ana"}
```
"""

from __future__ import annotations

from ._shapes import (
    BOOL,
    BYTES,
    BYTES_MUT,
    F32,
    F64,
    I8,
    I16,
    I32,
    I64,
    ISIZE,
    LIST,
    MAP,
    STR,
    TEXT,
    U8,
    U16,
    U32,
    U64,
    USIZE,
    VALUE,
    Shape,
    lent,
    taken,
)
from ._spec import Kind, Method, Object, Remote, live_objects
from ._table import FloorMismatch, table

__all__ = [
    "Kind",
    "Method",
    "Remote",
    "Object",
    "Shape",
    "FloorMismatch",
    "table",
    "taken",
    "lent",
    "live_objects",
    "BOOL",
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
    "F64",
    "STR",
    "BYTES",
    "BYTES_MUT",
    "TEXT",
    "VALUE",
    "MAP",
    "LIST",
]
