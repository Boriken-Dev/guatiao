# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Naming a place inside a value, whether a form fits a schema, its
layout, field visibility and the flat `key -> text` projection, over the
`guatiao_intake` library.

Importing this package touches no native library: the library is found
and loaded on the first call that needs it.
"""

from __future__ import annotations

import ctypes
import os
import threading
from collections.abc import Mapping
from pathlib import Path
from typing import Any, Optional, cast

from guatiao import Ref, Value, ffi

__version__ = "0.0.0a1"

__all__ = [
    "__version__",
    "PROVIDER_FORM_KEY",
    "at",
    "check",
    "flat_keys",
    "flatten",
    "for_schema",
    "form_for",
    "is_visible",
    "layout",
    "library",
    "resolve",
    "unflatten",
    "use_library",
]

#: The key of a provider's `meta` holding its form document: read it with
#: `guatiao.Registry.provider_meta`, then `check` it against
#: `provider_config`.
PROVIDER_FORM_KEY = "form"

_P = ctypes.POINTER
_u32 = ctypes.c_uint32
_V = _P(ffi.Node)
_A = _P(ffi.Alloc)

# name -> (argtypes, restype, the feature the library needs for it).
_EXPORTS: "dict[str, tuple[list[Any], Any, str | None]]" = {
    "guatiao_alloc_default": ([_A], _u32, None),
    "guatiao_intake_check": ([_V, _V, _A, _V], _u32, None),
    "guatiao_intake_layout": ([_V, _V, _A, _V], _u32, None),
    "guatiao_intake_is_visible": ([_V, _V, ffi.Str, _V, _P(ctypes.c_bool)], _u32, None),
    "guatiao_intake_for_schema": ([_V, _A, _V], _u32, None),
    "guatiao_intake_form_for": ([_V, _V, ffi.Str, _A, _V], _u32, None),
    "guatiao_intake_path_get": ([_V, ffi.Str], _V, None),
    "guatiao_intake_resolve": ([_V, ffi.Str], _V, None),
    "guatiao_intake_flat_keys": ([_V, ffi.Str, _A, _V], _u32, None),
    "guatiao_intake_flatten": ([_V, ffi.Str, _V, _A, _V], _u32, None),
    "guatiao_intake_unflatten": ([_V, ffi.Str, _V, _A, _V], _u32, None),
}

_lock = threading.Lock()
_library: "ffi.Library | None" = None
_chosen: "Path | None" = None


def use_library(path: "str | os.PathLike[str]") -> None:
    """Loads `guatiao_intake` from `path` -- a file exporting its
    functions, whatever it is called (an application's own library
    carrying several surfaces, say), or a directory holding it under its
    platform filename -- instead of looking for one. `RuntimeError` when
    it is already loaded from elsewhere; naming the one already loaded
    does nothing."""
    global _chosen
    chosen = Path(path)
    with _lock:
        if _library is not None:
            same = chosen if chosen.is_file() else chosen / ffi.dll_filename("guatiao_intake")
            if same.resolve() != _library.path.resolve():
                raise RuntimeError(f"guatiao_intake is already loaded from {_library.path}")
            return
        _chosen = chosen


def library() -> "ffi.Library":
    """The `guatiao_intake` library, found and loaded on first use:
    `use_library`'s choice, else `GUATIAO_INTAKE_LIBRARY`, else
    `GUATIAO_LIBRARY` (a directory holding it, or a file exporting its
    functions or sitting beside it), else the platform's search.
    `guatiao.LibraryNotFound` names every place tried."""
    global _library
    with _lock:
        if _library is None:
            path = ffi.find_library(
                "guatiao_intake",
                variable="GUATIAO_INTAKE_LIBRARY",
                symbol="guatiao_intake_check",
                chosen=_chosen,
            )
            _library = ffi.Library(path, _EXPORTS)
        return _library


def _check_status(status: int) -> None:
    ffi.check(status)


def _ptr(ref: Ref) -> Any:
    return ctypes.byref(ffi.node_of(ref))

def _alloc(given: "ffi.Alloc | None", lib: Any) -> "ffi.Alloc":
    return given if given is not None else lib.alloc


def check(schema: Ref, form: Ref, *, alloc: "ffi.Alloc | None" = None) -> Any:
    """`None` when `form` fits `schema`; otherwise the error map
    (`kind`, `at`, and whichever of `path`/`id`/`field`/`expected`/
    `message` apply) `guatiao_intake_check` writes."""
    lib = library()
    alloc_struct = _alloc(alloc, lib)
    out = ffi.Node()
    status = lib.guatiao_intake_check(
        _ptr(schema),
        _ptr(form),
        ctypes.byref(alloc_struct),
        ctypes.byref(out),
    )
    if status == ffi.Status.OK:
        return None
    if status != ffi.Status.ERR_BAD_VALUE:
        _check_status(status)
    value = ffi.adopt(out, alloc_struct)
    try:
        return value.to_python()
    finally:
        value.close()


def layout(schema: Ref, form: Ref, *, alloc: "ffi.Alloc | None" = None) -> Any:
    """The schema's fields grouped into sections and put in order, as
    `[{"section": <map or None>, "fields": [key, ...]}, ...]`."""
    lib = library()
    alloc_struct = _alloc(alloc, lib)
    out = ffi.Node()
    _check_status(
        lib.guatiao_intake_layout(
            _ptr(schema),
            _ptr(form),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    value = ffi.adopt(out, alloc_struct)
    try:
        return value.to_python()
    finally:
        value.close()


def is_visible(schema: Ref, form: Ref, key: str, values: Ref) -> bool:
    """Whether the field under `key` is shown, given `values` entered
    so far."""
    lib = library()
    key_view, _buf = ffi.text(key)
    out = ctypes.c_bool(False)
    _check_status(
        lib.guatiao_intake_is_visible(
            _ptr(schema),
            _ptr(form),
            key_view,
            _ptr(values),
            ctypes.byref(out),
        )
    )
    return bool(out.value)


def at(value: Ref, path: str) -> Optional[Ref]:
    """The value at `path`, or `None`.

    `agent[1].name[home].host`: a dot is a member, a bracket is a list
    position or a map key, and which one a bracket means is decided by
    what it is applied to -- so `env[PATH]` needs no quoting and
    `env["1"]` is a key rather than a position.

    What comes back **borrows** from `value` and is read-only: it is a
    view into the tree you passed, not a copy. It keeps `value` alive and
    is found again on every access, so reading it after `value` is closed
    raises `ValueError`, and a change to `value` is seen as it is now.
    A path that names nothing and a path that is not a path at all both
    answer `None`.
    """
    lib = library()
    view, _buf = ffi.text(path)

    def find() -> Any:
        ptr = lib.guatiao_intake_path_get(_ptr(value), view)
        return ptr.contents if ptr else None

    return ffi.borrow(value, find)


def for_schema(schema: Ref, *, alloc: "ffi.Alloc | None" = None) -> Value:
    """The form `schema` implies, for when nobody wrote one.

    One section per distinct `x-section` the fields name, in
    first-appearance order and **ids only** -- what a section is called
    is a form's business and a schema has no opinion -- plus each
    member's own form where there is something in it. A schema that
    groups nothing gives `{}`, which is a complete form.
    """
    lib = library()
    alloc_struct = _alloc(alloc, lib)
    out = ffi.Node()
    _check_status(
        lib.guatiao_intake_for_schema(
            _ptr(schema),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    return ffi.adopt(out, alloc_struct)


def form_for(
    form: Ref,
    schema: Ref,
    path: str,
    *,
    alloc: "ffi.Alloc | None" = None,
) -> "Value | None":
    """The form to show the field at `path` with: the one `form` assigns,
    or the one its schema implies.

    `None` when the path names no field, or names one with no members and
    no form assigned -- a text is shown by a control, not by a form.
    """
    lib = library()
    alloc_struct = _alloc(alloc, lib)
    view, _buf = ffi.text(path)
    out = ffi.Node()
    _check_status(
        lib.guatiao_intake_form_for(
            _ptr(form),
            _ptr(schema),
            view,
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    value = ffi.adopt(out, alloc_struct)
    if value.is_absent():
        value.close()
        return None
    return value


def resolve(schema: Ref, key: str) -> "Ref | None":
    """The schema field governing the flat `key`, or `None`.

    Follows one level of projection, so `auth.password` answers the field
    of that name inside the arm rather than `auth`. Everything a front end
    shows about a field -- `title`, `required`, `x-widget`, a secret mark --
    is read off the node returned. It **borrows** from `schema`, is
    read-only, keeps `schema` alive and is found again on every access:
    reading it after `schema` is closed raises `ValueError`.
    """
    lib = library()
    view, _buf = ffi.text(key)

    def find() -> Any:
        ptr = lib.guatiao_intake_resolve(_ptr(schema), view)
        return ptr.contents if ptr else None

    return ffi.borrow(schema, find)


def _flat_call(
    function: Any, schema: Ref, key: str, middle: "tuple[Any, ...]", alloc: "ffi.Alloc | None"
) -> "tuple[int, Value]":
    lib = library()
    alloc_struct = _alloc(alloc, lib)
    view, _buf = ffi.text(key)
    out = ffi.Node()
    status = function(
        _ptr(schema),
        view,
        *middle,
        ctypes.byref(alloc_struct),
        ctypes.byref(out),
    )
    return status, ffi.adopt(out, alloc_struct)


def flat_keys(
    schema: Ref, key: str, *, alloc: "ffi.Alloc | None" = None
) -> "list[str]":
    """The flat keys the field `key` projects onto: its own, then one per
    field of every arm. A member that is itself a container stores its
    leaves below its key and those are not listed. `WrongKind` when `key`
    names no field."""
    status, out = _flat_call(library().guatiao_intake_flat_keys, schema, key, (), alloc)
    try:
        _check_status(status)
        return cast("list[str]", out.to_python())
    finally:
        out.close()


def flatten(
    schema: Ref, key: str, value: Ref, *, alloc: "ffi.Alloc | None" = None
) -> "dict[str, str]":
    """`value` as the flat store of `key -> text` the field `key` keeps:
    one entry per scalar leaf (a tagged field's discriminant under its own
    key and its payload under `key.member`, a list indexed as
    `agent[0].name`, an open map bracketed as `env[PATH]`). An empty
    container stores nothing. `WrongKind` when `key` names no field or the
    value's shape contradicts it."""
    lib = library()
    status, out = _flat_call(
        lib.guatiao_intake_flatten, schema, key, (_ptr(value),), alloc
    )
    try:
        _check_status(status)
        return cast("dict[str, str]", out.to_python())
    finally:
        out.close()


