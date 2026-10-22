# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A kind described for Python: its table's `ctypes.Structure`, its
`FLOOR_HASH`, and what each slot takes and answers."""

from __future__ import annotations

import ctypes
import functools
from typing import Any, Sequence

from .. import _abi
from ..errors import Unsupported, check, raise_for_status
from . import _serve
from ._shapes import Scope, Shape, answer_shape
from ._table import FloorMismatch

_WORD = ctypes.sizeof(ctypes.c_void_p)

#: What a `Remote` or an `Object` answers itself, so no method may be
#: called it. A slot is found by its position, so the Python name of a
#: method is free to differ from the header's.
RESERVED = frozenset({"has", "close", "kind", "ctx", "size"})


class Method:
    """One slot of a kind: the method's name, what it takes, what it
    answers, and whether it can fail (a `guatiao_provider_error *err`
    after everything else).

    `args` are shapes: `STR`, `MAP`, `taken(MAP)`, `lent(LISTENER)` and
    the rest. `returns` is a shape, an object kind, or `None` for a slot
    that answers nothing. `required` marks a method every table must fill:
    the ones the kind was born with, which its `FLOOR_HASH` covers. A
    method appended later is not required, and a caller finds it absent
    in an older table.
    """

    def __init__(
        self,
        name: str,
        args: Sequence[Shape] = (),
        returns: Any = None,
        *,
        fallible: bool = False,
        required: bool = False,
    ) -> None:
        for shape in args:
            if getattr(shape, "is_object", False):
                raise TypeError(
                    f"`{name}`: an object argument is taken(kind) or lent(kind), not the kind itself"
                )
            if not isinstance(shape, Shape) or shape.c_arg is None:
                raise TypeError(f"`{name}`: {shape!r} is not an argument")
        self.name = name
        self.args = tuple(args)
        self.returns = answer_shape(returns) if returns is not None else None
        self.fallible = fallible
        self.required = required

    def slot_type(self) -> Any:
        """The slot's function-pointer type, for a struct written by hand."""
        return ctypes.CFUNCTYPE(ctypes.c_uint32, *self.prototype())

    def prototype(self) -> "list[Any]":
        """The slot's argument types, as the kind's C header declares them."""
        types: "list[Any]" = [ctypes.c_void_p]
        types += [shape.c_arg for shape in self.args]
        if self.returns is not None:
            types.append(ctypes.POINTER(self.returns.c_out))
        if self.fallible:
            types.append(ctypes.POINTER(_abi.ProviderError))
        return types

    def __repr__(self) -> str:
        return f"<guatiao.kinds.Method {self.name}>"


class Kind:
    """A kind's table, for calling one (`bind`) and for serving one from
    Python objects.

    `methods` are the slots in declaration order, as the header lists
    them, and `floor_hash` is the `<table>_FLOOR_HASH` the header defines,
    passed in as written there. The table's `ctypes.Structure` is built
    from them (`header`, then `destroy` for an object kind, then one slot
    per method), or given as `struct` when one is already written; each
    method is then checked against its field, so a description that
    disagrees with the struct fails here and not in a call.
    """

    def __init__(
        self,
        name: str,
        struct: Any = None,
        *,
        floor_hash: int,
        methods: Sequence[Method],
        is_object: bool = False,
    ) -> None:
        self.name = name
        self.floor_hash = int(floor_hash)
        self.methods = tuple(methods)
        self.is_object = bool(is_object)
        # A kind class's own method bodies: placeholders, never slots.
        self.stubs: dict[str, Any] = {}
        self._by_name = {m.name: m for m in self.methods}
        for m in self.methods:
            if m.name in RESERVED or m.name.startswith("_"):
                raise TypeError(
                    f"a method named `{m.name}` would hide the handle's own; "
                    f"name it otherwise (`{m.name}_`): a slot is found by position"
                )
        if struct is None:
            built: "list[tuple[str, Any]]" = [("header", _abi.KindHeader)]
            if self.is_object:
                built.append(("destroy", ctypes.CFUNCTYPE(None, ctypes.c_void_p)))
            built += [(m.name, m.slot_type()) for m in self.methods]
            struct = type(f"{name}_vtable", (ctypes.Structure,), {"_fields_": built})
        self.struct: Any = struct
        fields = dict(struct._fields_)
        names = [f[0] for f in struct._fields_]
        if not names or names[0] != "header" or fields["header"] is not _abi.KindHeader:
            raise TypeError(f"{struct.__name__}'s first field is not a guatiao_kind_header named `header`")
        expected = ["header"] + (["destroy"] if self.is_object else []) + [m.name for m in self.methods]
        if names != expected:
            raise TypeError(f"{struct.__name__} declares {names}, the methods describe {expected}")
        if self.is_object:
            destroy = fields["destroy"]
            if list(destroy._argtypes_) != [ctypes.c_void_p] or destroy._restype_ is not None:
                raise TypeError(f"{struct.__name__}.destroy is not `void (*)(void *ctx)`")
        for method in self.methods:
            slot = fields[method.name]
            if list(slot._argtypes_) != method.prototype() or slot._restype_ is not ctypes.c_uint32:
                raise TypeError(
                    f"{struct.__name__}.{method.name} is not the prototype the method describes"
                )
        floor = getattr(struct, "header").size
        if self.is_object:
            floor = struct.destroy.offset + _WORD
        for method in self.methods:
            if method.required:
                floor = max(floor, getattr(struct, method.name).offset + _WORD)
        self.floor = floor

    def slot_type(self, name: str) -> Any:
        return dict(self.struct._fields_)[name]

    def implemented(self, cls: type, name: str) -> bool:
        """Whether `cls` implements the slot `name` itself, rather than
        inheriting the kind class's placeholder."""
        found = getattr(cls, name, None)
        return callable(found) and found is not self.stubs.get(name)

    def method(self, name: str) -> Method:
        return self._by_name[name]

    def bind(self, table_ptr: Any, size: int, ctx: Any) -> "Remote":
        """A caller's handle on a table of this kind, answered by a
        registry (`Registry.provider_table`) or by another call."""
        return Remote(self, table_ptr, size, ctx)

    def __repr__(self) -> str:
        return f"<guatiao.kinds.Kind {self.name}>"


