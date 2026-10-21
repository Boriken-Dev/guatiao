# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The shapes a slot can carry, for building a `Kind` by hand with
`Method(...)`: what a kind class's annotations are read into.

`STR`, `BYTES`, `BYTES_MUT`, `TEXT` (a `String` answer), `VALUE` and
`MAP` (borrowed arguments, or answers), `LIST` (an answer), the scalars
`BOOL`, `I8` .. `U64`, `USIZE`, `ISIZE`, `F32`, `F64`, and `taken(x)` /
`lent(x)` for a tree or an object kind handed over or lent.
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

__all__ = [
    "Shape",
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
    "taken",
    "lent",
]
