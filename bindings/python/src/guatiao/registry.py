# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`Registry`: a host's table of loaded libraries and the providers
they offer, over `guatiao_registry_*`."""

from __future__ import annotations

import ctypes
import types
from typing import Any, Callable, Dict, List, Optional, cast

from . import _abi, _lib, kinds
from .errors import NotFound, check
from .value import Ref, Value, _make_str

#: A `guatiao_library_entry`-shaped callback for `Registry.register_entry`:
#: `const guatiao_library_info *(*)(const guatiao_host_info *)`, both
#: sides erased to `c_void_p` since this binding does not decode either
#: descriptor.
EntryFn = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)


class _Borrowed:
    """A read-only `Ref` owner for memory this binding does not free.

    With `keep` -- the owner the memory belongs to -- it holds that owner
    alive and refuses access once it is closed. With `find` it asks the
    owner for the node again on every access, so a tree that was changed
    since is read as it is now, or not at all, never through a stale
    address. With neither it is a view of memory that outlives it, such
    as a provider's config schema in the library's own image."""

    def __init__(
        self,
        raw: Any = None,
        *,
        keep: Any = None,
        find: Optional[Callable[[], Any]] = None,
    ) -> None:
        self._fixed = raw
        self._keep = keep
        self._find = find

    @property
    def _raw(self) -> "_abi.Value":
        if self._find is not None:
            node = self._find()
            if node is None:
                raise LookupError("the borrowed value is gone")
            return cast("_abi.Value", node)
        return cast("_abi.Value", self._fixed)

    def _check_open(self) -> None:
        if self._keep is not None:
            self._keep._check_open()

    def _check_writable(self) -> None:
        raise TypeError("a borrowed value is read-only")

    def _ensure_alloc(self) -> "_abi.Alloc":
        raise TypeError("a borrowed value is read-only")

    def _clone_alloc(self) -> "_abi.Alloc":
        return _lib.core().alloc


class Instance:
    """An instance `Registry.create` built. `close()` runs the
    provider's `destroy` (`guatiao_registry_provider_destroy`); `ctx` is
    what every call through that provider's kind tables takes."""

    def __init__(self, registry: "Registry", key: str, ctx: Any) -> None:
        self._registry = registry
        self._key = key
        self.ctx = ctx
        self._closed = False

    def bind(self, kind: Any) -> Any:
        """This instance as `kind` (a `guatiao.kinds.Kind`): a `Remote`
        whose calls go to this instance. Valid until it is closed."""
        kind = kinds.kind_of(kind)
        table, size, _ctx = self._registry.provider_table(self._key, kind=kind.name)
        if not table:
            raise NotFound(
                int(_abi.Status.ERR_NOT_FOUND), f"`{self._key}` serves no {kind.name} table"
            )
        return kind.bind(table, size, self.ctx)

    def close(self) -> None:
        if self._closed:
            return
        key_view, _buf = _make_str(self._key.encode("utf-8"))
        _lib.core().guatiao_registry_provider_destroy(
            self._registry._handle, key_view, self.ctx
        )
        self._closed = True

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "Instance":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()


