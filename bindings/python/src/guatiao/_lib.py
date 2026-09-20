# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Finding and loading `guatiao`, `guatiao_serde` and `guatiao_form`.

Every export gets a declared `argtypes`/`restype` up front, from a table,
so ctypes passes the width the function actually expects rather than
whatever the platform's default promotion happens to produce. This
declares the 50 real, linkable exports of `guatiao.h` -- not the 22
`static inline` helpers alongside them, which have no symbol in any
library and are reimplemented in `value.py` instead -- plus the 7 of
`guatiao_serde.h` and the 3 of `guatiao_form.h`.
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import platform
import threading
from pathlib import Path
from typing import Any

from . import _abi

c_void_p = ctypes.c_void_p
c_size_t = ctypes.c_size_t
c_uint32 = ctypes.c_uint32
c_int32 = ctypes.c_int32
c_uint8 = ctypes.c_uint8
c_bool = ctypes.c_bool
Str = _abi.Str
Bytes = _abi.Bytes
Value = _abi.Value
Alloc = _abi.Alloc
ProviderError = _abi.ProviderError

_P = ctypes.POINTER


class LibraryNotFound(RuntimeError):
    """No `basename` library was found by any of the three routes."""

    def __init__(self, basename: str) -> None:
        super().__init__(
            f"could not find {dll_filename(basename)}: checked the "
            "GUATIAO_LIBRARY environment variable, "
            f"ctypes.util.find_library({basename!r}), and this package's "
            "own _native/ directory (empty until a build step populates it)"
        )
        self.basename = basename


class MissingSymbol(RuntimeError):
    """`name` is not exported by the library this build linked.

    The library was found and loaded; it was simply built without the
    feature that symbol belongs to.
    """

    def __init__(self, path: Path, name: str, feature: str | None) -> None:
        where = f" (feature {feature!r})" if feature else ""
        super().__init__(f"{path} was not built with {name}{where}")
        self.path = path
        self.name = name
        self.feature = feature


def dll_filename(basename: str) -> str:
    """The platform's filename for a library of this base name."""
    system = platform.system()
    if system == "Windows":
        return f"{basename}.dll"
    if system == "Darwin":
        return f"lib{basename}.dylib"
    return f"lib{basename}.so"


def _native_dir() -> Path:
    return Path(__file__).resolve().parent / "_native"


def resolve(basename: str) -> Path:
    """Where `basename`'s library is, trying each route in order (Q2)."""
    filename = dll_filename(basename)
    env = os.environ.get("GUATIAO_LIBRARY")
    if env:
        env_path = Path(env)
        if env_path.is_dir():
            candidate = env_path / filename
            if candidate.is_file():
                return candidate
        elif env_path.is_file():
            # For the core library itself, a file GUATIAO_LIBRARY names is
            # taken as-is, whatever it is called; a sibling is looked for
            # beside it, under its own platform filename.
            if basename == "guatiao":
                return env_path
            candidate = env_path.parent / filename
            if candidate.is_file():
                return candidate
    found = ctypes.util.find_library(basename)
    if found:
        return Path(found)
    candidate = _native_dir() / filename
    if candidate.is_file():
        return candidate
    raise LibraryNotFound(basename)


