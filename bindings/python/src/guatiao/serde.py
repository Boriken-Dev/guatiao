# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Reading and writing a `Value` as JSON, TOML or YAML, over
`guatiao_serde`. Resolved lazily: importing this module touches no
library, only calling `loads`/`dumps` does."""

from __future__ import annotations

import ctypes
from typing import Any, Optional, cast

from . import _abi, _lib
from .errors import ParseError, check, raise_for_status
from .value import Value, _make_str, _string_view

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
    lib = _lib.serde()
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
    alloc: "_abi.Alloc | None" = None,
) -> Value:
    """Parses `text` as `format` into a new `Value`.

    A document that does not parse raises `guatiao.errors.ParseError`
    (a `BadValue` and a `ValueError`) carrying `format`, `line` and
    `column`."""
    parse_name, _emit, _emit_pretty, feature = _format(format)
    lib, fn = _resolved(parse_name, feature)
    view, _buf = _make_str(text.encode("utf-8"))
    alloc_struct = alloc if alloc is not None else lib.alloc
    raw = _abi.Value()
    error = _abi.ParseError()
    status = fn(
        view, how, ctypes.byref(alloc_struct), ctypes.byref(raw), ctypes.byref(error)
    )
    _finish_parse(status, error, format)
    return Value(_raw=raw, alloc=alloc_struct)


def _finish_parse(status: int, error: "_abi.ParseError", format: str) -> None:
    """Frees the error's message and raises for a failing `status`: a
    `ParseError` for a document that does not parse, any other refusal as
    its own exception with the message the library gave it."""
    message = _take_message(error)
    if status == _abi.Status.ERR_BAD_VALUE:
        raise ParseError(
            message,
            format=_abi.FORMAT_NAMES.get(error.format, format),
            line=error.line,
            column=error.column,
        )
    raise_for_status(status, message)


def _take_message(error: "_abi.ParseError") -> str:
    """The parse error's message as text; its string is freed here."""
    try:
        if error.message.tag != _abi.Tag.STRING:
            return ""
        return _string_view(error.message.payload.text).decode("utf-8", "replace")
    finally:
        _lib.core().guatiao_value_free(ctypes.byref(error.message))


def dumps(
    value: Value,
    *,
    format: str = "json",
    pretty: bool = False,
    how: int = 0,
    alloc: "_abi.Alloc | None" = None,
) -> str:
    """Writes `value` as `format`. A TOML document must be a map."""
    _parse, emit_name, emit_pretty_name, feature = _format(format)
    if pretty and emit_pretty_name is not None:
        name = emit_pretty_name
    else:
        name = emit_name
        if pretty:
            how |= PRETTY
    lib, fn = _resolved(name, feature)
    alloc_struct = alloc if alloc is not None else lib.alloc
    out = _abi.Value()
    check(fn(ctypes.byref(value._raw), how, ctypes.byref(alloc_struct), ctypes.byref(out)))
    result = Value(_raw=out, alloc=alloc_struct)
    try:
        return cast(str, result.to_python())
    finally:
        result.close()
