# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Reading and writing a `guatiao.Value` as JSON, TOML or YAML, over the
`guatiao_serde` library.

Importing this package touches no native library: the library is found
and loaded on the first `loads` or `dumps`.
"""

from __future__ import annotations

import ctypes
import os
import threading
from pathlib import Path
from typing import Any, Optional, cast

from guatiao import BadValue, Value, ffi

__version__ = "0.0.0a1"

__all__ = [
    "__version__",
    "BYTES_ARRAY",
    "BYTES_BASE64",
    "BYTES_DATA_URI",
    "BYTES_REFUSE",
    "PRETTY",
    "READ_DATA_URIS",
    "ParseError",
    "dumps",
    "library",
    "loads",
    "parse",
    "render",
    "use_library",
]

#: How a byte string is spelled where the format has none -- the low
#: byte of `how`.
BYTES_DATA_URI = 0
BYTES_BASE64 = 1
BYTES_ARRAY = 2
BYTES_REFUSE = 3
#: Turn a `data:;base64,...` string back into bytes when reading.
READ_DATA_URIS = 1 << 8
#: Indent the output for a person to read. JSON only; ignored elsewhere.
PRETTY = 1 << 9


class ParseError(BadValue, ValueError):
    """A document that does not parse, and where it stopped.

    `format` is `"json"`, `"toml"` or `"yaml"`; `line` and `column` count
    from 1 (the column in characters) and are 0 when the parser did not
    say where; `message` says what it expected, without quoting the
    document. For text that is not UTF-8 they name its first bad byte.
    """

    def __init__(self, message: str, *, format: str, line: int, column: int) -> None:
        where = f" at line {line}, column {column}" if line else ""
        super().__init__(int(ffi.Status.ERR_BAD_VALUE), f"{message}{where}")
        self.message = message
        self.format = format
        self.line = line
        self.column = column

    def __reduce__(self) -> "tuple[Any, tuple[Any, ...]]":
        return (_parse_error, (self.message, self.format, self.line, self.column))


def _parse_error(message: str, format: str, line: int, column: int) -> ParseError:
    return ParseError(message, format=format, line=line, column=column)


class _CParseError(ctypes.Structure):
    """`guatiao_parse_error`: where a document stopped parsing. `message`
    is an owned string value."""

    _fields_ = [
        ("format", ctypes.c_uint32),
        ("line", ctypes.c_uint32),
        ("column", ctypes.c_uint32),
        ("message", ffi.Node),
    ]


#: `GUATIAO_FORMAT_*`: which format a parse error names.
_FORMAT_NAMES = {0: "json", 1: "toml", 2: "yaml"}

_P = ctypes.POINTER
_u32 = ctypes.c_uint32
_PARSE = ([ffi.Str, _u32, _P(ffi.Alloc), _P(ffi.Node), _P(_CParseError)], _u32)
_EMIT = ([_P(ffi.Node), _u32, _P(ffi.Alloc), _P(ffi.Node)], _u32)
_TO_WIRE = _PARSE
_FROM_WIRE = ([ffi.Bytes, _u32, _P(ffi.Alloc), _P(ffi.Node)], _u32)

# name -> (argtypes, restype, the feature the library needs for it).
_EXPORTS: "dict[str, tuple[list[Any], Any, str | None]]" = {
    "guatiao_json_parse_ex": (*_PARSE, None),
    "guatiao_json_emit": (*_EMIT, None),
    "guatiao_json_emit_pretty": (*_EMIT, None),
    "guatiao_json_to_wire": (*_TO_WIRE, None),
    "guatiao_json_from_wire": (*_FROM_WIRE, None),
    "guatiao_toml_parse_ex": (*_PARSE, "toml"),
    "guatiao_toml_emit": (*_EMIT, "toml"),
    "guatiao_toml_to_wire": (*_TO_WIRE, "toml"),
    "guatiao_toml_from_wire": (*_FROM_WIRE, "toml"),
    "guatiao_yaml_parse_ex": (*_PARSE, "yaml"),
    "guatiao_yaml_emit": (*_EMIT, "yaml"),
    "guatiao_yaml_to_wire": (*_TO_WIRE, "yaml"),
    "guatiao_yaml_from_wire": (*_FROM_WIRE, "yaml"),
    "guatiao_alloc_default": ([_P(ffi.Alloc)], _u32, None),
}

_lock = threading.Lock()
_library: "ffi.Library | None" = None
_chosen: "Path | None" = None


def use_library(path: "str | os.PathLike[str]") -> None:
    """Loads `guatiao_serde` from `path` -- a file exporting its functions,
    whatever it is called (an application's own library carrying several
    surfaces, say), or a directory holding it under its platform filename
    -- instead of looking for one. `RuntimeError` when it is already
    loaded from elsewhere; naming the one already loaded does nothing."""
    global _chosen
    chosen = Path(path)
    with _lock:
        if _library is not None:
            same = chosen if chosen.is_file() else chosen / ffi.dll_filename("guatiao_serde")
            if same.resolve() != _library.path.resolve():
                raise RuntimeError(f"guatiao_serde is already loaded from {_library.path}")
            return
        _chosen = chosen


def library() -> "ffi.Library":
    """The `guatiao_serde` library, found and loaded on first use:
    `use_library`'s choice, else `GUATIAO_SERDE_LIBRARY`, else
    `GUATIAO_LIBRARY` (a directory holding it, or a file exporting its
    functions or sitting beside it), else the platform's search.
    `guatiao.LibraryNotFound` names every place tried."""
    global _library
    with _lock:
        if _library is None:
            path = ffi.find_library(
                "guatiao_serde",
                variable="GUATIAO_SERDE_LIBRARY",
                symbol="guatiao_json_parse_ex",
                chosen=_chosen,
            )
            _library = ffi.Library(path, _EXPORTS)
        return _library


# format -> (parse export, emit export, dedicated pretty-emit export or
# None, the feature that must be on in the loaded library, or None for JSON).
_FORMATS = {
    "json": ("guatiao_json_parse_ex", "guatiao_json_emit", "guatiao_json_emit_pretty", None),
    "toml": ("guatiao_toml_parse_ex", "guatiao_toml_emit", None, "toml"),
    "yaml": ("guatiao_yaml_parse_ex", "guatiao_yaml_emit", None, "yaml"),
}


def _format(format: str) -> "tuple[str, str, Optional[str], Optional[str]]":
    try:
        return _FORMATS[format]
    except KeyError:
        raise ValueError(f"unknown format: {format!r}") from None


def _resolved(name: str, feature: "str | None") -> "tuple[Any, Any]":
    lib = library()
    if name in lib.symbols_missing():
        raise NotImplementedError(
            f"{name} needs the {feature!r} feature" if feature else name
        )
    return lib, getattr(lib, name)


def loads(
    text: str,
    *,
    format: str = "json",
    how: int = 0,
    alloc: "ffi.Alloc | None" = None,
) -> Value:
    """Parses `text` as `format` into a new `guatiao.Value`, built in
    `alloc` (default: the library's own allocator).

    A document that does not parse raises `ParseError` (a
    `guatiao.BadValue` and a `ValueError`) carrying `format`, `line` and
    `column`."""
    parse_name, _emit, _emit_pretty, feature = _format(format)
    lib, fn = _resolved(parse_name, feature)
    view, _buf = ffi.text(text)
    alloc_struct = alloc if alloc is not None else lib.alloc
    raw = ffi.Node()
    error = _CParseError()
    status = fn(
        view, how, ctypes.byref(alloc_struct), ctypes.byref(raw), ctypes.byref(error)
    )
    _finish_parse(status, error, format)
    return ffi.adopt(raw, alloc_struct)


def _finish_parse(status: int, error: _CParseError, format: str) -> None:
    """Frees the error's message and raises for a failing `status`: a
    `ParseError` for a document that does not parse, any other refusal as
    its own exception with the message the library gave it."""
    try:
        message = ffi.text_of(error.message)
    finally:
        ffi.free(error.message)
    if status == ffi.Status.ERR_BAD_VALUE:
        raise ParseError(
            message,
            format=_FORMAT_NAMES.get(error.format, format),
            line=error.line,
            column=error.column,
        )
    ffi.check(status, message)


def dumps(
    value: "Value | Any",
    *,
    format: str = "json",
    pretty: bool = False,
    how: int = 0,
    alloc: "ffi.Alloc | None" = None,
) -> str:
    """Writes `value` (a `guatiao.Value` or `Ref`) as `format`. A TOML
    document must be a map."""
    _parse, emit_name, emit_pretty_name, feature = _format(format)
    if pretty and emit_pretty_name is not None:
        name = emit_pretty_name
    else:
        name = emit_name
        if pretty:
            how |= PRETTY
    lib, fn = _resolved(name, feature)
    alloc_struct = alloc if alloc is not None else lib.alloc
    out = ffi.Node()
    ffi.check(
        fn(
            ctypes.byref(ffi.node_of(value)),
            how,
            ctypes.byref(alloc_struct),
            ctypes.byref(out),
        )
    )
    result = ffi.adopt(out, alloc_struct)
    try:
        return cast(str, result.to_python())
    finally:
        result.close()


def parse(text: str, *, format: str = "json", how: int = 0, numbers: str = "auto") -> Any:
    """Parses `text` as `format` straight into plain Python objects, as
    `loads(text, format=format, how=how).to_python(numbers)` answers
    them, with no `Value` built: the document crosses once, as wire
    bytes, and is read here. Errors as `loads`."""
    parse_name, _emit, _emit_pretty, feature = _format(format)
    lib, fn = _resolved(parse_name.replace("_parse_ex", "_to_wire"), feature)
    view, _buf = ffi.text(text)
    raw = ffi.Node()
    error = _CParseError()
    status = fn(view, how, ctypes.byref(lib.alloc), ctypes.byref(raw), ctypes.byref(error))
    _finish_parse(status, error, format)
    try:
        tape = raw.payload.bytes
        data = ctypes.string_at(tape.ptr, tape.len) if tape.len else b""
    finally:
        ffi.free(raw)
    return ffi.to_python(data, numbers)


def render(obj: Any, *, format: str = "json", pretty: bool = False, how: int = 0) -> str:
    """Writes the plain Python object `obj` as `format`, as
    `dumps(Value.from_python(obj), ...)` writes it, with no `Value`
    built: it is written as wire bytes here and crosses once. The objects
    `Value.from_python` takes, with its errors; a TOML document must be
    a mapping."""
    _parse, emit_name, _emit_pretty, feature = _format(format)
    lib, fn = _resolved(emit_name.replace("_emit", "_from_wire"), feature)
    if pretty:
        how |= PRETTY
    data = ffi.from_python(obj)
    # Points into `data`, which outlives the call.
    view = ffi.Bytes(ptr=ctypes.cast(ctypes.c_char_p(data), ctypes.c_void_p), len=len(data))
    out = ffi.Node()
    ffi.check(fn(view, how, ctypes.byref(lib.alloc), ctypes.byref(out)))
    try:
        return ffi.text_of(out)
    finally:
        ffi.free(out)
