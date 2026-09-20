# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`Registry`: a host's table of loaded libraries and the providers
they offer, over `guatiao_registry_*`."""

from __future__ import annotations

import ctypes
from typing import Any

from . import _abi, _lib
from .errors import check
from .value import Ref, Value, _make_str

#: A `guatiao_library_entry`-shaped callback for `Registry.register_entry`:
#: `const guatiao_library_info *(*)(const guatiao_host_info *)`, both
#: sides erased to `c_void_p` since this binding does not decode either
#: descriptor.
EntryFn = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)


class _Borrowed:
    """A read-only `Ref` owner for memory this binding does not free --
    a provider's config schema, borrowed from the library's own image."""

    def __init__(self, raw: "_abi.Value") -> None:
        self._raw = raw

    def _check_open(self) -> None:
        return None

    def _ensure_alloc(self) -> "_abi.Alloc":
        raise RuntimeError("a borrowed value cannot be mutated")


class Instance:
    """An instance `Registry.create` built. `close()` runs the
    provider's `destroy` (`guatiao_registry_provider_destroy`); `ctx` is
    what every call through that provider's kind tables takes."""

    def __init__(self, registry: "Registry", key: str, ctx: Any) -> None:
        self._registry = registry
        self._key = key
        self.ctx = ctx
        self._closed = False

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

    def _call(self, fn, *args: Any) -> Any:
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

    def libraries(self) -> list:
        return self._call(_lib.core().guatiao_registry_libraries)

    def providers(self, kind: str = "") -> list:
        view, _buf = _make_str(kind.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_providers, view)

    def available(self, kind: str = "") -> list:
        view, _buf = _make_str(kind.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_available, view)

    def why_not(self, kind: str) -> dict:
        view, _buf = _make_str(kind.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_why_not, view)

    def provider(self, key: str) -> dict:
        view, _buf = _make_str(key.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_provider, view)

    def best(self, kind: str) -> dict:
        view, _buf = _make_str(kind.encode("utf-8"))
        return self._call(_lib.core().guatiao_registry_best, view)

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
        return _lib.core().guatiao_registry_priority(self._handle, view)

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
        the key may be loaded again. Anything already taken from it (a
        table from `provider_table`, an `Instance`) keeps working;
        `provider_config` for one of its providers does not, because the
        copy belonged to the registry. `guatiao.errors.NotFound` when
        nothing answers to `key`."""
        self._key_call(_lib.core().guatiao_registry_retire, key)

    def unload(self, key: str) -> None:
        """`retire`, then the library's own say, then unmapping it.

        **The caller states what nothing can check**: when this returns
        the library's code and its allocator are gone, so every value it
        built through its own allocator, every table, `ctx` and
        `Instance` taken from it must already be released.
        `guatiao.errors.WrongKind` when the library refuses or is linked
        into the host rather than mapped -- either way it stays loaded --
        and `guatiao.errors.NotFound` when nothing answers to `key`."""
        self._key_call(_lib.core().guatiao_registry_unload, key)

    def host(self) -> Any:
        """The raw `const guatiao_host_info *` this registry hands a
        library. Opaque here: for driving a `guatiao_library_entry`
        callback directly, which this binding does not decode."""
        self._check_open()
        return _lib.core().guatiao_registry_host(self._handle)