class Remote:
    """Calls a kind's table: each method of the kind is a method here,
    converting its arguments and its answer.

    Valid while the table's library is: until it is unloaded, or, once
    retired, collected. A slot the table does not carry (a method appended
    after the library was built) raises `guatiao.Unsupported`; `has(name)`
    asks first. A refusal raises the `GuatiaoError` its status names,
    with the provider's own words in `.message`.
    """

    def __init__(self, kind: Kind, table_ptr: Any, size: int, ctx: Any, *, scope: "Scope | None" = None) -> None:
        if not table_ptr:
            raise FloorMismatch(f"no {kind.name} table")
        if size < kind.floor:
            raise FloorMismatch(f"the {kind.name} table is {size} bytes, its required slots need {kind.floor}")
        header = ctypes.cast(table_ptr, ctypes.POINTER(_abi.KindHeader)).contents
        if header.floor_hash != kind.floor_hash:
            raise FloorMismatch(
                f"the table's floor_hash {header.floor_hash} is not the {kind.name} kind's {kind.floor_hash}"
            )
        self.kind = kind
        self.size = int(size)
        self.ctx = ctx
        self._table_ptr = table_ptr
        self._table: Any = ctypes.cast(table_ptr, ctypes.POINTER(kind.struct)).contents
        self._scope = scope

    def _check(self) -> None:
        if self._scope is not None:
            self._scope._check_open()

    def _slot(self, name: str) -> Any:
        field = getattr(self.kind.struct, name)
        if field.offset + _WORD > self.size:
            return None
        fn = getattr(self._table, name)
        return fn if fn else None

    def has(self, name: str) -> bool:
        """Whether the table carries the slot for `name`."""
        self._check()
        return self._slot(name) is not None

    def __getattr__(self, name: str) -> Any:
        kind = self.__dict__.get("kind")
        if kind is None or name not in kind._by_name:
            raise AttributeError(name)
        return functools.partial(self._call, kind._by_name[name])

    def _call(self, method: Method, *args: Any) -> Any:
        self._check()
        if len(args) != len(method.args):
            raise TypeError(f"`{method.name}` takes {len(method.args)} arguments, {len(args)} given")
        fn = self._slot(method.name)
        if fn is None:
            raise Unsupported(
                int(_abi.Status.ERR_UNSUPPORTED),
                f"this {self.kind.name} table has no `{method.name}`",
            )
        scope = Scope()
        try:
            c_args: "list[Any]" = [None] * len(args)
            # Handed-over arguments last: a conversion that fails before
            # them leaves the caller's trees where they were.
            for i, shape in enumerate(method.args):
                if not shape.takes:
                    c_args[i] = shape.pass_(args[i], scope)
            for i, shape in enumerate(method.args):
                if shape.takes:
                    c_args[i] = shape.pass_(args[i], scope)
            extra: "list[Any]" = []
            out = None
            if method.returns is not None:
                out = method.returns.out_slot(scope)
                extra.append(ctypes.byref(out))
            err = None
            if method.fallible:
                err = _abi.ProviderError()
                extra.append(ctypes.byref(err))
            status = fn(self.ctx, *c_args, *extra)
            scope.called()
            if status != _abi.Status.OK:
                if err is not None:
                    check(status, err)
                raise_for_status(status, f"`{method.name}` failed, which its kind says it cannot")
            return method.returns.answer(out, scope) if method.returns is not None else None
        finally:
            scope.close()

    def __repr__(self) -> str:
        return f"<guatiao.kinds.Remote {self.kind.name}>"


def destroy_raw(raw: "_abi.Object") -> None:
    """Calls an object's `destroy` slot, the first after its header."""
    if raw.table and raw.size >= 8 + _WORD:
        slot = ctypes.cast(raw.table + 8, ctypes.POINTER(ctypes.c_void_p)).contents
        if slot.value:
            ctypes.CFUNCTYPE(None, ctypes.c_void_p)(slot.value)(raw.ctx)


class Object(Remote):
    """An object of an object kind that this side owns: a `Remote` whose
    `close()` calls its `destroy` slot, once. Handed to a slot that takes
    it, it is the callee's, and closing it here does nothing."""

    def __init__(self, kind: Kind, raw: "_abi.Object") -> None:
        if not kind.is_object:
            raise TypeError(f"{kind.name} is not an object kind")
        self._raw = _abi.Object.from_buffer_copy(raw)
        self._open = False
        # A table that is not the kind's is refused and the object left
        # alone: its slot after the header need not be a `destroy`.
        super().__init__(kind, raw.table, raw.size, raw.ctx)
        self._open = True

    def _check(self) -> None:
        if not self._open:
            raise ValueError("this object is closed, or was handed over")

    def _raw_view(self) -> "_abi.Object":
        self._check()
        return self._raw

    def _release(self) -> "_abi.Object":
        self._check()
        self._open = False
        return self._raw

    def close(self) -> None:
        if getattr(self, "_open", False):
            self._open = False
            destroy_raw(self._raw)

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "Object":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def __repr__(self) -> str:
        return f"<guatiao.kinds.Object {self.kind.name}>"


def live_objects() -> int:
    """How many Python objects a C caller can still reach: objects and
    instances handed out and not yet destroyed or released."""
    return _serve.live()
