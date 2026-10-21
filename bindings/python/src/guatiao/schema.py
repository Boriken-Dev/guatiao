# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What a schema says about a value: here, which parts are secrets."""

from __future__ import annotations

import ctypes
from typing import Any

from . import _abi, _lib
from .errors import check
from .value import Ref, Value, _make_str

__all__ = ["redact"]


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
    try:
        pointers = []
        for item in (schema, value):
            if isinstance(item, Ref):
                pointers.append(item._ptr())
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
        for built in owned:
            built.close()
