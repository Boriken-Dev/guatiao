# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A provider's kind table, called from Python -- proof the C surface
carries a consumer that is not Rust or C.

**Deviation from the sub-plan's literal steps, discovered while writing
this test, recorded here rather than silently worked around:**
`#[derive(Provider)]` always files its kind tables under
`ProviderInfo.tables` (`crates/guatiao-derive/src/provider.rs`), never
the legacy single `vtable` field. `guatiao_registry_provider_vtable` --
the only C export for a provider's table, and what `Registry.provider_table`
wraps -- reads `vtable` alone; there is no export that fetches
`tables[i]` by kind name. So `derived_greeter`'s `greeter_vtable`
(`examples/greeter_kind/include/greeter_kind.h`, the shape this slice
was written against) is not reachable through this binding as the C ABI
stands today -- confirmed below, not hidden. Not fixable from
`bindings/python/`.

`hello_library`'s table IS reachable: it is hand-written straight into
`vtable` (`examples/hello_library/src/lib.rs`'s own `GreeterVtable` --
`struct_size`, `greet`, `outstanding`; no `KindHeader`/`floor_hash`, a
different, older shape than `greeter_kind.h`'s). It stands in for the
end-to-end call this slice asks for, appended slot included, and its
greeting text is the same `"hello, <name>"` pattern
`derived_library_load.rs` asserts for the kind-macro path.
"""

from __future__ import annotations

import ctypes
import os
import re
from pathlib import Path

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import greeter_table
from guatiao import _abi
from guatiao.registry import Registry
from guatiao.value import Value

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]
_GREETER_KIND_H = (
    Path(__file__).resolve().parents[3]
    / "examples"
    / "greeter_kind"
    / "include"
    / "greeter_kind.h"
)


def _floor_hash() -> int:
    text = _GREETER_KIND_H.read_text(encoding="utf-8")
    match = re.search(r"#define greeter_vtable_FLOOR_HASH (\d+)", text)
    assert match, "greeter_vtable_FLOOR_HASH not found in greeter_kind.h"
    return int(match.group(1))


def test_floor_hash_is_parsed_and_the_struct_matches_the_header():
    # The struct in greeter_table.py must stay the size the header's
    # own `greeter_vtable` declares: header(8) + 3 function pointers.
    assert ctypes.sizeof(greeter_table.GreeterVtable) == 8 + 3 * ctypes.sizeof(
        ctypes.c_void_p
    )
    assert _floor_hash() > 0


def test_a_derived_providers_table_is_not_reachable_via_provider_table():
    reg = Registry("py-tests", "0.1")
    try:
        reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        for key in ("derived_greeter_hello", "derived_greeter_shouter"):
            table, _size, _ctx = reg.provider_table(key)
            assert not table, f"{key}: expected unreachable, got a table"
    finally:
        reg.close()


class _HelloGreeterVtable(ctypes.Structure):
    """`examples/hello_library/src/lib.rs`'s own `GreeterVtable` --
    unrelated to `#[guatiao::kind]`, hence no `KindHeader` here."""

    _fields_ = [
        ("struct_size", ctypes.c_uint32),
        (
            "greet",
            ctypes.CFUNCTYPE(
                ctypes.c_uint32,
                ctypes.c_void_p,
                ctypes.POINTER(_abi.Value),
                ctypes.POINTER(_abi.Value),
            ),
        ),
        ("outstanding", ctypes.CFUNCTYPE(ctypes.c_int64, ctypes.c_void_p)),
    ]


def test_hello_library_greeter_called_live_from_python():
    reg = Registry("py-tests", "0.1")
    try:
        reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        raw_table, size, ctx = reg.provider_table("hello_library_greeter")
        assert raw_table
        assert size >= ctypes.sizeof(_HelloGreeterVtable)
        table = ctypes.cast(raw_table, ctypes.POINTER(_HelloGreeterVtable)).contents

        with Value.from_python({"name": "ana"}) as config:
            out = _abi.Value()
            status = table.greet(ctx, ctypes.byref(config._raw), ctypes.byref(out))
        assert status == 0
        with Value(_raw=out) as answer:
            assert answer.to_python() == {"greeting": "hello, ana"}

        assert table.outstanding(ctx) >= 0
    finally:
        reg.close()
