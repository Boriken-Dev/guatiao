# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Run by `test_app_library.py` in a process of its own: maps the
application's library with the `ctypes` mode named in `argv[1]` before the
binding opens it, then calls plugins that link guatiao themselves."""

from __future__ import annotations

import ctypes
import os
import sys

app = ctypes.CDLL(os.environ["GUATIAO_LIBRARY"], mode=getattr(ctypes, sys.argv[1]))
app.myapp_version.restype = ctypes.c_uint32
assert app.myapp_version() == 1

import greeter_table  # noqa: E402
import native  # noqa: E402
from guatiao import _abi, _lib  # noqa: E402
from guatiao.registry import Registry  # noqa: E402
from guatiao.value import Value, _make_str  # noqa: E402

assert _lib.core().path == os.environ["GUATIAO_LIBRARY"], _lib.core().path

with Registry("app-library", "0.1") as reg:
    reg.scan_dir(native.library_dir(), rules="kind=greeter")
    raw, size, ctx = reg.provider_table("derived_greeter_hello", kind="greeter")
    table = ctypes.cast(raw, ctypes.POINTER(greeter_table.GreeterVtable)).contents
    name, _keep = _make_str(b"ana")
    out = _abi.Map()
    err = _abi.ProviderError()
    assert table.greet(ctx, name, ctypes.byref(out), ctypes.byref(err)) == 0
    node = _abi.Value(tag=int(_abi.Tag.MAP), payload=_abi.Payload(map=out))
    with Value(_raw=node) as answer:
        assert answer.to_python() == {"greeting": "hello, ana"}
    with Value.from_python({"made": [1, "two"]}) as built:
        assert Value.from_wire(built.to_wire()).to_python() == {"made": [1, "two"]}
print("ok")
