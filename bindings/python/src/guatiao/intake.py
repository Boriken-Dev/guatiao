# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Naming a place inside a value, whether a form fits a schema, its
layout, field visibility and the flat `key -> text` projection, over
`guatiao_intake`. Resolved lazily, like `serde.py`."""

from __future__ import annotations

import ctypes
from collections.abc import Mapping
from typing import Any, Optional, cast

from . import _abi, _lib
from .errors import check as _check_status
from .value import Ref, Value, _make_str
from .registry import _Borrowed


def _alloc(given: "_abi.Alloc | None", lib: Any) -> "_abi.Alloc":
    return given if given is not None else lib.alloc


def check(schema: Ref, form: Ref, *, alloc: "_abi.Alloc | None" = None) -> Any:
    """`None` when `form` fits `schema`; otherwise the error map
    (`kind`, `at`, and whichever of `path`/`id`/`field`/`expected`/
    `message` apply) `guatiao_intake_check` writes."""
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    status = lib.guatiao_intake_check(
        schema._ptr(),
        form._ptr(),
        ctypes.byref(alloc_struct),
        ctypes.byref(out),
    )
    if status == _abi.Status.OK:
        return None
    if status != _abi.Status.ERR_BAD_VALUE:
        _check_status(status)
    value = Value(_raw=out, alloc=alloc_struct)
    try:
        return value.to_python()
    finally:
        value.close()


def layout(schema: Ref, form: Ref, *, alloc: "_abi.Alloc | None" = None) -> Any:
    """The schema's fields grouped into sections and put in order, as
    `[{"section": <map or None>, "fields": [key, ...]}, ...]`."""
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    _check_status(
        lib.guatiao_intake_layout(
            schema._ptr(),
            form._ptr(),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    value = Value(_raw=out, alloc=alloc_struct)
    try:
        return value.to_python()
    finally:
        value.close()


def is_visible(schema: Ref, form: Ref, key: str, values: Ref) -> bool:
    """Whether the field under `key` is shown, given `values` entered
    so far."""
    lib = _lib.form()
    key_view, _buf = _make_str(key.encode("utf-8"))
    out = ctypes.c_bool(False)
    _check_status(
        lib.guatiao_intake_is_visible(
            schema._ptr(),
            form._ptr(),
            key_view,
            values._ptr(),
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
    lib = _lib.form()
    view, _buf = _make_str(path.encode("utf-8"))

    def find() -> Any:
        ptr = lib.guatiao_intake_path_get(value._ptr(), view)
        return ptr.contents if ptr else None

    if find() is None:
        return None
    return Ref(_Borrowed(keep=value.owner, find=find), ())


def for_schema(schema: Ref, *, alloc: "_abi.Alloc | None" = None) -> Value:
    """The form `schema` implies, for when nobody wrote one.

    One section per distinct `x-section` the fields name, in
    first-appearance order and **ids only** -- what a section is called
    is a form's business and a schema has no opinion -- plus each
    member's own form where there is something in it. A schema that
    groups nothing gives `{}`, which is a complete form.
    """
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    _check_status(
        lib.guatiao_intake_for_schema(
            schema._ptr(),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    return Value(_raw=out, alloc=alloc_struct)


def form_for(
    form: Ref,
    schema: Ref,
    path: str,
    *,
    alloc: "_abi.Alloc | None" = None,
) -> "Value | None":
    """The form to show the field at `path` with: the one `form` assigns,
    or the one its schema implies.

    `None` when the path names no field, or names one with no members and
    no form assigned -- a text is shown by a control, not by a form.
    """
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    view, _buf = _make_str(path.encode("utf-8"))
    out = _abi.Value()
    _check_status(
        lib.guatiao_intake_form_for(
            form._ptr(),
            schema._ptr(),
            view,
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    value = Value(_raw=out, alloc=alloc_struct)
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
    lib = _lib.form()
    view, _buf = _make_str(key.encode("utf-8"))

    def find() -> Any:
        ptr = lib.guatiao_intake_resolve(schema._ptr(), view)
        return ptr.contents if ptr else None

    if find() is None:
        return None
    return Ref(_Borrowed(keep=schema.owner, find=find), ())


def _flat_call(
    function: Any, schema: Ref, key: str, middle: "tuple[Any, ...]", alloc: "_abi.Alloc | None"
) -> "tuple[int, Value]":
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    view, _buf = _make_str(key.encode("utf-8"))
    out = _abi.Value()
    status = function(
        schema._ptr(),
        view,
        *middle,
        ctypes.byref(alloc_struct),
        ctypes.byref(out),
    )
    return status, Value(_raw=out, alloc=alloc_struct)


def flat_keys(
    schema: Ref, key: str, *, alloc: "_abi.Alloc | None" = None
) -> "list[str]":
    """The flat keys the field `key` projects onto: its own, then one per
    field of every arm. A member that is itself a container stores its
    leaves below its key and those are not listed. `WrongKind` when `key`
    names no field."""
    status, out = _flat_call(_lib.form().guatiao_intake_flat_keys, schema, key, (), alloc)
    try:
        _check_status(status)
        return cast("list[str]", out.to_python())
    finally:
        out.close()


def flatten(
    schema: Ref, key: str, value: Ref, *, alloc: "_abi.Alloc | None" = None
) -> "dict[str, str]":
    """`value` as the flat store of `key -> text` the field `key` keeps:
    one entry per scalar leaf (a tagged field's discriminant under its own
    key and its payload under `key.member`, a list indexed as
    `agent[0].name`, an open map bracketed as `env[PATH]`). An empty
    container stores nothing. `WrongKind` when `key` names no field or the
    value's shape contradicts it."""
    lib = _lib.form()
    status, out = _flat_call(
        lib.guatiao_intake_flatten, schema, key, (value._ptr(),), alloc
    )
    try:
        _check_status(status)
        return cast("dict[str, str]", out.to_python())
    finally:
        out.close()


def _is_text_store(node: "_abi.Value") -> bool:
    if node.tag != _abi.Tag.MAP:
        return False
    entries = node.payload.map
    return all(entries.ptr[i].value.tag == _abi.Tag.STRING for i in range(entries.len))


def unflatten(
    schema: Ref,
    key: str,
    flat: "Mapping[str, str] | Ref",
    *,
    alloc: "_abi.Alloc | None" = None,
) -> "Value | None":
    """The value the flat store `flat` holds for the field `key`, the
    reverse of `flatten`; the caller closes it. `None` when `key` names no
    field, nothing is stored under it, or the stored shape cannot be its
    kind (a tagged field whose discriminant is missing or names no arm).
    An entry the schema does not declare is dropped.

    `flat` maps `str` to `str`, or is a `Value` or `Ref` onto such a map.
    A store that is not, in either form, raises `TypeError`: `None` is the
    answer for "nothing stored", never for a malformed store."""
    lib = _lib.form()
    owned = None
    if isinstance(flat, Ref):
        if not _is_text_store(flat._require_node()):
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
            lib.guatiao_intake_unflatten, schema, key, (store._ptr(),), alloc
        )
        if status == _abi.Status.ERR_WRONG_KIND:
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
