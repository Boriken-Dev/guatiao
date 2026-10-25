# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What crosses a kind slot, and how each shape is passed and received.

A shape converts in both directions: a CALLER (a Python host calling a
table) passes a Python value and reads the answer; a CALLEE (a table this
binding built over a Python object) receives the C arguments and writes
the answer. The rules are the C header's: a borrowed tree is read for the
call, a tree handed over is the callee's on every path, a lent tree is
the caller's and is changed in place, and an answer is built in the
allocator the caller's out slot carries.
"""

from __future__ import annotations

import ctypes
from collections.abc import Mapping
from typing import Any, Callable

from .. import _abi, _lib
from ..errors import BadValue, NullArgument, raise_for_status
from ..value import List, Map, Ref, Value, _make_bytes, _make_str

_P = ctypes.POINTER


class Scope:
    """What one call keeps alive, and what runs once it returns.

    Views handed to a callee check it on every access, so one kept past
    the call raises instead of reading memory that is no longer lent."""

    def __init__(self) -> None:
        self.open = True
        self._keep: "list[Any]" = []
        self._after: "list[Callable[[], Any]]" = []
        # Trees and objects given up for a call that may not happen: freed
        # by `close()` unless `called()` says the callee took them.
        self._given: "list[Callable[[], Any]]" = []

    def keep(self, obj: Any) -> Any:
        self._keep.append(obj)
        return obj

    def after(self, fn: Callable[[], Any]) -> None:
        self._after.append(fn)

    def given(self, release: Callable[[], Any]) -> None:
        self._given.append(release)

    def called(self) -> None:
        self._given.clear()

    def _check_open(self) -> None:
        if not self.open:
            raise ValueError("this view was lent for one call, which has returned")

    def close(self) -> None:
        if not self.open:
            return
        self.open = False
        failure = None
        for fn in reversed(self._after):
            try:
                fn()
            except BaseException as e:  # every step runs; the first failure is raised
                failure = failure or e
        for release in reversed(self._given):
            try:
                release()
            except BaseException as e:
                failure = failure or e
        self._keep.clear()
        if failure is not None:
            raise failure


class _View:
    """The owner of a `Ref` onto memory a call lends: valid while the
    scope is open, writable only when lent, never freed here."""

    def __init__(self, node: Any, scope: Scope, *, alloc: Any = None) -> None:
        self._raw = node
        self._scope = scope
        self._alloc = alloc

    def _check_open(self) -> None:
        self._scope._check_open()

    def _check_writable(self) -> None:
        self._scope._check_open()
        if self._alloc is None:
            raise TypeError("a borrowed value is read-only")

    def _ensure_alloc(self) -> Any:
        self._check_writable()
        return self._alloc

    def _clone_alloc(self) -> Any:
        return _lib.core().alloc


def binding_alloc() -> Any:
    """The allocator this binding builds in when nobody names one."""
    return _lib.core().alloc


def complete(alloc_ptr: Any) -> bool:
    """Whether a `guatiao_alloc *` names an allocator that can build."""
    if not alloc_ptr:
        return False
    a = alloc_ptr.contents
    floor = _abi.Alloc.free.offset + ctypes.sizeof(ctypes.c_void_p)
    return a.struct_size >= floor and bool(a.alloc) and bool(a.free)


def _move_out(value: Value) -> "_abi.Value":
    """The root node of `value`, moved into a fresh struct: `value` holds
    ABSENT afterwards and frees nothing."""
    value._check_open()
    moved = _abi.Value.from_buffer_copy(value._raw)
    ctypes.memset(ctypes.addressof(value._raw), 0, ctypes.sizeof(_abi.Value))
    return moved


def _owned_tree(value: Any, tag: "int | None", alloc: Any) -> "_abi.Value":
    """A root node holding `value`'s tree, given up by whoever held it: a
    `Value`, or a view of a whole `Value`, is moved; another `Ref` cloned;
    anything else built in `alloc`."""
    if isinstance(value, Ref) and not value.path and isinstance(value.owner, Value):
        value = value.owner
    if isinstance(value, Value):
        if tag is not None and value.tag != tag:
            raise TypeError(f"expected a {_abi.Tag(tag).name.lower()}, got {_abi.Tag(value.tag).name.lower()}")
        return _move_out(value)
    built = value.clone() if isinstance(value, Ref) else Value(value, alloc=alloc)
    if tag is not None and built.tag != tag:
        built.close()
        raise TypeError(f"expected a {_abi.Tag(tag).name.lower()}")
    return _move_out(built)


def _free_node(node: "_abi.Value") -> None:
    _lib.core().guatiao_value_free(ctypes.byref(node))


def _makes_local() -> bool:
    """Whether the core moves a tree home: a library older than
    `guatiao_value_make_local` leaves a tree as the slot built it."""
    global _MAKES_LOCAL
    if _MAKES_LOCAL is None:
        _MAKES_LOCAL = "guatiao_value_make_local" not in _lib.core().symbols_missing()
    return _MAKES_LOCAL


_MAKES_LOCAL: "bool | None" = None


def _make_local(node: "_abi.Value") -> None:
    """Moves a tree a slot answered into this binding's allocator, so it
    outlives the library: a slot that ignored the carrier built it in its
    own. Freed, and the refusal raised, when the move fails."""
    if not _makes_local():
        return
    status = _lib.core().guatiao_value_make_local(ctypes.byref(node), ctypes.byref(binding_alloc()))
    if status != _abi.Status.OK:
        _free_node(node)
        raise_for_status(status, "moving the answer into this binding's allocator")


def _settle(node: "_abi.Value", home: Any) -> None:
    """Moves a tree lent for a call where it stands into `home`, the
    allocator it recorded when it was lent, so what the callee grew it
    with, or wrote over it, outlives the callee's library. A refusal
    leaves it whole, where it was."""
    if _makes_local():
        _lib.core().guatiao_value_make_local(ctypes.byref(node), ctypes.byref(home))


def _recorded_alloc(node: "_abi.Value") -> Any:
    """The allocator a container node records, else this binding's."""
    arm = {
        _abi.Tag.MAP: "map",
        _abi.Tag.LIST: "list",
        _abi.Tag.STRING: "text",
        _abi.Tag.NUMBER: "number",
        _abi.Tag.BYTES: "bytes",
    }.get(node.tag)
    if arm is not None:
        ptr = getattr(node.payload, arm).alloc
        if complete(ptr):
            return ptr.contents
    return binding_alloc()


