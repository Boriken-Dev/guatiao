# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A library written in Python: providers, their kind tables and the
descriptor a host reads, built in memory this module keeps alive.

```python
from guatiao import provide

class Hello:
    def greet(self, name):
        return {"greeting": f"hello, {name}"}

library = provide.library("py_greeter", "1.0.0", providers=[
    provide.Provider("hello", Hello, kinds=[GREETER]),
])
reg.register_library(library)      # this process's own registry
```

What a host reads is the descriptor any library answers: a
`guatiao_library_info` listing a `guatiao_provider_info` per provider,
each with a table per kind built over the provider's class (see
`guatiao.kinds`). So the host may be written in any language: a Rust,
C, Dart or Python registry calls a Python provider as it calls any
other.
"""

from __future__ import annotations

import ctypes
import threading
from typing import Any, Callable, Sequence

from . import _abi
from .kinds import _serve
from .kinds._shapes import Scope
from .value import Ref, Value, _make_str

__all__ = ["Provider", "Library", "library"]


def _text(keep: list, text: str) -> "_abi.Str":
    view, buf = _make_str(text.encode("utf-8"))
    keep.append(buf)
    return view


class Provider:
    """One thing a Python library offers.

    `name` is the provider's own name; its id is `{library id}_{name}`
    unless `id` says otherwise. `cls` is the class whose methods serve
    each of `kinds` (`guatiao.kinds.Kind`s), a table per kind built over
    it.

    Without `config` the provider is its one instance, `cls()`, built
    when the library is. With `config`, a JSON Schema (a mapping, or a
    `Value`), it builds an instance per configuration: the host checks a
    configuration against the schema first, then `cls(config)` is called
    with it as plain Python data, and the instance lives until the host
    destroys it. `default=True` offers the one instance as well.

    `available`, when given, is called as `available()` each time a host
    asks and answers `None` for available or a reason it is not.
    """

    def __init__(
        self,
        name: str,
        cls: type,
        *,
        kinds: Sequence[Any],
        config: Any = None,
        default: "bool | None" = None,
        id: "str | None" = None,
        display_name: str = "",
        version: str = "",
        available: "Callable[[], str | None] | None" = None,
        meta: Any = None,
    ) -> None:
        if not isinstance(cls, type):
            raise TypeError("a provider's cls is a class: its tables are built over it")
        for kind in kinds:
            if getattr(kind, "is_object", True):
                raise TypeError(f"{kind!r} is an object kind; a provider serves provider kinds")
        self.name = name
        self.cls = cls
        self.kinds = tuple(kinds)
        self.config = config
        self.default = (config is None) if default is None else bool(default)
        self.id = id
        self.display_name = display_name
        self.version = version
        self.available = available
        self.meta = meta


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

    def __init__(self, id: str, version: str, providers: Sequence[Provider], meta: Any = None) -> None:
        self.id = id
        self.version = version
        self.providers = tuple(providers)
        self._lock = threading.Lock()
        self._instances: set = set()
        self._defaults: list = []
        self._keep: list = []
        self._closed = False
        names = [p.id or f"{id}_{p.name}" for p in self.providers]
        if len(set(names)) != len(names):
            raise ValueError(f"two providers of {id} share an id: {names}")
        self.kinds = tuple(dict.fromkeys(k.name for p in self.providers for k in p.kinds))

        infos = (_abi.ProviderInfo * max(1, len(self.providers)))()
        self._keep.append(infos)
        for at, (provider, provider_id) in enumerate(zip(self.providers, names)):
            self._describe(infos[at], provider, provider_id)

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

        self.entry = _ENTRY_FN(entry)

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

    def _describe(self, info: "_abi.ProviderInfo", provider: Provider, provider_id: str) -> None:
        keep = self._keep
        info.struct_size = ctypes.sizeof(_abi.ProviderInfo)
        names = (_abi.Str * max(1, len(provider.kinds)))()
        for i, kind in enumerate(provider.kinds):
            names[i] = _text(keep, kind.name)
        keep.append(names)
        info.kinds = _abi.Kinds(ptr=ctypes.cast(names, ctypes.POINTER(_abi.Str)), len=len(provider.kinds))
        info.id = _text(keep, provider_id)
        info.display_name = _text(keep, provider.display_name)
        info.version = _text(keep, provider.version)
        info.meta = self._map(provider.meta)

        tables = (_abi.KindTable * max(1, len(provider.kinds)))()
        keep.append(tables)
        for i, kind in enumerate(provider.kinds):
            table = _serve.table_for(kind, provider.cls)
            tables[i] = _abi.KindTable(
                struct_size=ctypes.sizeof(_abi.KindTable),
                vtable_size=ctypes.sizeof(kind.struct),
                kind=_text(keep, kind.name),
                vtable=ctypes.addressof(table),
            )
        info.tables = _abi.KindTables(
            ptr=ctypes.cast(tables, ctypes.POINTER(_abi.KindTable)),
            len=len(provider.kinds),
            stride=ctypes.sizeof(_abi.KindTable),
        )

        if provider.available is not None:
            slot = _abi.AvailableFn(_available(provider.available, keep))
            keep.append(slot)
            info.available = slot

        if provider.config is not None:
            schema = provider.config if isinstance(provider.config, Value) else Value(provider.config)
            keep.append(schema)
            info.config = ctypes.pointer(schema._raw)
            create = _abi.CreateFn(self._create_for(provider))
            destroy = _abi.DestroyFn(self._destroy)
            keep += [create, destroy]
            info.create = create
            info.destroy = destroy

        if provider.default:
            ctx = _serve.hold(provider.cls())
            self._defaults.append(ctx)
            info.ctx = ctx

    # -- the slots ------------------------------------------------------------

    def _create_for(self, provider: Provider) -> Callable:
        def create(ctx, config, out, err) -> int:
            try:
                if out:
                    out[0] = None
                if not config:
                    raise _null("no configuration")
                scope = Scope()
                try:
                    plain = Ref(_ReadOnly(config.contents, scope), ()).to_python()
                finally:
                    scope.close()
                instance = provider.cls(plain)
                handle = _serve.hold(instance)
                with self._lock:
                    self._instances.add(handle)
                out[0] = handle
                return int(_abi.Status.OK)
            except BaseException as exc:  # nothing may cross back into C
                status, message = _serve.status_of(exc)
                _serve.write_err(err, status, message)
                return status

        return create

    def _destroy(self, ctx, instance) -> None:
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


class _ReadOnly:
    """A `Ref` owner over a configuration a call lends: read, never kept."""

    def __init__(self, node, scope: Scope) -> None:
        self._raw = node
        self._scope = scope

    def _check_open(self) -> None:
        self._scope._check_open()

    def _check_writable(self) -> None:
        raise TypeError("a borrowed value is read-only")

    def _ensure_alloc(self):
        raise TypeError("a borrowed value is read-only")

    def _clone_alloc(self):
        from . import _lib

        return _lib.core().alloc


def _null(message: str) -> BaseException:
    from .errors import NullArgument

    return NullArgument(message)


#: `guatiao_library_entry`: `const guatiao_library_info *(*)(const guatiao_host_info *)`.
_ENTRY_FN = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)

# A reason a provider gives for being unavailable must outlive every
# reader, so each distinct one is kept for the process: a provider has a
# handful.
_reasons: "dict[str, Any]" = {}
_reasons_lock = threading.Lock()


def _available(ask: Callable[[], "str | None"], keep: list) -> Callable:
    def available(ctx, reason) -> bool:
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


def library(id: str, version: str, *, providers: Sequence[Provider], meta: Any = None) -> Library:
    """A library of `providers`, its descriptor built now."""
    return Library(id, version, providers, meta)