def _is_text_store(node: "ffi.Node") -> bool:
    if node.tag != ffi.Tag.MAP:
        return False
    entries = node.payload.map
    return all(entries.ptr[i].value.tag == ffi.Tag.STRING for i in range(entries.len))


def unflatten(
    schema: Ref,
    key: str,
    flat: "Mapping[str, str] | Ref",
    *,
    alloc: "ffi.Alloc | None" = None,
) -> "Value | None":
    """The value the flat store `flat` holds for the field `key`, the
    reverse of `flatten`; the caller closes it. `None` when `key` names no
    field, nothing is stored under it, or the stored shape cannot be its
    kind (a tagged field whose discriminant is missing or names no arm).
    An entry the schema does not declare is dropped.

    `flat` maps `str` to `str`, or is a `Value` or `Ref` onto such a map.
    A store that is not, in either form, raises `TypeError`: `None` is the
    answer for "nothing stored", never for a malformed store."""
    lib = library()
    owned = None
    if isinstance(flat, Ref):
        if not _is_text_store(ffi.node_of(flat)):
            raise TypeError("a flat store maps str to str")
        store = flat
    else:
        if not isinstance(flat, Mapping) or not all(
            isinstance(k, str) and isinstance(v, str) for k, v in flat.items()
        ):
            raise TypeError("a flat store maps str to str")
        store = owned = Value.from_python(dict(flat), alloc=alloc)
    try:
        status, out = _flat_call(
            lib.guatiao_intake_unflatten, schema, key, (_ptr(store),), alloc
        )
        if status == ffi.Status.ERR_WRONG_KIND:
            out.close()
            return None
        try:
            _check_status(status)
        except BaseException:
            out.close()
            raise
        return out
    finally:
        if owned is not None:
            owned.close()
