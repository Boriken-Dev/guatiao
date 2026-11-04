# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A guatiao value tree: `Value` owns the root, `Ref`/`Map`/`List` address
a node inside it by path and walk to it fresh on every access.

Reading needs no library at all -- the same promise `guatiao.h` makes --
because every reader here (`tag_of`, `_string_view`, `_map_find`, ...) is
a plain reimplementation of the header's `static inline` helpers over the
ctypes structs in `_abi.py`. Only building and freeing call into the
loaded `guatiao` library.
"""

from __future__ import annotations

import contextlib
import ctypes
import decimal
import functools
import math
import threading
import weakref
from collections.abc import Mapping
from typing import Any, Callable, Iterator, Protocol, TypeVar, Union, cast

from . import _abi, _lib
from .errors import check, raise_for_status

Number = Union[int, float, decimal.Decimal]

_INT64_MIN = -(2**63)
_INT64_MAX = 2**63 - 1


class _Absent:
    """The answer to a lookup that found nothing. Falsy, and distinct
    from `None`, which is the stored null."""

    __slots__ = ()

    def __repr__(self) -> str:
        return "guatiao.ABSENT"

    def __bool__(self) -> bool:
        return False


ABSENT = _Absent()


# ---- reading, with no library needed ----------------------------------


def _tag_of(node: "_abi.Value | None") -> int:
    return node.tag if node is not None else int(_abi.Tag.ABSENT)


def _string_view(s: "_abi.String") -> bytes:
    if s.len == 0:
        return b""
    return ctypes.string_at(s.ptr, s.len)


def _buffer_view(b: "_abi.Buffer") -> bytes:
    if b.len == 0:
        return b""
    return ctypes.string_at(b.ptr, b.len)


def _number_text(node: "_abi.Value | None") -> bytes:
    if node is None or node.tag != _abi.Tag.NUMBER:
        return b""
    return _string_view(node.payload.number)


def _list_at(node: "_abi.Value | None", index: int) -> "_abi.Value | None":
    if node is None or node.tag != _abi.Tag.LIST:
        return None
    lst = node.payload.list
    if index < 0:
        index += lst.len
    if not (0 <= index < lst.len):
        return None
    found: _abi.Value = lst.ptr[index]
    return found


def _map_find(node: "_abi.Value | None", key: bytes) -> "_abi.Value | None":
    if node is None or node.tag != _abi.Tag.MAP:
        return None
    m = node.payload.map
    for i in range(m.len):
        entry = m.ptr[i]
        if _string_view(entry.key) == key:
            value: _abi.Value = entry.value
            return value
    return None


def _walk(root: "_abi.Value", path: "tuple[Any, ...]") -> "_abi.Value":
    node = root
    for part in path:
        nxt = (
            _list_at(node, part)
            if isinstance(part, int)
            else _map_find(node, part.encode("utf-8"))
        )
        if nxt is None:
            raise LookupError(part)
        node = nxt
    return node


def _bool_or(node: "_abi.Value | None", fallback: bool) -> bool:
    if node is None or node.tag != _abi.Tag.BOOL:
        return fallback
    return bool(node.payload.b)


def _int_or(node: "_abi.Value | None", fallback: int) -> int:
    text = _number_text(node)
    if not text or len(text) >= 48:
        return fallback
    if any(c in text for c in b".eE"):
        return fallback
    try:
        value = int(text.decode("ascii"))
    except ValueError:
        return fallback
    return value if _INT64_MIN <= value <= _INT64_MAX else fallback


def _float_or(node: "_abi.Value | None", fallback: float) -> float:
    text = _number_text(node)
    if not text or len(text) >= 344:
        return fallback
    try:
        return float(text.decode("ascii"))
    except ValueError:
        return fallback


def _str_or(node: "_abi.Value | None", fallback: str) -> str:
    if node is None or node.tag != _abi.Tag.STRING:
        return fallback
    return _string_view(node.payload.text).decode("utf-8")


def _number_to_python(text: bytes, numbers: str) -> Any:
    if numbers == "str":
        return text.decode("ascii")
    if numbers == "decimal":
        return decimal.Decimal(text.decode("ascii"))
    if numbers == "auto":
        if any(c in text for c in b".eE"):
            return float(text.decode("ascii"))
        return int(text.decode("ascii"))
    raise ValueError(f"unknown numbers mode: {numbers!r}")


def _node_to_python(node: "_abi.Value | None", numbers: str) -> Any:
    if node is None or node.tag == _abi.Tag.ABSENT:
        return ABSENT
    if node.tag == _abi.Tag.NULL:
        return None
    if node.tag == _abi.Tag.BOOL:
        return bool(node.payload.b)
    if node.tag == _abi.Tag.NUMBER:
        return _number_to_python(_string_view(node.payload.number), numbers)
    if node.tag == _abi.Tag.STRING:
        return _string_view(node.payload.text).decode("utf-8")
    if node.tag == _abi.Tag.BYTES:
        return _buffer_view(node.payload.bytes)
    if node.tag == _abi.Tag.LIST:
        lst = node.payload.list
        return [_node_to_python(lst.ptr[i], numbers) for i in range(lst.len)]
    if node.tag == _abi.Tag.MAP:
        m = node.payload.map
        return {
            _string_view(m.ptr[i].key).decode("utf-8"): _node_to_python(
                m.ptr[i].value, numbers
            )
            for i in range(m.len)
        }
    return ABSENT  # a tag this binding does not know: skip, as the header asks


# ---- writing, through the loaded library -------------------------------


def _make_str(data: bytes) -> "tuple[_abi.Str, Any]":
    """A `Str` view over `data`, plus the buffer the caller must keep
    alive until the call using the view returns."""
    if not data:
        return _abi.Str(ptr=None, len=0), None
    buf = (ctypes.c_char * len(data)).from_buffer_copy(data)
    return _abi.Str(ptr=ctypes.cast(buf, ctypes.c_void_p), len=len(data)), buf


def _make_bytes(data: bytes) -> "tuple[_abi.Bytes, Any]":
    if not data:
        return _abi.Bytes(ptr=None, len=0), None
    buf = (ctypes.c_char * len(data)).from_buffer_copy(data)
    return _abi.Bytes(ptr=ctypes.cast(buf, ctypes.c_void_p), len=len(data)), buf


def _number_text_for(value: Number) -> str:
    if isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError(f"{value!r} is not a finite number")
        return repr(value)
    if isinstance(value, decimal.Decimal):
        if not value.is_finite():
            raise ValueError(f"{value!r} is not a finite number")
        return str(value)
    return str(value)


def _free(raw: "_abi.Value") -> None:
    """Frees whatever `raw` owns; harmless on an absent, null-tagged or
    already-moved-from value."""
    _lib.core().guatiao_value_free(ctypes.byref(raw))


def _free_all(values: "list[_abi.Value]") -> None:
    for raw in values:
        _free(raw)


def _write_python(raw: "_abi.Value", obj: Any, alloc: "_abi.Alloc") -> None:
    """Writes `obj` into `raw`, which must be absent or null-tagged.

    On any failure `raw` is freed and left empty, so a caller never holds
    a half-built tree. A node built here and then moved into a container
    is built in a scratch value that is freed afterwards: a refused move
    leaves it the caller's to free, and a completed one leaves it null.
    """
    try:
        _write_node(raw, obj, alloc)
    except BaseException:
        _free(raw)
        raise


def _write_node(raw: "_abi.Value", obj: Any, alloc: "_abi.Alloc") -> None:
    lib = _lib.core()
    if obj is None:
        check(lib.guatiao_value_null(ctypes.byref(raw)))
        return
    if obj is ABSENT:
        check(lib.guatiao_value_absent(ctypes.byref(raw)))
        return
    if isinstance(obj, Ref):
        node = obj._require_node()
        check(lib.guatiao_value_clone(ctypes.byref(alloc), ctypes.byref(node), ctypes.byref(raw)))
        return
    if isinstance(obj, bool):
        check(lib.guatiao_value_bool(bool(obj), ctypes.byref(raw)))
        return
    if isinstance(obj, (int, float, decimal.Decimal)):
        view, _buf = _make_str(_number_text_for(obj).encode("ascii"))
        check(lib.guatiao_value_number(ctypes.byref(alloc), view, ctypes.byref(raw)))
        return
    if isinstance(obj, str):
        view, _buf = _make_str(obj.encode("utf-8"))
        check(lib.guatiao_value_string(ctypes.byref(alloc), view, ctypes.byref(raw)))
        return
    if isinstance(obj, (bytes, bytearray)):
        data, _buf = _make_bytes(bytes(obj))
        check(lib.guatiao_value_bytes(ctypes.byref(alloc), data, ctypes.byref(raw)))
        return
    if isinstance(obj, (list, tuple)):
        check(lib.guatiao_value_list(ctypes.byref(alloc), ctypes.byref(raw)))
        for item in obj:
            tmp = _abi.Value()
            _write_python(tmp, item, alloc)
            try:
                check(lib.guatiao_list_push(ctypes.byref(alloc), ctypes.byref(raw), ctypes.byref(tmp)))
            finally:
                _free(tmp)
        return
    if isinstance(obj, Mapping):
        check(lib.guatiao_value_map(ctypes.byref(alloc), ctypes.byref(raw)))
        for key, item in obj.items():
            if not isinstance(key, str):
                raise TypeError("a map key must be str")
            tmp = _abi.Value()
            _write_python(tmp, item, alloc)
            try:
                key_view, _buf = _make_str(key.encode("utf-8"))
                check(
                    lib.guatiao_map_set(
                        ctypes.byref(alloc), ctypes.byref(raw), key_view, ctypes.byref(tmp)
                    )
                )
            finally:
                _free(tmp)
        return
    raise TypeError(f"cannot convert {type(obj)!r} to a guatiao value")


def _revealed(owner: "Value", ptr: int, size: int) -> Any:
    """The `size` bytes at `ptr` as a ctypes array that holds `owner`
    alive, recorded on `owner` as revealed while it lives: what every
    view and slice of a reveal refers to. Called under `owner._lock`."""
    holder_type: Any = type("_Revealed", (ctypes.c_char * size,), {})
    holder = holder_type.from_address(ptr)
    holder.owner = owner
    owner._reveals[id(holder)] = holder
    return holder


def wiping_alloc() -> "_abi.Alloc":
    """The allocator a secret is built in: every block freed through it is
    overwritten with zeros first, the block a growing string or list leaves
    behind included. `Value(secret, alloc=wiping_alloc())` builds there; a
    slot its kind marks sensitive answers there. Python's own `str` and
    `bytes` are not guatiao's to wipe: read a secret with `Ref.reveal()`
    rather than `to_python()`."""
    return _lib.core().wiping_alloc


# What a `Value` is claimed for while one operation has its tree: a reveal,
# and every other claim, is refused until it ends.
_CLOSING = "being closed"
_WIPING = "being wiped"
_HANDING = "being handed over"
_CHANGING = "being changed"
_LENT = "lent to a call"

_F = TypeVar("_F", bound=Callable[..., Any])


def _changes(method: _F) -> _F:
    """A method changing its node's tree: the owning `Value` is claimed
    for it, so a `reveal()`, a close or another change on another thread
    waits for nothing and is refused instead of reading what it frees."""

    @functools.wraps(method)
    def changing(self: "Ref", *args: Any, **kwargs: Any) -> Any:
        owner = self.owner
        if not isinstance(owner, Value):
            return method(self, *args, **kwargs)
        with owner._claimed(_CHANGING):
            return method(self, *args, **kwargs)

    return cast(_F, changing)


class _Owner(Protocol):
    """What a `Ref` walks from: a root node, and whether and how it may
    be read and changed. A `Value` is one; so is every view a call or a
    registry lends."""

    @property
    def _raw(self) -> "_abi.Value": ...

    def _check_open(self) -> None: ...

    def _check_writable(self) -> None: ...

    def _ensure_alloc(self) -> "_abi.Alloc": ...

    def _clone_alloc(self) -> "_abi.Alloc": ...


class Ref:
    """A node inside a `Value`'s tree: the owning `Value` plus a path of
    keys and indices. Every access walks the path again -- nothing here
    caches an address, because a sibling mutation can reallocate the
    array a cached one pointed into."""

    def __init__(self, owner: _Owner, path: "tuple[Any, ...]" = ()) -> None:
        self.owner = owner
        self.path = tuple(path)

    def _node(self) -> "_abi.Value | None":
        self.owner._check_open()
        try:
            return _walk(self.owner._raw, self.path)
        except LookupError:
            return None

    def _require_node(self) -> "_abi.Value":
        node = self._node()
        if node is None:
            raise LookupError(f"no value at {self.path!r}")
        return node

    def _ptr(self) -> Any:
        return ctypes.byref(self._require_node())

    def _writable(self) -> None:
        """Raises unless this node's tree may be changed through it."""
        self.owner._check_writable()

    @property
    def tag(self) -> int:
        return _tag_of(self._node())

    @property
    def is_sensitive(self) -> bool:
        """Whether any part of this node's tree lives in the wiping
        allocator: a secret, which `repr` does not show."""
        node = self._node()
        return node is not None and bool(_lib.core().guatiao_value_is_sensitive(ctypes.byref(node)))

    @contextlib.contextmanager
    def reveal(self) -> Iterator[memoryview]:
        """The bytes of this string, number or bytes node where they live:
        a read-only `memoryview` over the tree's own memory, no copy made,
        released when the `with` block ends. A secret read this way is
        wiped with its tree; `bytes(view)` or `str(...)` makes a copy
        Python keeps and guatiao cannot wipe.

        The view, and every slice taken from it, holds the tree alive: while
        any of them lives, `close()`, `wipe()`, handing the tree to a slot,
        lending it and every change to it raise `BufferError`, so none of
        them can read memory that was freed or moved. A slice kept in a
        reference cycle holds it until the garbage collector frees the
        cycle; one still alive when the interpreter exits keeps its tree
        from being freed, and so from being wiped, until the process ends.
        A `reveal()` on another thread while the value is being closed,
        wiped or handed over raises `BufferError`.

        Of a node a call lends or borrows, which its caller frees once the
        call returns, the view is over a copy made in the wiping
        allocator, wiped when the last view and slice of it go."""
        owner = self.owner
        if not isinstance(owner, Value):
            self._revealable()
            with self._private_copy().reveal() as view:
                yield view
            return
        with owner._lock:
            if owner._busy is not None:
                raise BufferError(f"this value is {owner._busy}: it cannot be revealed now")
            storage = self._revealable()
            holder = None if storage.len == 0 else _revealed(owner, storage.ptr, storage.len)
        if holder is None:
            view = memoryview(b"")
            try:
                yield view
            finally:
                view.release()
            return
        base = memoryview(holder).cast("B")
        del holder
        view = base.toreadonly()
        try:
            yield view
        finally:
            view.release()
            base.release()

    def _revealable(self) -> Any:
        """The text or bytes arm of this node, which `reveal()` reads."""
        node = self._require_node()
        if node.tag in (_abi.Tag.STRING, _abi.Tag.NUMBER):
            return node.payload.text
        if node.tag == _abi.Tag.BYTES:
            return node.payload.bytes
        raise TypeError("only a string, a number or bytes can be revealed")

    def _private_copy(self) -> "Value":
        """This node copied into the wiping allocator, as a `Value` of
        its own."""
        node = self._require_node()
        alloc = wiping_alloc()
        out = _abi.Value()
        check(
            _lib.core().guatiao_value_clone(
                ctypes.byref(alloc), ctypes.byref(node), ctypes.byref(out)
            )
        )
        return Value(_raw=out, alloc=alloc)

    def is_absent(self) -> bool:
        return self.tag == _abi.Tag.ABSENT

    def is_null(self) -> bool:
        return self.tag == _abi.Tag.NULL

    def bool_or(self, fallback: bool = False) -> bool:
        return _bool_or(self._node(), fallback)

    def int_or(self, fallback: int = 0) -> int:
        return _int_or(self._node(), fallback)

    def float_or(self, fallback: float = 0.0) -> float:
        return _float_or(self._node(), fallback)

    def str_or(self, fallback: str = "") -> str:
        return _str_or(self._node(), fallback)

    def to_python(self, numbers: str = "auto") -> Any:
        return _node_to_python(self._node(), numbers)

    def as_map(self) -> "Map":
        if self.tag != _abi.Tag.MAP:
            raise TypeError("value is not a map")
        return Map(self.owner, self.path)

    def as_list(self) -> "List":
        if self.tag != _abi.Tag.LIST:
            raise TypeError("value is not a list")
        return List(self.owner, self.path)

    def to_wire(self) -> bytes:
        """This node in the wire encoding: a tag byte per node, LEB128
        lengths, a number as its own text, a map in its order. The same
        tree comes back from `Value.from_wire`, in any language."""
        node = self._require_node()
        lib = _lib.core()
        out = _abi.Value()
        check(lib.guatiao_wire_encode(ctypes.byref(lib.alloc), ctypes.byref(node), ctypes.byref(out)))
        try:
            return _buffer_view(out.payload.bytes)
        finally:
            _free(out)

    def clone(self) -> "Value":
        """A deep, independent copy of this node, as its own `Value`."""
        node = self._require_node()
        alloc = self.owner._clone_alloc()
        out = _abi.Value()
        check(
            _lib.core().guatiao_value_clone(
                ctypes.byref(alloc), ctypes.byref(node), ctypes.byref(out)
            )
        )
        return Value(_raw=out, alloc=alloc)

    def __repr__(self) -> str:
        try:
            if self.is_sensitive:
                return "<guatiao.Ref [sensitive]>"
            return f"<guatiao.Ref {self.to_python()!r}>"
        except Exception:
            return "<guatiao.Ref>"