def _text_of(view: Any, what: str) -> str:
    if not view.len:
        return ""
    try:
        return ctypes.string_at(view.ptr, view.len).decode("utf-8")
    except UnicodeDecodeError:
        raise BadValue(int(_abi.Status.ERR_BAD_VALUE), f"{what} is not UTF-8") from None


class Shape:
    """One argument or answer of a kind slot, as the C header spells it."""

    name = "shape"
    #: The ctypes type in the slot's prototype, as an argument.
    c_arg: Any = None
    #: The ctypes type the answer is written into, through `out`.
    c_out: Any = None
    #: Handed over: the callee owns it on every path.
    takes = False

    def __repr__(self) -> str:
        return f"guatiao.kinds.shapes.{self.name}"

    # -- the caller's side ----------------------------------------------

    def pass_(self, value: Any, scope: Scope) -> Any:
        raise TypeError(f"{self!r} is not an argument")

    def out_slot(self, scope: Scope) -> Any:
        raise TypeError(f"{self!r} is not an answer")

    def answer(self, out: Any, scope: Scope) -> Any:
        raise TypeError(f"{self!r} is not an answer")

    # -- the callee's side ----------------------------------------------

    def receive(self, c_value: Any, scope: Scope) -> Any:
        raise TypeError(f"{self!r} is not an argument")

    def write(self, out_ptr: Any, result: Any) -> None:
        raise TypeError(f"{self!r} is not an answer")


