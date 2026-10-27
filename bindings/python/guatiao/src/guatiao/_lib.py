# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Finding and loading a guatiao library, and the `guatiao` one itself.

Every export gets a declared `argtypes`/`restype` up front, from a table,
so ctypes passes the width the function actually expects rather than
whatever the platform's default promotion happens to produce. This
declares every real, linkable export of `guatiao.h` -- not the 22
`static inline` helpers alongside them, which
have no symbol in any library and are reimplemented in `value.py`
instead. `find_library` and `Library` serve the packages over the other
guatiao libraries too (`guatiao.ffi`).
"""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import platform
import threading
from pathlib import Path
from typing import Any, Callable

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
c_int64 = ctypes.c_int64
c_uint64 = ctypes.c_uint64
TextError = _abi.TextError
WireEvent = _abi.WireEvent
ProviderError = _abi.ProviderError

_P = ctypes.POINTER


class LibraryNotFound(RuntimeError):
    """No `basename` library was found by any route; `looked` names each
    place tried, in order."""

    def __init__(self, basename: str, looked: "tuple[str, ...]" = ()) -> None:
        places = ", ".join(
            looked or ("the GUATIAO_LIBRARY environment variable (unset)",)
        )
        super().__init__(
            f"could not find {dll_filename(basename)}: checked {places} and "
            f"ctypes.util.find_library({basename!r})"
        )
        self.basename = basename
        self.looked = tuple(looked)


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


#: A library named by `use_library`, consulted before the environment.
_chosen: "Path | None" = None


def use_library(path: "str | os.PathLike[str]") -> None:
    """Loads the guatiao library from `path` -- a file exporting the ABI,
    whatever it is called, or a directory holding `guatiao.dll` /
    `libguatiao.so` / `libguatiao.dylib` -- instead of looking for one.

    For a program that carries the library under its own name, or already
    has it mapped: the next native call loads it from there. Only this
    package's library: `guatiao_serde` and `guatiao_intake` are told with
    their own `use_library`. `RuntimeError` when a library is already
    loaded from elsewhere; naming the one already loaded does nothing."""
    global _chosen
    chosen = Path(path)
    with _lock:
        if _core is not None:
            same = chosen if chosen.is_file() else chosen / dll_filename("guatiao")
            if same.resolve() != _core.path.resolve():
                raise RuntimeError(f"the guatiao library is already loaded from {_core.path}")
            return
        _chosen = chosen


def _exports(path: Path, symbol: str) -> bool:
    """Whether the library file at `path` exports `symbol`."""
    try:
        return hasattr(ctypes.CDLL(str(path)), symbol)
    except OSError:
        return False


def find_library(
    basename: str,
    *,
    variable: str = "GUATIAO_LIBRARY",
    symbol: "str | None" = None,
    chosen: "str | os.PathLike[str] | None" = None,
) -> Path:
    """Where the library `basename` (`"guatiao"`, `"guatiao_serde"`,
    `"guatiao_intake"`) is, trying in order:

    1. `chosen`, what the package was told in code;
    2. the environment variable `variable` (`GUATIAO_SERDE_LIBRARY`, say),
       when it is not `GUATIAO_LIBRARY`;
    3. `GUATIAO_LIBRARY`.

    A file named by 1 or 2 is taken as it is, whatever it is called; a
    directory holds the library under its platform filename. A file
    `GUATIAO_LIBRARY` names is taken when it exports `symbol` (or always,
    when `symbol` is `None`: the core, which every guatiao library
    carries), else its sibling under the platform filename is. Last,
    `ctypes.util.find_library(basename)`. `LibraryNotFound` names every
    place tried."""
    filename = dll_filename(basename)
    looked: list[str] = []

    def named(text: str, where: str, fallback: bool) -> "Path | None":
        path = Path(text)
        if path.is_dir():
            looked.append(f"{where} ({path / filename})")
            candidate = path / filename
            return candidate if candidate.is_file() else None
        looked.append(f"{where} ({path})")
        if not path.is_file():
            return None
        if not fallback or symbol is None or _exports(path, symbol):
            return path
        candidate = path.parent / filename
        return candidate if candidate.is_file() else None

    if chosen is not None:
        found = named(str(chosen), "use_library", False)
        if found is not None:
            return found
    if variable != "GUATIAO_LIBRARY":
        own = os.environ.get(variable)
        if own:
            found = named(own, variable, False)
            if found is not None:
                return found
        else:
            looked.append(f"{variable} (unset)")
    shared = os.environ.get("GUATIAO_LIBRARY")
    if shared:
        found = named(shared, "GUATIAO_LIBRARY", True)
        if found is not None:
            return found
    else:
        looked.append("GUATIAO_LIBRARY (unset)")
    platform_found = ctypes.util.find_library(basename)
    if platform_found:
        return Path(platform_found)
    raise LibraryNotFound(basename, tuple(looked))


def resolve(basename: str) -> Path:
    """Where `basename`'s library is, `use_library`'s choice first."""
    return find_library(basename, chosen=_chosen)


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
    "guatiao_registry_retire": ([c_void_p, Str], c_uint32, "load"),
    "guatiao_registry_unload": ([c_void_p, Str], c_uint32, "load"),
    "guatiao_registry_unload_unchecked": ([c_void_p, Str], c_uint32, "load"),
    "guatiao_registry_collect": ([c_void_p, _P(Alloc), _P(Value)], c_uint32, "load"),
    "guatiao_registry_add_source": (
        [c_void_p, Str, c_void_p, _P(ProviderError)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_add_source_file": (
        [c_void_p, Str, _P(Value), _P(ProviderError)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_watch_dir": ([c_void_p, Str, Str], c_uint32, "load"),
    "guatiao_registry_refresh": ([c_void_p, _P(Alloc), _P(Value)], c_uint32, "load"),
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
    "guatiao_registry_provider_table": (
        [c_void_p, Str, Str, _P(c_size_t)],
        c_void_p,
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
    "guatiao_registry_provider_meta": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
        "load",
    ),
    "guatiao_registry_kind_descriptions": (
        [c_void_p, Str, _P(Alloc), _P(Value)],
        c_uint32,
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
    "guatiao_schema_redact": (
        [_P(Value), _P(Value), Str, _P(Alloc), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_date_add_span": (
        [_P(_abi.DateFields), _P(_abi.SpanFields), _P(_abi.DateFields)],
        c_uint32,
        None,
    ),
    "guatiao_datetime_add_span": (
        [_P(_abi.DateTimeFields), _P(_abi.SpanFields), _P(_abi.DateTimeFields)],
        c_uint32,
        None,
    ),
    "guatiao_duration_read": (
        [_P(Value), _P(c_uint64), _P(c_uint32)],
        c_uint32,
        None,
    ),
    "guatiao_value_free": ([_P(Value)], c_uint32, None),
    "guatiao_value_clone": ([_P(Alloc), _P(Value), _P(Value)], c_uint32, None),
    "guatiao_value_make_local": ([_P(Value), _P(Alloc)], c_uint32, None),
    "guatiao_map_settle_lent": ([_P(Value), _P(Alloc)], c_uint32, None),
    "guatiao_kind_floor_hash": ([Str, _P(Alloc), _P(c_uint32), _P(Value)], c_uint32, None),
    "guatiao_value_null": ([_P(Value)], c_uint32, None),
    "guatiao_value_absent": ([_P(Value)], c_uint32, None),
    "guatiao_value_bool": ([c_bool, _P(Value)], c_uint32, None),
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
    "guatiao_wire_encode": ([_P(Alloc), _P(Value), _P(Value)], c_uint32, None),
    "guatiao_wire_decode": ([_P(Alloc), Bytes, _P(Value)], c_uint32, None),
    "guatiao_wire_announce": (
        [_P(Alloc), c_uint64, _P(Value), _P(Value)],
        c_uint32,
        None,
    ),
    "guatiao_wire_send": ([_P(Alloc), c_uint64, _P(Value), _P(Value)], c_uint32, None),
    "guatiao_wire_close": ([_P(Alloc), c_uint64, _P(Value)], c_uint32, None),
    "guatiao_wire_receiver_new": ([_P(Alloc)], c_void_p, None),
    "guatiao_wire_receiver_accept": ([c_void_p, Bytes, _P(WireEvent)], c_uint32, None),
    "guatiao_wire_receiver_free": ([c_void_p], None, None),
}

# The three text functions of each date, time and span form.
for _text_form, _fields in (
    ("date", _abi.DateFields),
    ("time", _abi.TimeFields),
    ("datetime", _abi.DateTimeFields),
    ("timestamp", _abi.TimestampFields),
    ("span", _abi.SpanFields),
):
    _CORE_EXPORTS[f"guatiao_{_text_form}_parse"] = (
        [_P(Alloc), Str, _P(_fields), _P(TextError)],
        c_uint32,
        None,
    )
    _CORE_EXPORTS[f"guatiao_{_text_form}_format"] = (
        [_P(Alloc), _P(_fields), _P(Value)],
        c_uint32,
        None,
    )
    _CORE_EXPORTS[f"guatiao_{_text_form}_normalize"] = (
        [_P(Alloc), Str, _P(Value), _P(TextError)],
        c_uint32,
        None,
    )
del _text_form, _fields

class Library:
    """One loaded library: its exports, declared, and its default allocator.

    `exports` maps each export's name to `(argtypes, restype, feature)`:
    `feature` is `None` for a symbol every build carries, else the cargo
    feature that must be on for it, named by the `MissingSymbol` a call
    raises when the library lacks it. Declare `guatiao_alloc_default` to
    use `alloc`."""

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

    def _missing_stub(self, name: str, feature: str | None) -> Callable[..., Any]:
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


def core() -> Library:
    """The `guatiao` library, loaded and declared on first use."""
    global _core
    with _lock:
        if _core is None:
            _core = Library(resolve("guatiao"), _CORE_EXPORTS)
        return _core
