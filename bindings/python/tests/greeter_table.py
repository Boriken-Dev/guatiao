# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`greeter_vtable`, written from
`examples/greeter_kind/include/greeter_kind.h`.

Not currently reachable through any `Registry` export for a
`#[derive(Provider)]`-based library -- see `test_greeter.py`. Kept as
the ctypes mirror of the real shape (checked against the header by
`test_greeter.py`'s `test_floor_hash_and_size_match_the_header`), so a
future C export that fetches `ProviderInfo.tables[i]` by kind has
something to test against without redesigning this file.
"""

from __future__ import annotations

import ctypes

from guatiao import _abi

GreetFn = ctypes.CFUNCTYPE(
    ctypes.c_uint32,
    ctypes.c_void_p,
    _abi.Str,
    ctypes.POINTER(_abi.Map),
    ctypes.POINTER(_abi.ProviderError),
)
ShoutFn = ctypes.CFUNCTYPE(
    ctypes.c_uint32, ctypes.c_void_p, _abi.Str, ctypes.POINTER(_abi.String)
)
StartFn = ctypes.CFUNCTYPE(
    ctypes.c_uint32,
    ctypes.c_void_p,
    _abi.Object,
    ctypes.POINTER(_abi.Object),
    ctypes.POINTER(_abi.ProviderError),
)


class GreeterVtable(ctypes.Structure):
    _fields_ = [
        ("header", _abi.KindHeader),
        ("greet", GreetFn),
        ("shout", ShoutFn),
        ("start", StartFn),
    ]