class _Scalar(Shape):
    def __init__(self, name: str, ctype: Any, kind: str, bits: int = 0) -> None:
        self.name = name
        self.c_arg = ctype
        self.c_out = ctype
        self._kind = kind
        if kind == "int":
            signed = name.startswith("I")
            self._lo = -(1 << (bits - 1)) if signed else 0
            self._hi = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1

    def _checked(self, value: Any) -> Any:
        if self._kind == "bool":
            if not isinstance(value, (bool, int)):
                raise TypeError(f"{self!r} takes a bool, not {type(value).__name__}")
            return bool(value)
        if self._kind == "float":
            if not isinstance(value, (int, float)) or isinstance(value, bool):
                raise TypeError(f"{self!r} takes a number, not {type(value).__name__}")
            return float(value)
        if not isinstance(value, int) or isinstance(value, bool):
            raise TypeError(f"{self!r} takes an int, not {type(value).__name__}")
        if not self._lo <= value <= self._hi:
            raise OverflowError(f"{value} does not fit {self!r}")
        return value

    def pass_(self, value: Any, scope: Scope) -> Any:
        return self._checked(value)

    def out_slot(self, scope: Scope) -> Any:
        return self.c_out()

    def answer(self, out: Any, scope: Scope) -> Any:
        return out.value

    def receive(self, c_value: Any, scope: Scope) -> Any:
        return bool(c_value) if self._kind == "bool" else c_value

    def write(self, out_ptr: Any, result: Any) -> None:
        out_ptr[0] = self._checked(result)


class _Str(Shape):
    name = "STR"
    c_arg = _abi.Str

    def pass_(self, value: Any, scope: Scope) -> Any:
        if not isinstance(value, str):
            raise TypeError(f"STR takes a str, not {type(value).__name__}")
        view, buf = _make_str(value.encode("utf-8"))
        scope.keep(buf)
        return view

    def receive(self, c_value: Any, scope: Scope) -> Any:
        return _text_of(c_value, "the text argument")


class _Bytes(Shape):
    name = "BYTES"
    c_arg = _abi.Bytes

    def pass_(self, value: Any, scope: Scope) -> Any:
        view, buf = _make_bytes(bytes(value))
        scope.keep(buf)
        return view

    def receive(self, c_value: Any, scope: Scope) -> Any:
        return ctypes.string_at(c_value.ptr, c_value.len) if c_value.len else b""


class _BytesMut(Shape):
    """An out-buffer: the caller's writable memory, for the call."""

    name = "BYTES_MUT"
    c_arg = _abi.BytesMut

    def pass_(self, value: Any, scope: Scope) -> Any:
        view = memoryview(value)
        if view.readonly:
            raise TypeError("BYTES_MUT takes a writable buffer, such as a bytearray")
        n = view.nbytes
        if not n:
            return _abi.BytesMut(ptr=None, len=0)
        array = (ctypes.c_char * n).from_buffer(value)
        scope.keep(array)
        return _abi.BytesMut(ptr=ctypes.cast(array, ctypes.c_void_p), len=n)

    def receive(self, c_value: Any, scope: Scope) -> Any:
        # A copy the method writes into, copied back once it returns: a
        # memoryview over the caller's memory would outlive the call.
        n = c_value.len
        buf = bytearray(ctypes.string_at(c_value.ptr, n)) if n else bytearray()
        if n:
            scope.after(
                lambda: ctypes.memmove(c_value.ptr, bytes(buf[:n]), min(n, len(buf)))
            )
        return buf