class Map(Ref):
    """A `Ref` whose node is a map: the mapping protocol over it."""

    def __len__(self) -> int:
        node = self._node()
        return node.payload.map.len if node is not None and node.tag == _abi.Tag.MAP else 0

    def __contains__(self, key: str) -> bool:
        return _map_find(self._node(), key.encode("utf-8")) is not None

    def __getitem__(self, key: str) -> Ref:
        if _map_find(self._node(), key.encode("utf-8")) is None:
            raise KeyError(key)
        return Ref(self.owner, self.path + (key,))

    @_changes
    def __setitem__(self, key: str, value: Any) -> None:
        self._writable()
        alloc = self.owner._ensure_alloc()
        tmp = _abi.Value()
        _write_python(tmp, value, alloc)
        try:
            key_view, _buf = _make_str(key.encode("utf-8"))
            check(
                _lib.core().guatiao_map_set(
                    ctypes.byref(alloc), self._ptr(), key_view, ctypes.byref(tmp)
                )
            )
        finally:
            _free(tmp)

    @_changes
    def __delitem__(self, key: str) -> None:
        self._writable()
        key_view, _buf = _make_str(key.encode("utf-8"))
        check(_lib.core().guatiao_map_discard(self._ptr(), key_view))

    def __iter__(self) -> Iterator[str]:
        return iter(self.keys())

    def get(self, key: str, default: Any = None) -> Any:
        """The plain Python value at `key` (as `to_python()` reads it),
        or `default` when the map has no such key."""
        node = _map_find(self._node(), key.encode("utf-8"))
        return default if node is None else _node_to_python(node, "auto")

    def keys(self) -> "list[str]":
        node = self._require_node()
        if node.tag != _abi.Tag.MAP:
            raise TypeError("value is not a map")
        m = node.payload.map
        return [_string_view(m.ptr[i].key).decode("utf-8") for i in range(m.len)]

    def items(self) -> "Iterator[tuple[str, Ref]]":
        for key in self.keys():
            yield key, self[key]

    @_changes
    def clear(self) -> None:
        self._writable()
        check(_lib.core().guatiao_map_clear(self._ptr()))

    @_changes
    def copy_from(self, src: Ref) -> None:
        """Copies every entry of `src`, replacing collisions in place."""
        self._writable()
        alloc = self.owner._ensure_alloc()
        check(
            _lib.core().guatiao_map_copy_from(
                ctypes.byref(alloc), self._ptr(), src._ptr()
            )
        )


