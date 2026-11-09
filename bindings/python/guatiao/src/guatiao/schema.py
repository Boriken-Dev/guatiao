# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What a schema says about a value: here, which parts are secrets."""

from __future__ import annotations

import contextlib
import ctypes
from typing import Any

from . import _abi, _lib
from .errors import check
from .value import Ref, Value, _make_str, _reading

__all__ = ["differences", "redact"]


def redact(
    schema: Any,
    value: Any,
    placeholder: str = "[redacted]",
    *,
    alloc: "_abi.Alloc | None" = None,
) -> Value:
    """A copy of `value` with every secret replaced by `placeholder`.

    A secret is a field whose schema sets `x-sensitive` or `writeOnly`, at
    any depth: in a nested object, in every element of a list, in a
    variant's arm. Where the schema stops describing the value -- a key it
    does not declare, say -- the value is hidden too, since nothing says it
    is not a secret. Local `$ref`s are followed.

    `schema` and `value` are `Value`s or `Ref`s, or what `Value(...)`
    takes. The copy is a new `Value` the caller closes. Raises `WrongKind`
    for a schema that is not a map."""
    lib = _lib.core()
    alloc_struct = alloc if alloc is not None else lib.alloc
    owned = []
    reads = contextlib.ExitStack()
    try:
        pointers = []
        for item in (schema, value):
            if isinstance(item, Ref):
                pointers.append(reads.enter_context(_reading(item)))
            else:
                built = Value(item)
                owned.append(built)
                pointers.append(ctypes.byref(built._raw))
        view, _buf = _make_str(placeholder.encode("utf-8"))
        out = _abi.Value()
        check(
            lib.guatiao_schema_redact(
                pointers[0], pointers[1], view, ctypes.byref(alloc_struct), ctypes.byref(out)
            )
        )
        return Value(_raw=out, alloc=alloc_struct)
    finally:
        reads.close()
        for built in owned:
            built.close()


def differences(
    schema: Any,
    value: Any,
    baseline: Any = None,
    placeholder: str = "***",
    *,
    alloc: "_abi.Alloc | None" = None,
) -> Value:
    """Where `value` differs from `baseline`, or from the schema's defaults
    when `baseline` is `None`.

    A list of maps `{"path": [segments], "key": "tls.ca", "value": ..,
    "default": ..}`; `value` and `default` are left out where that side
    has nothing. An object's declared fields, an open map's keys and the
    fields of a variant that kept its arm are places of their own;
    anything else (a number, text, a list, a tuple such as a duration, a
    variant that changed arm) is one place, compared whole. A field a side
    leaves out reads as its schema default. A secret that differs is
    listed with `placeholder` on each side, never its value. Local `$ref`s
    are followed.

    `schema`, `value` and `baseline` are `Value`s or `Ref`s, or what
    `Value(...)` takes. The result is a new `Value` the caller closes.
    Raises `WrongKind` for a schema that is not a map."""
    lib = _lib.core()
    alloc_struct = alloc if alloc is not None else lib.alloc
    owned = []
    reads = contextlib.ExitStack()
    try:
        pointers: list[Any] = []
        for item in (schema, value, baseline):
            if item is None:
                pointers.append(None)
            elif isinstance(item, Ref):
                pointers.append(reads.enter_context(_reading(item)))
            else:
                built = Value(item)
                owned.append(built)
                pointers.append(ctypes.byref(built._raw))
        view, _buf = _make_str(placeholder.encode("utf-8"))
        out = _abi.Value()
        check(
            lib.guatiao_schema_differences(
                pointers[0],
                pointers[1],
                pointers[2],
                view,
                ctypes.byref(alloc_struct),
                ctypes.byref(out),
            )
        )
        return Value(_raw=out, alloc=alloc_struct)
    finally:
        reads.close()
        for built in owned:
            built.close()
