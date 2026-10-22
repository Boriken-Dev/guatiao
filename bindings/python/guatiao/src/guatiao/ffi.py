# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What a package over another guatiao C library builds on.

`guatiao-serde` and `guatiao-intake` call their own libraries and pass
trees to and from this package through what is here, as may any package
binding a C library that takes and answers guatiao values. A tree crosses
as the address of its root node: whichever library built it, any of them
frees it, since a value records its own allocator. The library that built
it must stay loaded while the tree lives.

Supported API, versioned with the package.
"""

from __future__ import annotations

import ctypes
from typing import Any, Callable, Optional

from . import _abi, _lib
from ._abi import Alloc, Bytes, Status, Str, Tag
from ._lib import Library, LibraryNotFound, MissingSymbol, dll_filename, find_library
from .errors import raise_for_status
from .value import Ref, Value, _make_str, _string_view

#: The C struct `guatiao_value`: one node of a tree, not a handle that owns
#: one (that is `guatiao.Value`).
Node = _abi.Value

__all__ = [
    "Alloc",
    "Bytes",
    "Library",
    "LibraryNotFound",
    "MissingSymbol",
    "Node",
    "Status",
    "Str",
    "Tag",
    "adopt",
    "borrow",
    "check",
    "dll_filename",
    "find_library",
    "free",
    "node_of",
    "text",
    "text_of",
]


def check(status: int, message: str = "") -> None:
    """Raises the `guatiao.GuatiaoError` subclass for `status`, carrying
    `message`, unless it is `Status.OK`."""
    raise_for_status(status, message)


def text(data: "str | bytes") -> "tuple[Str, Any]":
    """A `guatiao_str` over `data` (UTF-8 for a `str`), and the buffer to
    keep referenced until the call that reads it returns."""
    return _make_str(data.encode("utf-8") if isinstance(data, str) else data)


def text_of(node: Node) -> str:
    """The text of a string node (invalid UTF-8 replaced); `""` for a node
    of any other kind."""
    if node.tag != Tag.STRING:
        return ""
    return _string_view(node.payload.text).decode("utf-8", "replace")


def free(node: Node) -> None:
    """Frees what `node` owns, through the `guatiao` library, and leaves it
    absent. For a node a call wrote that no `Value` adopted."""
    _lib.core().guatiao_value_free(ctypes.byref(node))


def adopt(node: Node, alloc: Alloc) -> Value:
    """A `Value` taking over the root `node` a call wrote in `alloc`; the
    `Value` frees it. `alloc` is kept with it, to grow the tree with."""
    return Value(_raw=node, alloc=alloc)


def node_of(ref: Ref) -> Node:
    """The node `ref` names now, to pass to a call by address
    (`ctypes.byref`) for as long as nothing changes its tree.
    `ValueError` once its value is closed, `LookupError` when its path
    names nothing."""
    return ref._require_node()


def borrow(keep: Ref, find: Callable[[], Optional[Node]]) -> "Ref | None":
    """A read-only `Ref` onto the node `find` answers, asked again on every
    access, which keeps `keep`'s value alive and refuses to read once it
    is closed. `None` when `find` answers nothing now."""
    from .registry import _Borrowed

    if find() is None:
        return None
    return Ref(_Borrowed(keep=keep.owner, find=find), ())
