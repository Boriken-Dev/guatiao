# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A kind written as a typed `Protocol` class: its slots are its methods,
in declaration order, and what each takes and answers is read from the
annotations."""

from __future__ import annotations

import inspect
import typing
from collections.abc import Mapping, Sequence
from typing import Any, Callable, ClassVar, Optional, Protocol, TypeVar, Union

from ..value import List, Map, Ref, Value
from . import _shapes as shapes
from ._spec import Kind, Method

F = TypeVar("F", bound=Callable[..., Any])
T = TypeVar("T")


class _Mark:
    """What an `Annotated` alias says about the type it wraps."""

    def __init__(self, what: str, shape: Optional[shapes.Shape] = None) -> None:
        self.what = what
        self.shape = shape

    def __repr__(self) -> str:
        return f"<guatiao.kinds {self.what}>"


_LENT = _Mark("lent")
_TAKEN = _Mark("taken")

#: An argument lent for the call (`&mut` in Rust): `Lent[Map]`, `Lent[List]`,
#: `Lent[Ref]` (a value), or an object kind, `Lent[Listener]`. A tree arrives
#: as a writable view, valid for the call, changed where the caller holds it.
Lent = typing.Annotated[T, _LENT]
#: An argument handed over with the call: `Taken[Map]`, `Taken[List]`,
#: `Taken[Value]`, or an object kind, `Taken[Listener]`. The callee owns it
#: on every path.
Taken = typing.Annotated[T, _TAKEN]

I8 = typing.Annotated[int, _Mark("I8", shapes.I8)]
I16 = typing.Annotated[int, _Mark("I16", shapes.I16)]
I32 = typing.Annotated[int, _Mark("I32", shapes.I32)]
I64 = typing.Annotated[int, _Mark("I64", shapes.I64)]
U8 = typing.Annotated[int, _Mark("U8", shapes.U8)]
U16 = typing.Annotated[int, _Mark("U16", shapes.U16)]
U32 = typing.Annotated[int, _Mark("U32", shapes.U32)]
U64 = typing.Annotated[int, _Mark("U64", shapes.U64)]
USIZE = typing.Annotated[int, _Mark("USIZE", shapes.USIZE)]
ISIZE = typing.Annotated[int, _Mark("ISIZE", shapes.ISIZE)]
F32 = typing.Annotated[float, _Mark("F32", shapes.F32)]

#: What a slot answering a map may return: a `Map`, or plain data.
MapAnswer = Union[Map, Mapping[str, Any]]
#: What a slot answering a list may return: a `List`, or plain data.
ListAnswer = Union[List, Sequence[Any]]
#: What a slot answering a value may return: a `Ref`, or plain data.
ValueAnswer = Union[Ref, None, bool, int, float, str, bytes, Sequence[Any], Mapping[str, Any]]

_REQUIRED = "__guatiao_required__"
_FALLIBLE = "__guatiao_fallible__"


def required(fn: F) -> F:
    """Marks a slot every table fills. A provider may leave any other slot
    null."""
    setattr(fn, _REQUIRED, True)
    return fn


def fallible(fn: F) -> F:
    """Marks a slot that can fail: its C prototype ends with a
    `guatiao_provider_error *err`, and a Rust method answers `Result`."""
    setattr(fn, _FALLIBLE, True)
    return fn


def kind_of(what: Any) -> Kind:
    """The `Kind` a kind class (or a `Kind`) stands for."""
    if isinstance(what, Kind):
        return what
    found = getattr(what, "kind", None) if isinstance(what, type) else None
    if isinstance(found, Kind):
        return found
    raise TypeError(f"{what!r} is not a kind")


def _is_kind_class(what: Any) -> bool:
    return isinstance(what, type) and isinstance(what.__dict__.get("kind"), Kind)


def _split(annotation: Any) -> "tuple[Any, list[Any]]":
    if typing.get_origin(annotation) is typing.Annotated:
        args = typing.get_args(annotation)
        return args[0], list(args[1:])
    return annotation, []


def _shape(annotation: Any, *, answer: bool, where: str) -> Any:
    base, marks = _split(annotation)
    for mark in marks:
        if isinstance(mark, _Mark) and mark.shape is not None:
            return mark.shape
    lent = any(m is _LENT for m in marks)
    taken = any(m is _TAKEN for m in marks)
    if lent or taken:
        if answer:
            raise TypeError(f"{where}: an answer is neither lent nor taken")
        wrap = shapes.lent if lent else shapes.taken
        if _is_kind_class(base):
            return wrap(kind_of(base))
        if base is Map:
            return wrap(shapes.MAP)
        if base is List:
            return wrap(shapes.LIST)
        if base in (Value, Ref):
            return wrap(shapes.VALUE)
        raise TypeError(f"{where}: {base!r} cannot be lent or taken")
    if answer:
        if base is None or base is type(None):
            return None
        if base is str:
            return shapes.TEXT
        if base in (Map, MapAnswer):
            return shapes.MAP
        if base in (List, ListAnswer):
            return shapes.LIST
        if base in (Value, Ref, ValueAnswer):
            return shapes.VALUE
        if _is_kind_class(base):
            return kind_of(base)
    else:
        if base is str:
            return shapes.STR
        if base is bytes:
            return shapes.BYTES
        if base is bytearray:
            return shapes.BYTES_MUT
        if base is Map:
            return shapes.MAP
        if base in (Ref, Value, Optional[Ref], Optional[Value]):
            return shapes.VALUE
        if _is_kind_class(base):
            raise TypeError(f"{where}: an object is handed over or lent: Taken[{base.__name__}] or Lent[{base.__name__}]")
        if base is List:
            raise TypeError(f"{where}: a list is not borrowed: Lent[List] or Taken[List]")
    if base is bool:
        return shapes.BOOL
    if base is float:
        return shapes.F64
    if base is int:
        raise TypeError(f"{where}: `int` has no width; say it with kinds.I64, kinds.U32 and the rest")
    raise TypeError(f"{where}: {annotation!r} is no shape a slot can carry")


#: Names a kind class keeps for itself, never slots.
_OWN = frozenset({"kind"})


class KindProtocol(Protocol):
    """The base of a kind written as a class: `class Greeter(KindProtocol,
    Protocol, kind="greeter", floor_hash=2032737757)`, a method per slot in
    the order the kind's C header lists them, `floor_hash` as the header
    defines it. Each method's shapes come from its annotations; `@required`
    and `@fallible` say the rest. `Greeter.kind` is the `Kind` built, and
    the class itself is accepted wherever a `Kind` is.

    Listing `Protocol` makes the class a protocol for a type checker, so a
    plain object with the right methods is a `Listener` as far as mypy is
    concerned; a provider class inherits its kinds.
    """

    def __init_subclass__(
        cls,
        *,
        kind: Optional[str] = None,
        floor_hash: Optional[int] = None,
        is_object: bool = False,
        struct: Optional[type] = None,
        **kwargs: Any,
    ) -> None:
        super().__init_subclass__(**kwargs)
        if floor_hash is None and kind is None and not is_object and struct is None:
            return
        methods: list[Method] = []
        stubs: dict[str, Any] = {}
        for attr, fn in cls.__dict__.items():
            if attr.startswith("_") or attr in _OWN or not callable(fn):
                continue
            where = f"{cls.__name__}.{attr}"
            try:
                hints = typing.get_type_hints(fn, include_extras=True)
            except NameError as unknown:
                raise TypeError(f"{where}: {unknown}; a kind class's annotations are read in its module") from None
            params = list(fn.__code__.co_varnames[1 : fn.__code__.co_argcount])
            missing = [p for p in params if p not in hints]
            if missing or "return" not in hints:
                raise TypeError(f"{where}: every argument and the answer need an annotation")
            args = [_shape(hints[p], answer=False, where=f"{where}({p})") for p in params]
            answer = _shape(hints["return"], answer=True, where=f"{where} answer")
            methods.append(
                Method(
                    attr,
                    [a for a in args if a is not None],
                    answer,
                    fallible=bool(getattr(fn, _FALLIBLE, False)),
                    required=bool(getattr(fn, _REQUIRED, False)),
                    arg_names=params,
                    doc=inspect.cleandoc(fn.__doc__) if fn.__doc__ else None,
                )
            )
            stubs[attr] = fn
        name = kind if kind is not None else cls.__name__.lower()
        doc = cls.__dict__.get("__doc__")
        if floor_hash is None:
            # A kind born here: hashed by the library as #[guatiao::kind]
            # hashes the same slots, so another language's declaration agrees.
            from ._description import floor_hash_of, slot_description

            floor_hash = floor_hash_of(is_object, [slot_description(m) for m in methods])
        built = Kind(
            name,
            struct,
            floor_hash=floor_hash,
            methods=methods,
            is_object=is_object,
            doc=inspect.cleandoc(doc) if doc else None,
        )
        built.stubs = stubs
        setattr(cls, "kind", built)
