# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A configuration's JSON Schema, read from its type, and the configuration
built back from the plain data a host checked against it.

A dataclass or a `TypedDict`, whose fields are `str`, `int`, `float`,
`bool`, a `Literal` of them, `Optional[T]`, `list[T]`, `dict[str, T]`, a
nested dataclass or `TypedDict`, and `Annotated[T, {...}]` adding JSON
Schema keywords. A field with a default, an `Optional` one and a
`NotRequired` key are not required; a dataclass refuses keys it does not
declare.
"""

from __future__ import annotations

import dataclasses
import typing
from collections.abc import Mapping, Sequence
from typing import Any, Literal, Union

_NONE = type(None)


def _not_required() -> "tuple[Any, ...]":
    found = []
    for module in ("typing", "typing_extensions"):
        try:
            found.append(getattr(__import__(module), "NotRequired"))
        except (ImportError, AttributeError):
            pass
    return tuple(found)


def is_typeddict(tp: Any) -> bool:
    """Whether `tp` is a `TypedDict` class, on every supported Python."""
    return isinstance(tp, type) and issubclass(tp, dict) and hasattr(tp, "__required_keys__")


def is_config_type(tp: Any) -> bool:
    """Whether a schema can be read from `tp`."""
    return dataclasses.is_dataclass(tp) and isinstance(tp, type) or is_typeddict(tp)


def _hints(tp: Any) -> "dict[str, Any]":
    return typing.get_type_hints(tp, include_extras=True)


def _strip(tp: Any) -> "tuple[Any, dict[str, Any], bool]":
    """`tp` without `Annotated` and `NotRequired`, the keywords an
    `Annotated` adds, and whether it was `NotRequired`."""
    extra: dict[str, Any] = {}
    optional_key = False
    while True:
        origin = typing.get_origin(tp)
        if origin is typing.Annotated:
            args = typing.get_args(tp)
            for meta in args[1:]:
                if isinstance(meta, Mapping):
                    extra.update(meta)
            tp = args[0]
        elif origin is not None and origin in _not_required():
            optional_key = True
            tp = typing.get_args(tp)[0]
        else:
            return tp, extra, optional_key


def _nullable(tp: Any) -> "tuple[Any, bool]":
    if typing.get_origin(tp) is Union:
        args = [a for a in typing.get_args(tp) if a is not _NONE]
        if len(args) < len(typing.get_args(tp)):
            return (args[0] if len(args) == 1 else Union[tuple(args)]), True
    return tp, False


def schema_for(tp: Any) -> "dict[str, Any]":
    """The JSON Schema of a configuration of type `tp`. `TypeError` for a
    type no schema here describes, naming it."""
    tp, extra, _ = _strip(tp)
    inner, nullable = _nullable(tp)
    if nullable:
        schema: dict[str, Any] = {"anyOf": [schema_for(inner), {"type": "null"}]}
        schema.update(extra)
        return schema
    origin = typing.get_origin(tp)
    if tp is bool:
        schema = {"type": "boolean"}
    elif tp is int:
        schema = {"type": "integer"}
    elif tp is float:
        schema = {"type": "number"}
    elif tp is str:
        schema = {"type": "string"}
    elif origin is Literal:
        schema = {"enum": list(typing.get_args(tp))}
    elif origin in (list, Sequence, typing.List):
        (item,) = typing.get_args(tp) or (Any,)
        schema = {"type": "array", "items": schema_for(item)}
    elif origin in (dict, Mapping, typing.Dict):
        key, value = typing.get_args(tp) or (str, Any)
        if key is not str:
            raise TypeError(f"a configuration's map keys are text, not {key!r}")
        schema = {"type": "object", "additionalProperties": schema_for(value)}
    elif tp is Any:
        schema = {}
    elif dataclasses.is_dataclass(tp) and isinstance(tp, type):
        hints = _hints(tp)
        properties: dict[str, Any] = {}
        required: list[str] = []
        for field in dataclasses.fields(tp):
            ftype = hints[field.name]
            properties[field.name] = schema_for(ftype)
            has_default = (
                field.default is not dataclasses.MISSING
                or field.default_factory is not dataclasses.MISSING
            )
            if field.default is not dataclasses.MISSING and _plain(field.default):
                properties[field.name]["default"] = field.default
            if not has_default and not _nullable(_strip(ftype)[0])[1]:
                required.append(field.name)
        schema = {"type": "object", "properties": properties, "additionalProperties": False}
        if required:
            schema["required"] = required
    elif is_typeddict(tp):
        hints = _hints(tp)
        properties = {key: schema_for(value) for key, value in hints.items()}
        keys = getattr(tp, "__required_keys__", frozenset(hints))
        required = [
            key
            for key in hints
            if key in keys and not _strip(hints[key])[2] and not _nullable(_strip(hints[key])[0])[1]
        ]
        schema = {"type": "object", "properties": properties, "additionalProperties": False}
        if required:
            schema["required"] = required
    else:
        raise TypeError(f"no configuration schema describes {tp!r}")
    schema.update(extra)
    return schema


def _plain(value: Any) -> bool:
    return value is None or isinstance(value, (bool, int, float, str))


def build(tp: Any, data: Any) -> Any:
    """The configuration of type `tp` that plain `data` describes: a
    dataclass built field by field, nested ones too; anything else as it
    is. `data` has already passed `schema_for(tp)`."""
    tp, _, _ = _strip(tp)
    inner, nullable = _nullable(tp)
    if nullable:
        return None if data is None else build(inner, data)
    origin = typing.get_origin(tp)
    if dataclasses.is_dataclass(tp) and isinstance(tp, type) and isinstance(data, Mapping):
        hints = _hints(tp)
        kwargs = {
            field.name: build(hints[field.name], data[field.name])
            for field in dataclasses.fields(tp)
            if field.name in data
        }
        return tp(**kwargs)
    if is_typeddict(tp) and isinstance(data, Mapping):
        hints = _hints(tp)
        return {key: build(hints[key], value) if key in hints else value for key, value in data.items()}
    if origin in (list, Sequence, typing.List) and isinstance(data, list):
        (item,) = typing.get_args(tp) or (Any,)
        return [build(item, value) for value in data]
    if origin in (dict, Mapping, typing.Dict) and isinstance(data, Mapping):
        args = typing.get_args(tp)
        value_type = args[1] if len(args) == 2 else Any
        return {key: build(value_type, value) for key, value in data.items()}
    if tp is float and isinstance(data, int) and not isinstance(data, bool):
        return float(data)
    return data
