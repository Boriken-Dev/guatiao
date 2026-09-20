# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Whether a form fits a schema, its layout, and field visibility, over
`guatiao_form`. Resolved lazily, like `serde.py`."""

from __future__ import annotations

import ctypes

from . import _abi, _lib
from .errors import check as _check_status
from .value import Value, _make_str


def _alloc(given: "_abi.Alloc | None", lib) -> "_abi.Alloc":
    return given if given is not None else lib.alloc


def check(schema: Value, form: Value, *, alloc: "_abi.Alloc | None" = None):
    """`None` when `form` fits `schema`; otherwise the error map
    (`kind`, `at`, and whichever of `path`/`id`/`field`/`expected`/
    `message` apply) `guatiao_form_check` writes."""
    lib = _lib.form()
    alloc_struct = _alloc(alloc, lib)
    out = _abi.Value()
    status = lib.guatiao_form_check(
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
        lib.guatiao_form_layout(
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
        lib.guatiao_form_is_visible(
            ctypes.byref(schema._raw),
            ctypes.byref(form._raw),
            key_view,
            ctypes.byref(values._raw),
            ctypes.byref(out),
        )
    )
    return bool(out.value)
