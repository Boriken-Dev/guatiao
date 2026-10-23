# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The wire encoding read into plain Python objects and written from
them in Python, with no library: one crossing carries a whole document
as bytes, where walking a tree node by node costs a crossing per node.

Read bytes come from a library that checked them as it wrote them, so
reading here guards against nothing worse than a cut-short body."""

from __future__ import annotations

import decimal
import math
from collections.abc import Mapping
from typing import Any

from .errors import BadValue
from .value import ABSENT, Ref

_ABSENT, _NULL, _BOOL, _NUMBER, _STRING, _BYTES, _LIST, _MAP = range(8)

#: How deep containers nest: what every walk in the library follows.
MAX_DEPTH = 128

_NUMBERS = ("auto", "decimal", "str")


def _leb_rest(data: bytes, at: int, n: int) -> "tuple[int, int]":
    """The rest of a LEB128 count whose first byte `n` had its high bit
    set; `at` is just past that byte."""
    n &= 0x7F
    shift = 7
    while True:
        b = data[at]
        at += 1
        n |= (b & 0x7F) << shift
        if b < 0x80:
            return n, at
        shift += 7


def to_python(data: bytes, numbers: str = "auto") -> Any:
    """The plain Python value wire bytes hold, as `Value.to_python`
    answers it for the same tree."""
    if numbers not in _NUMBERS:
        raise ValueError(f"unknown numbers mode: {numbers!r}")
    auto = numbers == "auto"
    as_decimal = numbers == "decimal"
    end = len(data)
    at = 0
    # Each open container: [it, elements left, is a map, the key it goes under].
    stack: "list[list[Any]]" = []
    key: Any = None
    try:
        while True:
            if stack and stack[-1][2]:
                n = data[at]
                at += 1
                if n >= 0x80:
                    n, at = _leb_rest(data, at, n)
                if at + n > end:
                    raise IndexError
                key = data[at : at + n].decode("utf-8")
                at += n
            tag = data[at]
            at += 1
            if _NUMBER <= tag <= _BYTES:
                n = data[at]
                at += 1
                if n >= 0x80:
                    n, at = _leb_rest(data, at, n)
                if at + n > end:
                    raise IndexError
                body = data[at : at + n]
                at += n
                if tag == _STRING:
                    value: Any = body.decode("utf-8")
                elif tag == _NUMBER:
                    if auto:
                        if b"." in body or b"e" in body or b"E" in body:
                            value = float(body)
                        else:
                            value = int(body)
                    elif as_decimal:
                        value = decimal.Decimal(body.decode("ascii"))
                    else:
                        value = body.decode("ascii")
                else:
                    value = body
            elif tag == _LIST or tag == _MAP:
                n = data[at]
                at += 1
                if n >= 0x80:
                    n, at = _leb_rest(data, at, n)
                if n:
                    if len(stack) >= MAX_DEPTH:
                        raise BadValue(message=f"nested deeper than {MAX_DEPTH}")
                    stack.append([[] if tag == _LIST else {}, n, tag == _MAP, key])
                    continue
                value = [] if tag == _LIST else {}
            elif tag == _NULL:
                value = None
            elif tag == _BOOL:
                value = data[at] == 1
                at += 1
            elif tag == _ABSENT:
                value = ABSENT
            else:
                raise BadValue(message=f"unknown tag {tag} at {at - 1}")
            # Place the value, closing every container it completes.
            while stack:
                top = stack[-1]
                if top[2]:
                    top[0][key] = value
                else:
                    top[0].append(value)
                top[1] -= 1
                if top[1]:
                    break
                stack.pop()
                value = top[0]
                key = top[3]
            else:
                if at != end:
                    raise BadValue(message=f"bytes after the value, from {at}")
                return value
    except IndexError:
        raise BadValue(message="the bytes end inside a node") from None
    except UnicodeDecodeError:
        raise BadValue(message="text that is not UTF-8") from None


def _leb(out: bytearray, n: int) -> None:
    while n >= 0x80:
        out.append((n & 0x7F) | 0x80)
        n >>= 7
    out.append(n)


def _write(out: bytearray, obj: Any, depth: int) -> None:
    # Most common first; bool before int, which it subclasses.
    if isinstance(obj, str):
        body = obj.encode("utf-8")
        out.append(_STRING)
    elif obj is None:
        out.append(_NULL)
        return
    elif obj is True or obj is False:
        out.append(_BOOL)
        out.append(1 if obj else 0)
        return
    elif isinstance(obj, int):
        body = str(obj).encode("ascii")
        out.append(_NUMBER)
    elif isinstance(obj, float):
        if not math.isfinite(obj):
            raise ValueError(f"{obj!r} is not a finite number")
        body = repr(obj).encode("ascii")
        out.append(_NUMBER)
    elif isinstance(obj, Mapping):
        if depth >= MAX_DEPTH:
            raise BadValue(message=f"nested deeper than {MAX_DEPTH}")
        out.append(_MAP)
        _leb(out, len(obj))
        for key, item in obj.items():
            if not isinstance(key, str):
                raise TypeError("a map key must be str")
            k = key.encode("utf-8")
            _leb(out, len(k))
            out += k
            _write(out, item, depth + 1)
        return
    elif isinstance(obj, (list, tuple)):
        if depth >= MAX_DEPTH:
            raise BadValue(message=f"nested deeper than {MAX_DEPTH}")
        out.append(_LIST)
        _leb(out, len(obj))
        for item in obj:
            _write(out, item, depth + 1)
        return
    elif isinstance(obj, (bytes, bytearray)):
        body = bytes(obj)
        out.append(_BYTES)
    elif isinstance(obj, decimal.Decimal):
        if not obj.is_finite():
            raise ValueError(f"{obj!r} is not a finite number")
        body = str(obj).encode("ascii")
        out.append(_NUMBER)
    elif obj is ABSENT:
        out.append(_ABSENT)
        return
    elif isinstance(obj, Ref):
        out += obj.to_wire()
        return
    else:
        raise TypeError(f"cannot convert {type(obj)!r} to a guatiao value")
    _leb(out, len(body))
    out += body


def from_python(obj: Any) -> bytes:
    """The wire bytes of what `Value.from_python(obj)` builds, with no
    tree built: the same objects accepted, the same errors raised."""
    out = bytearray()
    _write(out, obj, 0)
    return bytes(out)
