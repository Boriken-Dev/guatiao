# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Taking a provider's kind table as the `ctypes.Structure` it declares."""

from __future__ import annotations

import ctypes

from . import _abi


class FloorMismatch(RuntimeError):
    """The table is too small for this binding's struct, or its
    `floor_hash` does not match what this binding was built against."""


def table(table_ptr, size: int, struct_type: type, *, floor_hash: int):
    """Casts `table_ptr` (from `Registry.provider_table`) to
    `POINTER(struct_type).contents`.

    `struct_type` is a `ctypes.Structure` whose first field is a
    `guatiao_kind_header` -- the shape a kind's own C header declares
    (`greeter_vtable` and friends). Checked before the cast: `size`
    against `sizeof(struct_type)` (a shorter table is an older library,
    refused here rather than read past its end), and the header's
    `floor_hash` against the one the kind's header names
    (`<table>_FLOOR_HASH`).
    """
    if not table_ptr:
        raise FloorMismatch("no such table")
    floor = ctypes.sizeof(struct_type)
    if size < floor:
        raise FloorMismatch(f"table is {size} bytes, this binding needs {floor}")
    header = ctypes.cast(table_ptr, ctypes.POINTER(_abi.KindHeader)).contents
    if header.floor_hash != floor_hash:
        raise FloorMismatch(
            f"floor_hash {header.floor_hash} does not match {floor_hash}"
        )
    return ctypes.cast(table_ptr, ctypes.POINTER(struct_type)).contents