class _Text(Shape):
    """`String` as an answer: owned text, read and freed by the caller."""

    name = "TEXT"
    c_out = _abi.String

    def out_slot(self, scope: Scope) -> Any:
        return _abi.String()

    def answer(self, out: Any, scope: Scope) -> Any:
        node = _abi.Value(tag=int(_abi.Tag.STRING), payload=_abi.Payload(text=out))
        try:
            return _text_of(out, "the answer")
        finally:
            _free_node(node)

    def write(self, out_ptr: Any, result: Any) -> None:
        if not isinstance(result, str):
            raise TypeError(f"a TEXT answer is a str, not {type(result).__name__}")
        built = Value.string(result)
        moved = _move_out(built)
        out_ptr[0] = moved.payload.text


class _Tree(Shape):
    """A value, a map or a list, in one of the ways a tree crosses."""

    def __init__(self, name: str, tag: "int | None", mode: str) -> None:
        self.name = name
        self.tag = tag
        self.mode = mode  # "borrow", "take", "lend", "answer"
        self.takes = mode == "take"
        struct: Any = _abi.Value if tag is None else (_abi.Map if tag == _abi.Tag.MAP else _abi.List)
        self._struct = struct
        if mode == "answer":
            self.c_out = struct
        else:
            self.c_arg = _P(struct)

    # -- a container's node and the struct that crosses -------------------

    def _arm(self, node: "_abi.Value") -> Any:
        """The struct that crosses, inside `node`: the node itself for a
        value, its payload arm for a map or list."""
        if self.tag is None:
            return node
        if node.tag != self.tag:
            raise TypeError(f"expected a {_abi.Tag(self.tag).name.lower()}")
        return node.payload.map if self.tag == _abi.Tag.MAP else node.payload.list

    def _node_over(self, struct: Any) -> "_abi.Value":
        """A fresh node holding a copy of `struct`'s fields."""
        if self.tag is None:
            return _abi.Value.from_buffer_copy(struct)
        node = _abi.Value(tag=int(self.tag))
        ctypes.memmove(ctypes.addressof(node) + _abi.Value.payload.offset,
                       ctypes.addressof(struct), ctypes.sizeof(self._struct))
        return node

    def _wrap(self, ref: Ref) -> Ref:
        if self.tag == _abi.Tag.MAP:
            return Map(ref.owner, ref.path)
        if self.tag == _abi.Tag.LIST:
            return List(ref.owner, ref.path)
        return ref

    # -- the caller ---------------------------------------------------------

    def pass_(self, value: Any, scope: Scope) -> Any:
        if self.mode == "borrow":
            if value is None and self.tag is None:
                return None
            if not isinstance(value, Ref):
                value = scope.keep(Value(value))
                scope.after(value.close)
            return ctypes.byref(self._arm(value._require_node()))
        if self.mode == "lend":
            if not isinstance(value, Ref):
                raise TypeError(
                    f"{self!r} takes a Value or a Ref, so what the callee changes has somewhere to go"
                )
            value._writable()
            node = value._require_node()
            # What the callee grew it with may be in its library's
            # allocator: moved into the one the tree records now, which a
            # callee writing a fresh tree over it would change.
            home = _recorded_alloc(node)
            scope.after(lambda: _settle(node, home))
            return ctypes.byref(self._arm(node))
        # Handed over: given up now, and freed by `close()` if the call
        # never happens.
        node = _owned_tree(value, self.tag, binding_alloc())
        scope.keep(node)
        scope.given(lambda: _free_node(node))
        return ctypes.byref(self._arm(node))

    def out_slot(self, scope: Scope) -> Any:
        # The carrier: owns nothing, and names the allocator to build in.
        alloc = ctypes.pointer(binding_alloc())
        scope.keep(alloc)
        if self.tag is None:
            out = _abi.Value(tag=int(_abi.Tag.ABSENT))
            out.payload.map.alloc = alloc
            return out
        return self._struct(alloc=alloc)

    def answer(self, out: Any, scope: Scope) -> Any:
        node = out if self.tag is None else self._node_over(out)
        _make_local(node)
        return Value(_raw=node, alloc=binding_alloc())

    # -- the callee ---------------------------------------------------------

    def receive(self, c_value: Any, scope: Scope) -> Any:
        if not c_value:
            if self.mode == "borrow" and self.tag is None:
                return None
            raise NullArgument(int(_abi.Status.ERR_NULL), f"the {self.name} argument is null")
        if self.mode == "take":
            owned = Value(_raw=self._node_over(c_value.contents), alloc=binding_alloc())
            return self._wrap(Ref(owned, ())) if self.tag is not None else owned
        if self.tag is None:
            node = c_value.contents
        else:
            # A node of our own over the caller's struct: what the method
            # changes lands here, and is written back once it returns.
            node = self._node_over(c_value.contents)
            if self.mode == "lend":
                target = ctypes.addressof(c_value.contents)
                size = ctypes.sizeof(self._struct)
                offset = _abi.Value.payload.offset
                scope.after(
                    lambda: ctypes.memmove(target, ctypes.addressof(node) + offset, size)
                )
            scope.keep(node)
        alloc = _recorded_alloc(node) if self.mode == "lend" else None
        return self._wrap(Ref(_View(node, scope, alloc=alloc), ()))

    def write(self, out_ptr: Any, result: Any) -> None:
        carrier = out_ptr.contents
        named = carrier.payload.map.alloc if self.tag is None else carrier.alloc
        alloc = named.contents if complete(named) else binding_alloc()
        node = _owned_tree(result, self.tag, alloc)
        ctypes.memmove(ctypes.addressof(carrier), ctypes.addressof(self._arm(node)),
                       ctypes.sizeof(self._struct))


