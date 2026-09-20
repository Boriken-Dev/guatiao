# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A provider's kind table, called from Python: a table fetched by kind
and checked against the kind's header, and a provider that declares one
single table of its own shape."""

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


def test_a_table_is_fetched_by_kind_and_called():
    from guatiao import kinds
    from guatiao.value import _make_str

    with Registry("py-tests", "0.1") as reg:
        reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        key = "derived_greeter_hello"
        assert not reg.provider_table(key)[0], "it declares no single table"
        assert not reg.provider_table(key, kind="codec")[0]

        raw, size, ctx = reg.provider_table(key, kind="greeter")
        greeter = kinds.table(raw, size, greeter_table.GreeterVtable, floor_hash=_floor_hash())

        name, _keep = _make_str(b"ana")
        out = _abi.Map()
        err = _abi.ProviderError()
        assert greeter.greet(ctx, name, ctypes.byref(out), ctypes.byref(err)) == 0
        node = _abi.Value(tag=int(_abi.Tag.MAP), payload=_abi.Payload(map=out))
        with Value(_raw=node) as answer:
            assert answer.to_python() == {"greeting": "hello, ana"}


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
