# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A library written in Python: a module of provider classes, and the
descriptor any host reads, built in memory this module keeps alive.

```python
# greeters.py
from dataclasses import dataclass

from guatiao import BadValue, provide
from greeter_kind import Counter, Greeter

__version__ = "1.0.0"
__all__ = ["Hello", "Shouter"]

class Hello(provide.Provider, Greeter, Counter, display_name="Hello, in Python"):
    def greet(self, name: str) -> dict[str, str]:
        if not name:
            raise BadValue("nobody to greet")
        return {"greeting": f"hello, {name}"}
    def count(self) -> int: ...

@dataclass(frozen=True)
class ShouterConfig:
    prefix: str

class Shouter(provide.Provider, Greeter):
    def __init__(self, config: ShouterConfig) -> None:
        self.prefix = config.prefix
    def greet(self, name: str) -> dict[str, str]:
        return {"greeting": f"{self.prefix} {name.upper()}"}
```

`reg.register_library(greeters)` registers the module: its id is its
`__name__`, its version `__version__`, its providers what it exports.
What a host reads is the descriptor any library answers, so the host may
be written in any language.
"""

from __future__ import annotations

import ctypes
import dataclasses
import inspect
import re
import threading
import types
import typing
import weakref
from typing import Any, Callable, ClassVar, Mapping, Optional, Sequence

from . import _abi, _lib
from ._module_meta import meta_of
from ._typed_schema import build, is_config_type, schema_for
from .errors import NullArgument
from .kinds import _serve
from .kinds._protocol import _is_kind_class
from .kinds._shapes import Scope
from .kinds._spec import Kind
from .value import Ref, Value, _make_str

__all__ = ["Provider", "Library", "library", "library_of"]


def _snake(name: str) -> str:
    return re.sub(r"(?<=[a-z0-9])([A-Z])", r"_\1", name).lower()


@dataclasses.dataclass(frozen=True)
class _Spec:
    name: str
    id: Optional[str]
    display_name: str
    version: str
    meta: Any
    kinds: "tuple[Kind, ...]"
    schema: Any
    config_type: Any
    builds_instances: bool
    default: bool


class Provider:
    """The base of a provider class: `class Hello(provide.Provider,
    Greeter, Counter)` serves every kind class it inherits, through the
    methods it defines.

    Class keywords: `named` (default: the class name in snake_case),
    `id` (default `{library id}_{name}`), `display_name`, `version`
    (empty: the library's), `meta` (a map), `default` and
    `config_schema`. A method a kind requires and the class does not
    define is a `TypeError` when the class is created.

    **Configuration.** A class whose `__init__` takes no argument is its
    one instance, built with the library. One whose `__init__` takes a
    configuration builds an instance per configuration the host checked
    first: its schema is read from the annotation (a dataclass or a
    `TypedDict`) and the instance receives the dataclass, or the dict; or
    `config_schema=` says the schema and the instance receives plain data.
    `default=True` also offers `cls()` when the configuration is optional.

    `available()`, a classmethod, answers `None` when the provider can
    run here and a reason when it cannot; it is asked every time a host
    asks.
    """

    __guatiao_provider__: ClassVar[_Spec]

    def __init_subclass__(
        cls,
        *,
        named: Optional[str] = None,
        id: Optional[str] = None,
        display_name: str = "",
        version: str = "",
        meta: Optional[Mapping[str, Any]] = None,
        default: Optional[bool] = None,
        config_schema: Optional[Mapping[str, Any]] = None,
        **kwargs: Any,
    ) -> None:
        super().__init_subclass__(**kwargs)
        kinds: list[Kind] = []
        for base in cls.__mro__:
            if _is_kind_class(base) and base.__dict__["kind"] not in kinds:
                kinds.append(base.__dict__["kind"])
        if not kinds:
            raise TypeError(f"{cls.__name__} inherits no kind class to serve")
        for kind in kinds:
            if kind.is_object:
                raise TypeError(f"{cls.__name__}: {kind.name} is an object kind; a provider serves provider kinds")
            for method in kind.methods:
                if method.required and not kind.implemented(cls, method.name):
                    raise TypeError(
                        f"{cls.__name__} does not implement `{method.name}`, which the {kind.name} kind requires"
                    )
        config_type, takes_config, must = _config_of(cls)
        schema: Any = config_schema
        if schema is None and takes_config:
            if not is_config_type(config_type):
                raise TypeError(
                    f"{cls.__name__}.__init__ takes a configuration: annotate it with a dataclass or a "
                    "TypedDict, or say its schema with config_schema="
                )
            schema = schema_for(config_type)
        if default is None:
            default = not takes_config
        if default and must:
            raise TypeError(f"{cls.__name__} has no instance without a configuration to offer by default")
        cls.__guatiao_provider__ = _Spec(
            name=named if named is not None else _snake(cls.__name__),
            id=id,
            display_name=display_name,
            version=version,
            meta=meta,
            kinds=tuple(kinds),
            schema=schema,
            config_type=config_type if takes_config and is_config_type(config_type) else None,
            builds_instances=takes_config,
            default=default,
        )

    @classmethod
    def available(cls) -> Optional[str]:
        """`None` when this provider can run here, else why not. Asked
        every time a host asks; override it to say."""
        return None


def _config_of(cls: type) -> "tuple[Any, bool, bool]":
    """The configuration type `cls.__init__` takes, whether it takes one,
    and whether it must."""
    for base in cls.__mro__:
        if base is Provider or base is object or _is_kind_class(base) or base.__module__ == "typing":
            continue
        init = base.__dict__.get("__init__")
        # A protocol's own __init__ is typing's stand-in, not a constructor.
        if init is None or getattr(base, "_is_protocol", False) or getattr(init, "__module__", "") == "typing":
            continue
        params = list(inspect.signature(init).parameters.values())[1:]
        if not params:
            return None, False, False
        if len(params) > 1:
            raise TypeError(f"{cls.__name__}.__init__ takes one configuration, not {len(params)} arguments")
        hints = typing.get_type_hints(init, include_extras=True)
        param = params[0]
        return hints.get(param.name), True, param.default is inspect.Parameter.empty
    return None, False, False


def _text(keep: list[Any], text: str) -> Any:
    view, buf = _make_str(text.encode("utf-8"))
    keep.append(buf)
    return view


class _ReadOnly:
    """A `Ref` owner over a configuration a call lends: read, never kept."""

    def __init__(self, node: Any, scope: Scope) -> None:
        self._raw = node
        self._scope = scope

    def _check_open(self) -> None:
        self._scope._check_open()

    def _check_writable(self) -> None:
        raise TypeError("a borrowed value is read-only")

    def _ensure_alloc(self) -> Any:
        raise TypeError("a borrowed value is read-only")

    def _clone_alloc(self) -> Any:
        return _lib.core().alloc


#: `guatiao_library_entry`: `const guatiao_library_info *(*)(const guatiao_host_info *)`.
_ENTRY_FN = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)

# A reason a provider gives for being unavailable must outlive every
# reader, so each distinct one is kept for the process: a provider has a
# handful.
_reasons: "dict[str, Any]" = {}
_reasons_lock = threading.Lock()


class Library:
    """A built library: its descriptor, and everything the descriptor
    points at, alive while this object is.

    `entry` is a `guatiao_library_entry` (`Registry.register_entry` takes
    it); `info_address` the address of the `guatiao_library_info` it
    answers. Keep the object for as long as a host may read the
    descriptor or call a table: through a registry that retired it,
    until that registry collected it. `close()` lets go of the providers'
    own instances once nothing will call them again.
    """

    def __init__(
        self,
        id: str,
        version: str,
        providers: Sequence[type[Provider]],
        meta: Optional[Mapping[str, Any]] = None,
    ) -> None:
        for cls in providers:
            if not (isinstance(cls, type) and issubclass(cls, Provider) and cls is not Provider):
                raise TypeError(f"{cls!r} is not a provide.Provider subclass")
        self.id = id
        self.version = version
        self.providers: tuple[type[Provider], ...] = tuple(providers)
        self._lock = threading.Lock()
        self._instances: set[int] = set()
        self._defaults: list[int] = []
        self._keep: list[Any] = []
        self._closed = False
        specs = [cls.__guatiao_provider__ for cls in self.providers]
        ids = [spec.id or f"{id}_{spec.name}" for spec in specs]
        if len(set(ids)) != len(ids):
            raise ValueError(f"two providers of {id} share an id: {ids}")
        self.kinds: tuple[str, ...] = tuple(dict.fromkeys(k.name for s in specs for k in s.kinds))

        infos = (_abi.ProviderInfo * max(1, len(self.providers)))()
        self._keep.append(infos)
        for at, (cls, provider_id) in enumerate(zip(self.providers, ids)):
            self._describe(infos[at], cls, provider_id)

        info = _abi.LibraryInfo()
        info.struct_size = ctypes.sizeof(_abi.LibraryInfo)
        info.abi_version = _abi.ABI_VERSION
        info.id = _text(self._keep, id)
        info.version = _text(self._keep, version)
        info.providers = _abi.Providers(
            ptr=ctypes.cast(infos, ctypes.POINTER(_abi.ProviderInfo)),
            len=len(self.providers),
            stride=ctypes.sizeof(_abi.ProviderInfo),
        )
        info.meta = self._map(meta)
        unload = _abi.UnloadFn(self._unload)
        self._keep.append(unload)
        info.unload = unload
        self._info = info

        def entry(host: Any) -> int:
            try:
                return ctypes.addressof(self._info) if not self._closed else 0
            except BaseException:  # nothing may cross back into C
                return 0

        self.entry: Any = _ENTRY_FN(entry)

    @property
    def info_address(self) -> int:
        """The address of this library's `guatiao_library_info`."""
        return ctypes.addressof(self._info)

    # -- building the descriptor ----------------------------------------------

    def _map(self, meta: Any) -> Any:
        if meta is None:
            return None
        value = meta if isinstance(meta, Value) else Value(meta)
        if value.tag != _abi.Tag.MAP:
            raise TypeError("meta is a map")
        self._keep.append(value)
        return ctypes.pointer(value._raw.payload.map)

    def _describe(self, info: Any, cls: type[Provider], provider_id: str) -> None:
        spec = cls.__guatiao_provider__
        keep = self._keep
        info.struct_size = ctypes.sizeof(_abi.ProviderInfo)
        names = (_abi.Str * max(1, len(spec.kinds)))()
        for i, kind in enumerate(spec.kinds):
            names[i] = _text(keep, kind.name)
        keep.append(names)
        info.kinds = _abi.Kinds(ptr=ctypes.cast(names, ctypes.POINTER(_abi.Str)), len=len(spec.kinds))
        info.id = _text(keep, provider_id)
        info.display_name = _text(keep, spec.display_name)
        info.version = _text(keep, spec.version)
        info.meta = self._map(spec.meta)

        tables = (_abi.KindTable * max(1, len(spec.kinds)))()
        keep.append(tables)
        for i, kind in enumerate(spec.kinds):
            table = _serve.table_for(kind, cls)
            tables[i] = _abi.KindTable(
                struct_size=ctypes.sizeof(_abi.KindTable),
                vtable_size=ctypes.sizeof(kind.struct),
                kind=_text(keep, kind.name),
                vtable=ctypes.addressof(table),
            )
        info.tables = _abi.KindTables(
            ptr=ctypes.cast(tables, ctypes.POINTER(_abi.KindTable)),
            len=len(spec.kinds),
            stride=ctypes.sizeof(_abi.KindTable),
        )

        # Asked only when the class says something of its own.
        if "available" in {k for base in cls.__mro__[:-1] if base is not Provider for k in base.__dict__}:
            slot = _abi.AvailableFn(_available(cls.available))
            keep.append(slot)
            info.available = slot

        if spec.builds_instances:
            if spec.schema is not None:
                schema = spec.schema if isinstance(spec.schema, Value) else Value(spec.schema)
                keep.append(schema)
                info.config = ctypes.pointer(schema._raw)
            create = _abi.CreateFn(self._create_for(cls))
            destroy = _abi.DestroyFn(self._destroy)
            keep += [create, destroy]
            info.create = create
            info.destroy = destroy

        if spec.default:
            ctx = _serve.hold(cls())
            self._defaults.append(ctx)
            info.ctx = ctx

    # -- the slots ------------------------------------------------------------

    def _create_for(self, cls: type[Provider]) -> Callable[..., int]:
        spec = cls.__guatiao_provider__

        def create(ctx: Any, config: Any, out: Any, err: Any) -> int:
            try:
                if out:
                    out[0] = None
                if not config:
                    raise NullArgument("no configuration")
                scope = Scope()
                try:
                    plain = Ref(_ReadOnly(config.contents, scope), ()).to_python()
                finally:
                    scope.close()
                made = build(spec.config_type, plain) if spec.config_type is not None else plain
                handle = _serve.hold(cls(made))  # type: ignore[call-arg]
                with self._lock:
                    self._instances.add(handle)
                out[0] = handle
                return int(_abi.Status.OK)
            except BaseException as exc:  # nothing may cross back into C
                status, message = _serve.status_of(exc)
                _serve.write_err(err, status, message)
                return status

        return create

    def _destroy(self, ctx: Any, instance: Any) -> None:
        try:
            with self._lock:
                self._instances.discard(int(instance or 0))
            _serve.release(instance)
        except BaseException:  # nothing may cross back into C
            pass

    def _unload(self) -> int:
        with self._lock:
            busy = bool(self._instances)
        return int(_abi.Status.ERR_BUSY if busy else _abi.Status.OK)

    # -- lifetime -------------------------------------------------------------

    def live_instances(self) -> int:
        """How many instances a host built and has not destroyed."""
        with self._lock:
            return len(self._instances)

    def close(self) -> None:
        """Lets go of each provider's one instance, and answers null to
        any host that asks for the descriptor from now on. Call it once
        no host will call this library again; the descriptor itself stays
        readable while this object lives."""
        if self._closed:
            return
        self._closed = True
        for ctx in self._defaults:
            _serve.release(ctx)
        self._defaults.clear()

    def __repr__(self) -> str:
        return f"<guatiao.provide.Library {self.id} {self.version}>"


def _available(ask: Callable[[], Optional[str]]) -> Callable[[Any, Any], bool]:
    def available(ctx: Any, reason: Any) -> bool:
        try:
            why = ask()
        except BaseException as exc:  # nothing may cross back into C
            why = f"{type(exc).__name__}: {exc}"
        if why is None:
            return True
        text = str(why)
        with _reasons_lock:
            buf = _reasons.get(text)
            if buf is None:
                buf = _reasons[text] = ctypes.create_string_buffer(text.encode("utf-8"))
        if reason:
            reason[0] = _abi.Str(ptr=ctypes.cast(buf, ctypes.c_void_p), len=len(buf) - 1)
        return False

    return available


def library(
    id: str,
    version: str,
    *,
    providers: Sequence[type[Provider]],
    meta: Optional[Mapping[str, Any]] = None,
) -> Library:
    """A library of `providers`, its descriptor built now: the primitive
    `library_of` builds a module's with, for a test, a library made at run
    time, or one that declares `meta`."""
    return Library(id, version, providers, meta)


_by_module: "weakref.WeakKeyDictionary[types.ModuleType, Library]" = weakref.WeakKeyDictionary()
_by_module_lock = threading.Lock()


def library_of(module: types.ModuleType) -> Library:
    """The library a module is: its id is the module's `__name__`, its
    version `__version__`, its providers the `Provider` subclasses it
    exports -- those in `__all__`, in its order, or without `__all__` the
    ones it defines, in definition order. Built once per module object.

    Its `meta` merges the installed distribution's metadata (summary,
    licence, authors, URLs, and each licence file as a notice), then the
    module's `__doc__`, `__author__`, `__license__` and `__copyright__`,
    then a `_meta_` mapping of plain data the module may define.

    `TypeError` for `__main__` (import the module by name), a missing or
    non-text `__version__`, or a module exporting no provider."""
    with _by_module_lock:
        built = _by_module.get(module)
        if built is not None:
            return built
        name = module.__name__
        if name == "__main__":
            raise TypeError("a library is a module imported by name; __main__ has no name to be filed under")
        version = getattr(module, "__version__", None)
        if not isinstance(version, str):
            raise TypeError(f"{name}.__version__ is the library's version, and it is {version!r}")
        exported = getattr(module, "__all__", None)
        if exported is not None:
            found = [getattr(module, attr, None) for attr in exported]
        else:
            found = [v for v in vars(module).values() if getattr(v, "__module__", None) == name]
        providers = [
            cls for cls in found if isinstance(cls, type) and issubclass(cls, Provider) and cls is not Provider
        ]
        if not providers:
            raise TypeError(f"{name} exports no provide.Provider subclass")
        built = Library(name, version, providers, meta_of(module) or None)
        _by_module[module] = built
        return built