class Registry:
    """A host's registry. Not thread-safe, like the handle it wraps."""

    def __init__(self, id: str, version: str, *, alloc: "_abi.Alloc | None" = None) -> None:
        lib = _lib.core()
        self._alloc = alloc if alloc is not None else lib.alloc
        id_view, _id_buf = _make_str(id.encode("utf-8"))
        version_view, _version_buf = _make_str(version.encode("utf-8"))
        handle = lib.guatiao_registry_new(id_view, version_view, ctypes.byref(self._alloc))
        if not handle:
            raise RuntimeError(
                "guatiao_registry_new failed: id/version not UTF-8, or an incomplete allocator"
            )
        self._handle = handle
        self._closed = False
        self._python_libraries: List[Any] = []

    def _check_open(self) -> None:
        if self._closed:
            raise ValueError("this registry is closed")

    def close(self) -> None:
        if getattr(self, "_closed", True):
            return
        try:
            _lib.core().guatiao_registry_free(self._handle)
        finally:
            self._closed = True

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def __enter__(self) -> "Registry":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()

    def _call(self, fn: Callable[..., int], *args: Any) -> Any:
        """Calls one of the `(reg, ..., alloc, out) -> status` answers,
        and hands back its plain Python value, freed."""
        self._check_open()
        raw = _abi.Value()
        check(fn(self._handle, *args, ctypes.byref(self._alloc), ctypes.byref(raw)))
        value = Value(_raw=raw, alloc=self._alloc)
        try:
            return value.to_python()
        finally:
            value.close()

    # ---- loading ---------------------------------------------------

    def load_file(self, path: str) -> Any:
        view, _buf = _make_str(str(path).encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_load_file, view)

    def register_entry(self, name: str, entry: Any) -> Any:
        """`entry`: an `EntryFn`, or any object ctypes accepts where one
        is expected -- typically built with `EntryFn(callback)`."""
        view, _buf = _make_str(name.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_register_entry, view, entry)

    def register_library(self, library: Any) -> Any:
        """Registers a library written in Python under its id, as
        `register_entry` does a C one, and keeps it alive as long as this
        registry: a module (`guatiao.provide.library_of` says what it
        serves) or a `guatiao.provide.Library`."""
        from . import provide

        if isinstance(library, types.ModuleType):
            library = provide.library_of(library)
        self._python_libraries.append(library)
        return self.register_entry(library.id, library.entry)

    def scan_dir(self, directory: str, *, descending: bool = False, rules: "str | None" = None) -> Any:
        dir_view, _dir_buf = _make_str(str(directory).encode("utf-8"))
        lib = _lib.core()
        if rules is None:
            return self._call(lib.guatiao_registry_scan_dir, dir_view, descending)
        rules_view, _rules_buf = _make_str(rules.encode("utf-8"))
        return self._call(lib.guatiao_registry_scan_dir_rules, dir_view, descending, rules_view)

    def scan_path(self, spec: str, *, descending: bool = False, rules: "str | None" = None) -> Any:
        spec_view, _spec_buf = _make_str(spec.encode("utf-8"))
        rules_view, _rules_buf = _make_str((rules or "").encode("utf-8"))
        return self._call(
            _lib.core().guatiao_registry_scan_path, spec_view, descending, rules_view
        )

    # ---- what is loaded ---------------------------------------------

    def libraries(self) -> List[Any]:
        return cast(List[Any], self._call(_lib.core().guatiao_registry_libraries))

    def providers(self, kind: str = "") -> List[Any]:
        view, _buf = _make_str(kind.encode("utf-8"))
        return cast(List[Any], self._call(_lib.core().guatiao_registry_providers, view))

    def available(self, kind: str = "") -> List[Any]:
        view, _buf = _make_str(kind.encode("utf-8"))
        return cast(List[Any], self._call(_lib.core().guatiao_registry_available, view))

    def why_not(self, kind: str) -> Dict[str, Any]:
        view, _buf = _make_str(kind.encode("utf-8"))
        return cast(Dict[str, Any], self._call(_lib.core().guatiao_registry_why_not, view))

    def provider(self, key: str) -> Dict[str, Any]:
        view, _buf = _make_str(key.encode("utf-8"))
        return cast(Dict[str, Any], self._call(_lib.core().guatiao_registry_provider, view))

    def best(self, kind: str) -> Dict[str, Any]:
        view, _buf = _make_str(kind.encode("utf-8"))
        return cast(Dict[str, Any], self._call(_lib.core().guatiao_registry_best, view))

    def provider_available(self, key: str) -> "tuple[bool, str | None]":
        self._check_open()
        view, _buf = _make_str(key.encode("utf-8"))
        reason = _abi.Str()
        ok = _lib.core().guatiao_registry_provider_available(
            self._handle, view, ctypes.byref(reason)
        )
        if ok:
            return True, None
        text = ctypes.string_at(reason.ptr, reason.len).decode("utf-8") if reason.len else ""
        return False, text

    # ---- keys and priority -------------------------------------------

    def keyed_by(self, template: str) -> "Registry":
        self._check_open()
        view, _buf = _make_str(template.encode("utf-8"))
        check(_lib.core().guatiao_registry_keyed_by(self._handle, view))
        return self

    def libraries_keyed_by(self, template: str) -> "Registry":
        self._check_open()
        view, _buf = _make_str(template.encode("utf-8"))
        check(_lib.core().guatiao_registry_libraries_keyed_by(self._handle, view))
        return self

    def set_priority(self, id: str, priority: int) -> None:
        self._check_open()
        view, _buf = _make_str(id.encode("utf-8"))
        check(_lib.core().guatiao_registry_set_priority(self._handle, view, priority))

    def priority(self, id: str) -> int:
        self._check_open()
        view, _buf = _make_str(id.encode("utf-8"))
        return int(_lib.core().guatiao_registry_priority(self._handle, view))

    # ---- a provider's table and configuration ------------------------

    def provider_table(self, key: str, kind: "str | None" = None) -> "tuple[Any, int, Any]":
        """`(table, size, ctx)` for the table `key` speaks `kind` through.
        Without `kind`, the provider's single table. `table` is null when
        there is no such provider or table."""
        self._check_open()
        view, _buf = _make_str(key.encode("utf-8"))
        lib = _lib.core()
        size = ctypes.c_size_t(0)
        if kind is None:
            table = lib.guatiao_registry_provider_vtable(self._handle, view, ctypes.byref(size))
        else:
            kind_view, _kbuf = _make_str(kind.encode("utf-8"))
            table = lib.guatiao_registry_provider_table(
                self._handle, view, kind_view, ctypes.byref(size)
            )
        ctx = lib.guatiao_registry_provider_ctx(self._handle, view)
        return table, size.value, ctx

    def bind(self, key: str, kind: Any) -> Any:
        """The provider filed under `key`, as `kind` (a
        `guatiao.kinds.Kind`): a `Remote` calling its table with its own
        `ctx`, the provider's one instance. `guatiao.NotFound` when it
        serves no table for the kind; `guatiao.kinds.FloorMismatch` when
        the table is not the one `kind` describes. A provider that only
        builds instances is bound through `create(...).bind(kind)`.

        Valid while the library is: until it is unloaded, or, once
        retired, collected."""
        kind = kinds.kind_of(kind)
        table, size, ctx = self.provider_table(key, kind=kind.name)
        if not table:
            raise NotFound(int(_abi.Status.ERR_NOT_FOUND), f"`{key}` serves no {kind.name} table")
        return kind.bind(table, size, ctx)

    def provider_config(self, key: str) -> "Ref | None":
        """A read-only, borrowed view of the provider's configuration
        schema, or `None` when it declares none. Never `.close()` this:
        it is not owned, and lives as long as the library."""
        self._check_open()
        view, _buf = _make_str(key.encode("utf-8"))
        ptr = _lib.core().guatiao_registry_provider_config(self._handle, view)
        if not ptr:
            return None
        return Ref(_Borrowed(ptr.contents), ())

    def create(self, key: str, config: Any) -> Instance:
        """Builds an instance of the provider filed under `key` from
        `config` (a `Value`, or anything `Value.from_python` accepts)."""
        self._check_open()
        key_view, _buf = _make_str(key.encode("utf-8"))
        owns_config = not isinstance(config, Value)
        config_value = config if isinstance(config, Value) else Value.from_python(
            config, alloc=self._alloc
        )
        try:
            out = ctypes.c_void_p()
            err = _abi.ProviderError()
            status = _lib.core().guatiao_registry_provider_create(
                self._handle,
                key_view,
                ctypes.byref(config_value._raw),
                ctypes.byref(out),
                ctypes.byref(err),
            )
            check(status, err)
        finally:
            if owns_config:
                config_value.close()
        return Instance(self, key, out)

    def _key_call(self, fn: Any, key: str) -> None:
        """One `(reg, key) -> status` export."""
        self._check_open()
        view, _buf = _make_str(key.encode("utf-8"))
        check(fn(self._handle, view))

    def retire(self, key: str) -> None:
        """Takes the library `key` names out of this registry and leaves
        it mapped.

        `key` is the LIBRARY key -- `libraries_keyed_by`'s template,
        `%id` by default -- not a provider key. Its providers leave, and
        the key may be loaded again. `provider_config` for one of its
        providers goes with it, because the copy belonged to the registry.
        `guatiao.errors.NotFound` when nothing answers to `key`.

        The library is closed once nothing uses it: by `collect`, or by
        this registry's next load, retire or unload, once no live
        `Instance` or object of its own remains and its own `unload`
        function agrees. A table from `provider_table` is valid until
        then and not after."""
        self._key_call(_lib.core().guatiao_registry_retire, key)

    def collect(self) -> List[str]:
        """Closes every retired library nothing uses any more, and returns
        their library keys, oldest first.

        One whose own `unload` function refuses stays retired and mapped,
        and is asked again next time; one with no such function stays
        mapped. No table taken from a retired library is used again."""
        return cast(List[str], self._call(_lib.core().guatiao_registry_collect))

    def unload(self, key: str, *, unchecked: bool = False) -> None:
        """Unmaps a library, when the library agrees.

        `guatiao.Busy` while a handle into it is held (an answer another
        library pinned through the host's services); then the library's
        own `unload` function is asked, its promise that what it can
        account for is released: `guatiao.Busy` again while an instance or
        object it handed out is alive; `guatiao.Unsupported`
        when it has no such function and so promises nothing;
        `guatiao.WrongKind` when it is linked into the host rather than
        mapped; `guatiao.NotFound` for an unknown key. A refusal leaves it
        loaded.

        `unchecked=True` is the caller insisting on a library with no
        `unload` function: nothing it handed out is alive at all. Either
        way, no table, `ctx` or borrowed schema taken from the library is
        used again."""
        lib = _lib.core()
        call = lib.guatiao_registry_unload_unchecked if unchecked else lib.guatiao_registry_unload
        self._key_call(call, key)

    # ---- sources: libraries that come and go -----------------------

    def add_source_file(self, path: Any, config: Any = None) -> str:
        """Maps a source file (one exporting `guatiao_source`, such as a
        bridge to another language) and adds the source it answers for
        `config` (a mapping or `Value`, the source's own settings). Answers
        the source's name, its path.

        The file stays mapped for the life of the process and is not a
        library: nothing here retires it. Nothing it serves is loaded until
        `refresh()`. A refusal raises the status's `GuatiaoError` with the
        source's words: `NotFound` when the file cannot be mapped,
        `WrongKind` when it is no source, `BadValue` for a table the checks
        refuse or a path already added, the source's own status when it
        refuses `config`."""
        self._check_open()
        name = str(path)
        view, _buf = _make_str(name.encode("utf-8"))
        owned = not isinstance(config, Value)
        value = config if not owned else Value.from_python(
            {} if config is None else config, alloc=self._alloc
        )
        try:
            err = _abi.ProviderError()
            status = _lib.core().guatiao_registry_add_source_file(
                self._handle, view, ctypes.byref(value._raw), ctypes.byref(err)
            )
            check(status, err)
        finally:
            if owned:
                value.close()
        return name

    def watch_dir(self, directory: Any, *, rules: "str | None" = None) -> None:
        """Watches a directory: each `refresh()` loads a library file that
        appeared there (filtered by `rules`, one `[!]KEY=VALUE` per line, as
        `scan_dir` takes them), retires one that vanished, and retires and
        loads again one whose length or modification time changed.
        `BadValue` for a rule that is not one or a directory already
        watched."""
        self._check_open()
        dir_view, _dir_buf = _make_str(str(directory).encode("utf-8"))
        rules_view, _rules_buf = _make_str((rules or "").encode("utf-8"))
        check(_lib.core().guatiao_registry_watch_dir(self._handle, dir_view, rules_view))

    def refresh(self) -> Dict[str, Any]:
        """Asks every source and watched directory what it serves now and
        makes this registry agree. Answers what changed:

        `{"loaded": [{"source", "id", "stamp", "key"}...], "retired": [the
        same...], "skipped": [{"source", "id", "skipped", ...}...],
        "refused": [{"source", "id"?, "why"}...]}`.

        A new candidate loads; a changed one loads and, once it has, its old
        build is retired, while an `Instance` of the old keeps working; a
        vanished one is retired; a library retired by hand stays out until
        its stamp changes. A retired library closes once nothing uses it,
        and its source is told. Refusals are part of the answer."""
        return cast(Dict[str, Any], self._call(_lib.core().guatiao_registry_refresh))

    def host(self) -> Any:
        """The raw `const guatiao_host_info *` this registry hands a
        library. Opaque here: for driving a `guatiao_library_entry`
        callback directly, which this binding does not decode."""
        self._check_open()
        return _lib.core().guatiao_registry_host(self._handle)