# name -> (argtypes, restype, feature). `feature` is None for a symbol
# every build carries; otherwise the cargo feature that must be on for it
# to exist, named in the error a call raises when it is missing.
_CORE_EXPORTS: dict[str, tuple[list[Any], Any, str | None]] = {
    "guatiao_registry_new": ([Str, Str, _P(Alloc)], c_void_p, "load"),
    "guatiao_registry_provider_create": (
        [c_void_p, Str, _P(Value), _P(c_void_p), _P(ProviderError)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_provider_destroy": (
        [c_void_p, Str, c_void_p],
        None,
        "load",
    ),
    "guatiao_registry_host": ([c_void_p], c_void_p, "load"),
    "guatiao_registry_free": ([c_void_p], None, "load"),
    "guatiao_registry_keyed_by": ([c_void_p, Str], c_uint32, "load"),
    "guatiao_registry_libraries_keyed_by": ([c_void_p, Str], c_uint32, "load"),
    "guatiao_registry_load_file": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_register_entry": (
        [c_void_p, Str, c_void_p, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_scan_dir": (
        [c_void_p, Str, c_bool, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_scan_dir_rules": (
        [c_void_p, Str, c_bool, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_scan_path": (
        [c_void_p, Str, c_bool, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_libraries": (
        [c_void_p, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_providers": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_provider": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_available": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_why_not": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_provider_available": (
        [c_void_p, Str, _P(Str)],
        c_bool,
        "load",
    ),
    "guatiao_registry_set_priority": (
        [c_void_p, Str, c_int32],
        c_uint32,
        "load",
    ),
    "guatiao_registry_priority": ([c_void_p, Str], c_int32, "load"),
    "guatiao_registry_best": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_provider_vtable": (
        [c_void_p, Str, _P(c_size_t)],
        c_void_p,
        "load",
    ),
    "guatiao_registry_provider_ctx": ([c_void_p, Str], c_void_p, "load"),
    "guatiao_registry_provider_config": (
        [c_void_p, Str],
        _P(Value),
        "load",
    ),
    "guatiao_merge": (
        [
            c_uint32,
            _P(Value),
            _P(Value),
            _P(Alloc),
            c_uint32,
            c_void_p,
            c_size_t,
            _P(Value),
            _P(Value),
        ],
        c_uint32,
        None,
    ),
    "guatiao_schema_validate": (
        [_P(Value), _P(Value), _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_schema_resolve": ([_P(Value), Str], _P(Value), None),
    "guatiao_schema_flat_keys": (
        [_P(Value), Str, _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_schema_flatten": (
        [_P(Value), Str, _P(Value), _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_schema_unflatten": (
        [_P(Value), Str, _P(Value), _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_value_free": ([_P(Value)], c_uint32, None),
    "guatiao_value_clone": ([_P(Alloc), _P(Value), _P(Value)], c_uint32, None),
    "guatiao_value_null": ([_P(Value)], c_uint32, None),
    "guatiao_value_absent": ([_P(Value)], c_uint32, None),
    "guatiao_value_bool": ([c_uint8, _P(Value)], c_uint32, None),
    "guatiao_value_map": ([_P(Alloc), _P(Value)], c_uint32, None),
    "guatiao_value_list": ([_P(Alloc), _P(Value)], c_uint32, None),
    "guatiao_value_string": ([_P(Alloc), Str, _P(Value)], c_uint32, None),
    "guatiao_value_number": ([_P(Alloc), Str, _P(Value)], c_uint32, None),
    "guatiao_value_bytes": ([_P(Alloc), Bytes, _P(Value)], c_uint32, None),
    "guatiao_map_set": (
        [_P(Alloc), _P(Value), Str, _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_map_discard": ([_P(Value), Str], c_uint32, None),
    "guatiao_map_clear": ([_P(Value)], c_uint32, None),
    "guatiao_map_copy_from": (
        [_P(Alloc), _P(Value), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_list_push": ([_P(Alloc), _P(Value), _P(Value)], c_uint32, None),
    "guatiao_list_discard": ([_P(Value), c_size_t], c_uint32, None),
    "guatiao_list_clear": ([_P(Value)], c_uint32, None),
    "guatiao_string_push": ([_P(Alloc), _P(Value), Str], c_uint32, None),
    "guatiao_buffer_push": ([_P(Alloc), _P(Value), Bytes], c_uint32, None),
    "guatiao_alloc_default": ([_P(Alloc)], c_uint32, None),
}

_SERDE_EXPORTS: dict[str, tuple[list[Any], Any, str | None]] = {
    "guatiao_json_parse": ([Str, c_uint32, _P(Alloc), _P(Value)], c_uint32, None),
    "guatiao_json_emit": (
        [_P(Value), c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_json_emit_pretty": (
        [_P(Value), c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_toml_parse": (
        [Str, c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        "toml",
    ),
    "guatiao_toml_emit": (
        [_P(Value), c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        "toml",
    ),
    "guatiao_yaml_parse": (
        [Str, c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        "yaml",
    ),
    "guatiao_yaml_emit": (
        [_P(Value), c_uint32, _P(Alloc), _P(Value)],
        c_uint32,
        "yaml",
    ),
}

_FORM_EXPORTS: dict[str, tuple[list[Any], Any, str | None]] = {
    "guatiao_form_check": (
        [_P(Value), _P(Value), _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_form_layout": (
        [_P(Value), _P(Value), _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_form_is_visible": (
        [_P(Value), _P(Value), Str, _P(Value), _P(c_bool)],
        c_uint32,
        None,
    ),
}


class Library:
    """One loaded DLL: its exports, declared, and its default allocator."""

    def __init__(
        self, path: Path, exports: dict[str, tuple[list[Any], Any, str | None]]
    ) -> None:
        self.path = path
        self._cdll = ctypes.CDLL(str(path))
        self._funcs: dict[str, Any] = {}
        self._missing: set[str] = set()
        for name, (argtypes, restype, feature) in exports.items():
            try:
                fn = getattr(self._cdll, name)
            except AttributeError:
                self._missing.add(name)
                self._funcs[name] = self._missing_stub(name, feature)
                continue
            fn.argtypes = argtypes
            fn.restype = restype
            self._funcs[name] = fn
        self._alloc: Alloc | None = None

    def _missing_stub(self, name: str, feature: str | None):
        path = self.path

        def _stub(*_args: Any, **_kwargs: Any) -> Any:
            raise MissingSymbol(path, name, feature)

        return _stub

    def __getattr__(self, name: str) -> Any:
        try:
            return self._funcs[name]
        except KeyError:
            raise AttributeError(name) from None

    def symbols_missing(self) -> frozenset[str]:
        return frozenset(self._missing)

    @property
    def alloc(self) -> Alloc:
        """The default allocator, filled by `guatiao_alloc_default` once."""
        if self._alloc is None:
            allocator = Alloc()
            status = self.guatiao_alloc_default(ctypes.byref(allocator))
            if status != 0:
                raise RuntimeError(
                    f"guatiao_alloc_default failed with status {status}"
                )
            self._alloc = allocator
        return self._alloc


_lock = threading.Lock()
_core: Library | None = None
_serde: Library | None = None
_form: Library | None = None


def core() -> Library:
    """The `guatiao` library, loaded and declared on first use."""
    global _core
    with _lock:
        if _core is None:
            _core = Library(resolve("guatiao"), _CORE_EXPORTS)
        return _core


def serde() -> Library:
    """The `guatiao_serde` library, loaded and declared on first use."""
    global _serde
    with _lock:
        if _serde is None:
            _serde = Library(resolve("guatiao_serde"), _SERDE_EXPORTS)
        return _serde


def form() -> Library:
    """The `guatiao_form` library, loaded and declared on first use."""
    global _form
    with _lock:
        if _form is None:
            _form = Library(resolve("guatiao_form"), _FORM_EXPORTS)
        return _form