class _ObjectShape(Shape):
    """An object of an object kind: handed over, lent, or answered."""

    def __init__(self, kind: Any, mode: str) -> None:
        self.kind = kind
        self.mode = mode  # "take", "lend", "answer"
        self.name = f"{'lent' if mode == 'lend' else 'taken'}({kind.name})"
        self.takes = mode == "take"
        if mode == "take":
            self.c_arg = _abi.Object
        elif mode == "lend":
            self.c_arg = _P(_abi.Object)
        else:
            self.c_out = _abi.Object

    def __repr__(self) -> str:
        if self.mode == "answer":
            return f"<object kind {self.kind.name}>"
        return f"guatiao.kinds.shapes.{self.name}"

    def _raw_for(self, value: Any, scope: "Scope | None", *, lent: bool) -> Any:
        from ._spec import Object
        from ._serve import serve

        if isinstance(value, Object):
            if value.kind is not self.kind:
                raise TypeError(f"an object of {value.kind.name} where {self.kind.name} was asked for")
            return value._raw_view() if lent else value._release()
        return serve(self.kind, value, scope if lent else None)

    def pass_(self, value: Any, scope: Scope) -> Any:
        if self.mode == "lend":
            raw = scope.keep(self._raw_for(value, scope, lent=True))
            return ctypes.byref(raw)
        raw = self._raw_for(value, None, lent=False)
        from ._spec import destroy_raw

        scope.given(lambda: destroy_raw(raw))
        return raw

    def out_slot(self, scope: Scope) -> Any:
        return _abi.Object()

    def answer(self, out: Any, scope: Scope) -> Any:
        from ._spec import Object

        return Object(self.kind, out)

    def receive(self, c_value: Any, scope: Scope) -> Any:
        from ._spec import Object, Remote

        if self.mode == "take":
            return Object(self.kind, _abi.Object.from_buffer_copy(c_value))
        if not c_value:
            raise NullArgument(int(_abi.Status.ERR_NULL), "no object was lent")
        raw = c_value.contents
        return Remote(self.kind, raw.table, raw.size, raw.ctx, scope=scope)

    def write(self, out_ptr: Any, result: Any) -> None:
        out_ptr[0] = self._raw_for(result, None, lent=False)