class List(Ref):
    """A `Ref` whose node is a list: the sequence protocol over it.

    `guatiao_list` exports only append (`push`), discard-by-index and
    clear -- there is no native "set" or "insert" at a position.
    `__setitem__` and `insert` are built from what exists: every element
    is cloned, the list is cleared, and the new order is pushed back.
    That makes them O(n); `append` and `del list[i]` stay O(1)-ish,
    through their own direct export.
    """

    def __len__(self) -> int:
        node = self._node()
        return node.payload.list.len if node is not None and node.tag == _abi.Tag.LIST else 0

    def _checked_index(self, index: int) -> int:
        node = self._require_node()
        if node.tag != _abi.Tag.LIST:
            raise TypeError("value is not a list")
        length = node.payload.list.len
        if index < 0:
            index += length
        if not (0 <= index < length):
            raise IndexError(index)
        return index

    def __getitem__(self, index: int) -> Ref:
        index = self._checked_index(index)
        return Ref(self.owner, self.path + (index,))

    @_changes
    def __delitem__(self, index: int) -> None:
        self._writable()
        index = self._checked_index(index)
        check(_lib.core().guatiao_list_discard(self._ptr(), index))

    def __iter__(self) -> Iterator[Ref]:
        for i in range(len(self)):
            yield Ref(self.owner, self.path + (i,))

    @_changes
    def append(self, value: Any) -> None:
        self._writable()
        alloc = self.owner._ensure_alloc()
        tmp = _abi.Value()
        _write_python(tmp, value, alloc)
        try:
            check(_lib.core().guatiao_list_push(ctypes.byref(alloc), self._ptr(), ctypes.byref(tmp)))
        finally:
            _free(tmp)

    @_changes
    def clear(self) -> None:
        self._writable()
        check(_lib.core().guatiao_list_clear(self._ptr()))

    def _cloned_elements(self) -> "list[_abi.Value]":
        """Clones of every element; none is left allocated when one fails."""
        node = self._require_node()
        if node.tag != _abi.Tag.LIST:
            raise TypeError("value is not a list")
        alloc = self.owner._ensure_alloc()
        lib = _lib.core()
        out: "list[_abi.Value]" = []
        try:
            for i in range(node.payload.list.len):
                src = node.payload.list.ptr[i]
                tmp = _abi.Value()
                out.append(tmp)
                check(
                    lib.guatiao_value_clone(
                        ctypes.byref(alloc), ctypes.byref(src), ctypes.byref(tmp)
                    )
                )
        except BaseException:
            _free_all(out)
            raise
        return out

    def _rebuild(self, elements: "list[_abi.Value]") -> None:
        alloc = self.owner._ensure_alloc()
        lib = _lib.core()
        check(lib.guatiao_list_clear(self._ptr()))
        for tmp in elements:
            check(lib.guatiao_list_push(ctypes.byref(alloc), self._ptr(), ctypes.byref(tmp)))

    @_changes
    def __setitem__(self, index: int, value: Any) -> None:
        self._writable()
        elements = self._cloned_elements()
        try:
            index = index + len(elements) if index < 0 else index
            if not (0 <= index < len(elements)):
                raise IndexError(index)
            replacement = _abi.Value()
            _write_python(replacement, value, self.owner._ensure_alloc())
            _free(elements[index])
            elements[index] = replacement
            self._rebuild(elements)
        finally:
            _free_all(elements)

    @_changes
    def insert(self, index: int, value: Any) -> None:
        self._writable()
        elements = self._cloned_elements()
        try:
            length = len(elements)
            index = max(0, length + index) if index < 0 else index
            index = min(index, length)
            replacement = _abi.Value()
            _write_python(replacement, value, self.owner._ensure_alloc())
            elements.insert(index, replacement)
            self._rebuild(elements)
        finally:
            _free_all(elements)


