# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Naming a place inside a value, whether a form fits a schema, its
layout, and field visibility, over `guatiao_intake`. Resolved lazily,
like `serde.py`."""

from __future__ import annotations

import ctypes

from . import _abi, _lib
from .errors import check as _check_status
from .value import Ref, Value, _make_str
from .registry import _Borrowed


def _alloc(given: "_abi.Alloc | None", lib) -> "_abi.Alloc":
    return given if given is not None else lib.alloc


def check(schema: Value, form: Value, *, alloc: "_abi.Alloc | None" = None):
    """`None` when `form` fits `schema`; otherwise the error map
    (`kind`, `at`, and whichever of `path`/`id`/`field`/`expected`/
    `message` apply) `guatiao_intake_check` writes."""
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    status = lib.guatiao_intake_check(
        ctypes.byref(schema._raw),
        ctypes.byref(form._raw),
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


def layout(schema: Value, form: Value, *, alloc: "_abi.Alloc | None" = None):
    """The schema's fields grouped into sections and put in order, as
    `[{"section": <map or None>, "fields": [key, ...]}, ...]`."""
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    _check_status(
        lib.guatiao_intake_layout(
            ctypes.byref(schema._raw),
            ctypes.byref(form._raw),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    value = Value(_raw=out, alloc=alloc_struct)
    try:
        return value.to_python()
    finally:
        value.close()


def is_visible(schema: Value, form: Value, key: str, values: Value) -> bool:
    """Whether the field under `key` is shown, given `values` entered
    so far."""
    lib = _lib.form()
    key_view, _buf = _make_str(key.encode("utf-8"))
    out = ctypes.c_bool(False)
    _check_status(
        lib.guatiao_intake_is_visible(
            ctypes.byref(schema._raw),
            ctypes.byref(form._raw),
            key_view,
            ctypes.byref(values._raw),
            ctypes.byref(out),
        )
    )
    return bool(out.value)


def at(value: Value, path: str):
    """The value at `path`, or `None`.

    `agent[1].name[home].host`: a dot is a member, a bracket is a list
    position or a map key, and which one a bracket means is decided by
    what it is applied to -- so `env[PATH]` needs no quoting and
    `env["1"]` is a key rather than a position.

    What comes back **borrows** from `value` and is read-only: it is a
    view into the tree you passed, not a copy, so it must not outlive it.
    A path that names nothing and a path that is not a path at all both
    answer `None`.
    """
    lib = _lib.form()
    view, _buf = _make_str(path.encode("utf-8"))
    ptr = lib.guatiao_intake_path_get(ctypes.byref(value._raw), view)
    if not ptr:
        return None
    return Ref(_Borrowed(ptr.contents), ())


def for_schema(schema: Value, *, alloc: "_abi.Alloc | None" = None) -> Value:
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
            ctypes.byref(schema._raw),
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    return Value(_raw=out, alloc=alloc_struct)


def form_for(
    form: Value,
    schema: Value,
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
            ctypes.byref(form._raw),
            ctypes.byref(schema._raw),
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