def taken(what: Any) -> Shape:
    """An argument the caller hands over: `taken(MAP)`, `taken(VALUE)`,
    `taken(LIST)`, or an object of an object kind, `taken(LISTENER)`."""
    if isinstance(what, _Tree) and what.mode == "borrow":
        return _Tree(f"taken({what.name})", what.tag, "take")
    if what is LIST:
        return _TAKEN_LIST
    if getattr(what, "is_object", False):
        return _ObjectShape(what, "take")
    raise TypeError(f"taken() takes VALUE, MAP, LIST or an object kind, not {what!r}")


def lent(what: Any) -> Shape:
    """An argument the caller lends for the call (`&mut`): `lent(MAP)`,
    `lent(VALUE)`, `lent(LIST)`, or an object, `lent(LISTENER)`."""
    if isinstance(what, _Tree) and what.mode == "borrow":
        return _Tree(f"lent({what.name})", what.tag, "lend")
    if what is LIST:
        return _LENT_LIST
    if getattr(what, "is_object", False):
        return _ObjectShape(what, "lend")
    raise TypeError(f"lent() takes VALUE, MAP, LIST or an object kind, not {what!r}")


def answer_shape(what: Any) -> Shape:
    """The shape of a slot's answer: a scalar, TEXT, VALUE, MAP, LIST, or
    an object kind for an object the caller then owns."""
    if getattr(what, "is_object", False):
        return _ObjectShape(what, "answer")
    if isinstance(what, _Tree):
        if what.mode == "borrow" or what is LIST:
            return _Tree(what.name, what.tag, "answer")
    if isinstance(what, Shape) and what.c_out is not None:
        return what
    raise TypeError(f"{what!r} cannot be a slot's answer")


BOOL = _Scalar("BOOL", ctypes.c_bool, "bool")
I8 = _Scalar("I8", ctypes.c_int8, "int", 8)
I16 = _Scalar("I16", ctypes.c_int16, "int", 16)
I32 = _Scalar("I32", ctypes.c_int32, "int", 32)
I64 = _Scalar("I64", ctypes.c_int64, "int", 64)
U8 = _Scalar("U8", ctypes.c_uint8, "int", 8)
U16 = _Scalar("U16", ctypes.c_uint16, "int", 16)
U32 = _Scalar("U32", ctypes.c_uint32, "int", 32)
U64 = _Scalar("U64", ctypes.c_uint64, "int", 64)
USIZE = _Scalar("USIZE", ctypes.c_size_t, "int", 64)
ISIZE = _Scalar("ISIZE", ctypes.c_ssize_t, "int", 64)
F32 = _Scalar("F32", ctypes.c_float, "float")
F64 = _Scalar("F64", ctypes.c_double, "float")
STR = _Str()
BYTES = _Bytes()
BYTES_MUT = _BytesMut()
TEXT = _Text()
VALUE = _Tree("VALUE", None, "borrow")
MAP = _Tree("MAP", _abi.Tag.MAP, "borrow")
# A list is never borrowed (the macro has no `&List` argument), so LIST is
# an answer, and `taken(LIST)` / `lent(LIST)` the arguments.
LIST = _Tree("LIST", _abi.Tag.LIST, "answer")
_TAKEN_LIST = _Tree("taken(LIST)", _abi.Tag.LIST, "take")
_LENT_LIST = _Tree("lent(LIST)", _abi.Tag.LIST, "lend")

__all__ = [
    "Shape",
    "BOOL",
    "I8",
    "I16",
    "I32",
    "I64",
    "U8",
    "U16",
    "U32",
    "U64",
    "USIZE",
    "ISIZE",
    "F32",
    "F64",
    "STR",
    "BYTES",
    "BYTES_MUT",
    "TEXT",
    "VALUE",
    "MAP",
    "LIST",
    "taken",
    "lent",
]