_UNSET: Any = object()


class Value(Ref):
    """Owns a root `guatiao_value`, freed on `close()`/`__del__`/`__exit__`."""

    def __init__(
        self,
        obj: Any = _UNSET,
        /,
        *,
        _raw: "_abi.Value | None" = None,
        alloc: "_abi.Alloc | None" = None,
        **fields: Any,
    ) -> None:
        """`Value(obj)` converts as `from_python` does; `Value(k=v, ...)` is a
        map, and `Value(mapping, k=v)` adds to one, as `dict` does. `Value()`
        is absent. `alloc` is reserved, so it cannot be a keyword field."""
        self._raw = _raw if _raw is not None else _abi.Value()
        self._alloc_override = alloc
        # The holders of views `reveal()` answered that are still alive.
        self._reveals: "weakref.WeakValueDictionary[int, Any]" = weakref.WeakValueDictionary()
        # Held while a reveal is recorded or the tree is claimed, so the two
        # never interleave across threads. `_busy` says what has the tree
        # (`_CLOSING`, `_CHANGING`, ...), `_busy_by` which thread, and
        # `_depth` how many changes that thread has nested.
        self._lock = threading.Lock()
        self._busy: "str | None" = None
        self._busy_by = 0
        self._depth = 0
        self._closed = False
        Ref.__init__(self, self, ())
        if obj is _UNSET and not fields:
            return
        if _raw is not None:
            raise TypeError("_raw takes no content")
        if fields:
            if obj is _UNSET:
                obj = fields
            elif isinstance(obj, Mapping):
                obj = {**obj, **fields}
            else:
                raise TypeError("keyword fields need a mapping, or nothing, to add to")
        _write_python(self._raw, obj, self._ensure_alloc())

    def _check_open(self) -> None:
        if self._closed:
            raise ValueError("this value is closed")

    def _check_unrevealed(self) -> None:
        if self._reveals:
            raise BufferError(
                "a view reveal() answered, or a slice of one, is still alive: "
                "release it and drop every slice before changing or freeing this value"
            )

    def _check_writable(self) -> None:
        self._check_open()
        self._check_unrevealed()

    def _claim(self, what: str) -> None:
        """Claims the tree for `what` until `_release()`: `BufferError`
        while a view `reveal()` answered lives or another claim holds it,
        except a change nested in a change on the same thread."""
        with self._lock:
            self._check_open()
            if self._busy is not None:
                if what == _CHANGING == self._busy and self._busy_by == threading.get_ident():
                    self._depth += 1
                    return
                raise BufferError(f"this value is {self._busy}: it cannot be {what} now")
            self._check_unrevealed()
            self._busy, self._busy_by, self._depth = what, threading.get_ident(), 1

    def _release(self) -> None:
        with self._lock:
            self._depth -= 1
            if self._depth == 0:
                self._busy = None

    @contextlib.contextmanager
    def _claimed(self, what: str) -> Iterator[None]:
        self._claim(what)
        try:
            yield
        finally:
            self._release()

    def _ensure_alloc(self) -> "_abi.Alloc":
        if self._alloc_override is not None:
            return self._alloc_override
        return _lib.core().alloc

    def _clone_alloc(self) -> "_abi.Alloc":
        return self._ensure_alloc()

    def wipe(self) -> None:
        """Overwrites with zeros every byte this tree holds, in whatever
        allocator each part lives, then frees it: this value is null
        afterwards. What a secret built outside the wiping allocator needs
        before it goes; one built there is wiped by `close()` as well."""
        with self._claimed(_WIPING):
            check(_lib.core().guatiao_value_wipe(ctypes.byref(self._raw)))

    def close(self) -> None:
        """Frees the tree, wiping what lives in the wiping allocator.
        `BufferError` while a view `reveal()` answered, or a slice of one,
        lives, or while the value is lent to a call, changed, wiped, handed
        over or closed on another thread."""
        if getattr(self, "_closed", True):
            return
        with self._lock:
            if self._closed:
                return
            if self._busy is not None:
                raise BufferError(f"this value is {self._busy}: it cannot be closed now")
            self._check_unrevealed()
            self._busy, self._busy_by, self._depth = _CLOSING, threading.get_ident(), 1
        try:
            _lib.core().guatiao_value_free(ctypes.byref(self._raw))
        finally:
            # Closed before the claim ends, so nothing claims a freed tree.
            with self._lock:
                self._closed = True
                self._busy, self._depth = None, 0

    def __del__(self) -> None:
        # A view still alive here is in a cycle being collected, or the
        # interpreter is exiting: the tree is left unfreed, and unwiped,
        # rather than freed under it.
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "Value":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    @classmethod
    def null(cls) -> "Value":
        raw = _abi.Value()
        check(_lib.core().guatiao_value_null(ctypes.byref(raw)))
        return cls(_raw=raw)

    @classmethod
    def absent(cls) -> "Value":
        raw = _abi.Value()
        check(_lib.core().guatiao_value_absent(ctypes.byref(raw)))
        return cls(_raw=raw)

    @classmethod
    def bool(cls, b: bool) -> "Value":
        raw = _abi.Value()
        check(_lib.core().guatiao_value_bool(bool(b), ctypes.byref(raw)))
        return cls(_raw=raw)

    @classmethod
    def map(cls) -> "Value":
        value = cls()
        check(_lib.core().guatiao_value_map(ctypes.byref(value._ensure_alloc()), ctypes.byref(value._raw)))
        return value

    @classmethod
    def list(cls) -> "Value":
        value = cls()
        check(_lib.core().guatiao_value_list(ctypes.byref(value._ensure_alloc()), ctypes.byref(value._raw)))
        return value

    @classmethod
    def string(cls, text: str) -> "Value":
        value = cls()
        view, _buf = _make_str(text.encode("utf-8"))
        check(
            _lib.core().guatiao_value_string(
                ctypes.byref(value._ensure_alloc()), view, ctypes.byref(value._raw)
            )
        )
        return value

    @classmethod
    def number(cls, text: str) -> "Value":
        """A number built from its exact text, verbatim -- no float or
        Decimal detour, matching `guatiao_value_number` directly."""
        value = cls()
        view, _buf = _make_str(text.encode("ascii"))
        check(
            _lib.core().guatiao_value_number(
                ctypes.byref(value._ensure_alloc()), view, ctypes.byref(value._raw)
            )
        )
        return value

    @classmethod
    def bytes_(cls, data: bytes) -> "Value":
        value = cls()
        view, _buf = _make_bytes(bytes(data))
        check(
            _lib.core().guatiao_value_bytes(
                ctypes.byref(value._ensure_alloc()), view, ctypes.byref(value._raw)
            )
        )
        return value

    @classmethod
    def from_wire(cls, data: bytes, *, alloc: "_abi.Alloc | None" = None) -> "Value":
        """The value the wire encoding in `data` holds.

        Raises `BadValue` for bytes that are not exactly one value:
        truncated, an unknown tag, a length not in its shortest form, text
        that is not UTF-8, a number outside the grammar, a repeated key,
        anything after the value."""
        value = cls(alloc=alloc)
        view, _buf = _make_bytes(bytes(data))
        raise_for_status(
            _lib.core().guatiao_wire_decode(
                ctypes.byref(value._ensure_alloc()), view, ctypes.byref(value._raw)
            ),
            "bytes that are not a value in the wire encoding",
        )
        return value

    @classmethod
    def from_python(cls, obj: Any, *, alloc: "_abi.Alloc | None" = None) -> "Value":
        return cls(obj, alloc=alloc)

    def __repr__(self) -> str:
        if self._closed:
            return "<guatiao.Value closed>"
        try:
            if self.is_sensitive:
                return "<guatiao.Value [sensitive]>"
            return f"<guatiao.Value {self.to_python()!r}>"
        except Exception:
            return "<guatiao.Value>"
