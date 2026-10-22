# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Kind tables built over Python objects, and the `ctx` each call names.

A table is built once per kind and class and kept for the life of the
process: C may hold its address for as long as anything it served is
alive, and a table is a few pointers. A `ctx` is the address of a byte
this module owns, one per live object, so it is a real, aligned address
that names nothing once the object is dropped; a call naming it then
answers `GUATIAO_ERR_GONE` rather than reaching a freed object.
"""

from __future__ import annotations

import ctypes
import logging
import threading
from typing import Any

from .. import _abi
from ..errors import GuatiaoError
from ..value import Value
from ._shapes import Scope, _move_out

_log = logging.getLogger("guatiao.kinds")

_lock = threading.Lock()
_live: "dict[int, tuple[Any, Any]]" = {}
_tables: "dict[tuple[int, type], tuple[Any, list[Any], Any]]" = {}


class _Gone(Exception):
    pass


def hold(obj: Any) -> int:
    """A `ctx` naming `obj` until `release(ctx)`."""
    byte = ctypes.create_string_buffer(1)
    ctx = ctypes.addressof(byte)
    with _lock:
        _live[ctx] = (obj, byte)
    return ctx


def release(ctx: Any) -> Any:
    """Lets go of what `ctx` names; harmless for one already released."""
    with _lock:
        held = _live.pop(int(ctx or 0), None)
    return held[0] if held else None


def held(ctx: Any) -> Any:
    if not ctx:
        raise _Gone(int(_abi.Status.ERR_NULL))
    with _lock:
        found = _live.get(int(ctx))
    if found is None:
        raise _Gone(int(_abi.Status.ERR_GONE))
    return found[0]


def live() -> int:
    """How many objects a C caller can still reach through a `ctx`."""
    with _lock:
        return len(_live)


def status_of(exc: BaseException) -> "tuple[int, str]":
    """The status and words a refusal crosses with."""
    if isinstance(exc, GuatiaoError):
        return exc.status, exc.message or ""
    if isinstance(exc, NotImplementedError):
        return int(_abi.Status.ERR_UNSUPPORTED), str(exc)
    return int(_abi.Status.ERR_INTERNAL), f"{type(exc).__name__}: {exc}"


def write_err(err: Any, status: int, message: str) -> None:
    """Fills a `guatiao_provider_error *`, its message owned text the
    caller frees through the allocator it records."""
    if not err:
        return
    target = err.contents
    target.status = status
    try:
        text = _move_out(Value.string(message)).payload.text if message else _abi.String()
    except BaseException:
        text = _abi.String()
    target.message = text


def _refuse(exc: BaseException, err: Any, where: str) -> int:
    if isinstance(exc, _Gone):
        status, message = exc.args[0], "the object this call names is gone"
    else:
        status, message = status_of(exc)
    if status == int(_abi.Status.OK):
        status = int(_abi.Status.ERR_INTERNAL)
    if err is None:
        _log.error("%s cannot fail and raised", where, exc_info=exc)
    else:
        write_err(err, status, message)
    return int(status)


def _shim(kind: Any, method: Any) -> Any:
    proto = kind.slot_type(method.name)
    shapes = method.args
    n = len(shapes)
    answer = method.returns
    where = f"`{kind.name}.{method.name}`"

    def shim(ctx: Any, *rest: Any) -> int:
        scope = Scope()
        err = rest[-1] if method.fallible else None
        status = int(_abi.Status.OK)
        try:
            values: "list[Any]" = [None] * n
            # What was handed over is taken first, so a refusal below
            # still owns it, and frees it, rather than leaking it.
            for i, shape in enumerate(shapes):
                if shape.takes:
                    values[i] = shape.receive(rest[i], scope)
            target = held(ctx)
            for i, shape in enumerate(shapes):
                if not shape.takes:
                    values[i] = shape.receive(rest[i], scope)
            result = getattr(target, method.name)(*values)
            if answer is not None:
                answer.write(rest[n], result)
        except BaseException as exc:  # nothing may cross back into C
            status = _refuse(exc, err, where)
        try:
            scope.close()
        except BaseException as exc:
            if status == int(_abi.Status.OK):
                status = _refuse(exc, err, where)
        return status

    return proto(shim)


def _destroy_shim(kind: Any) -> Any:
    proto = kind.slot_type("destroy")

    def destroy(ctx: Any) -> None:
        try:
            release(ctx)
        except BaseException:  # nothing may cross back into C
            _log.exception("`%s.destroy` raised", kind.name)

    return proto(destroy)


def table_for(kind: Any, cls: type) -> Any:
    """The table serving `kind` through objects of `cls`, built on first
    use: a slot for each method `cls` defines, null for the rest, so a
    Rust caller runs the kind's own default for an appended one."""
    key = (id(kind), cls)
    with _lock:
        found = _tables.get(key)
    if found is not None:
        return found[0]
    missing = [m.name for m in kind.methods if m.required and not kind.implemented(cls, m.name)]
    if missing:
        raise TypeError(
            f"{cls.__name__} does not implement {', '.join(missing)}, which the {kind.name} kind requires"
        )
    table = kind.struct()
    table.header = _abi.KindHeader(
        struct_size=ctypes.sizeof(kind.struct), floor_hash=kind.floor_hash
    )
    keep = []
    if kind.is_object:
        slot = _destroy_shim(kind)
        keep.append(slot)
        table.destroy = slot
    for method in kind.methods:
        if kind.implemented(cls, method.name):
            slot = _shim(kind, method)
            keep.append(slot)
            setattr(table, method.name, slot)
    with _lock:
        # Another thread may have built one meanwhile; the first stays.
        found = _tables.setdefault(key, (table, keep, kind))
    return found[0]


def serve(kind: Any, obj: Any, scope: "Scope | None" = None) -> "_abi.Object":
    """`obj` as a `guatiao_object` of `kind`: held until its holder calls
    `destroy`, or, lent, until `scope` closes."""
    table = table_for(kind, type(obj))
    ctx = hold(obj)
    if scope is not None:
        scope.after(lambda: release(ctx))
    return _abi.Object(
        table=ctypes.addressof(table), size=ctypes.sizeof(kind.struct), ctx=ctx
    )
