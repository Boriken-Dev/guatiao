# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A configuration's schema read from its type, and the configuration
built back from plain data. Needs no native library."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Annotated, Any, Dict, List, Literal, Optional, TypedDict

import pytest

from guatiao._typed_schema import build, is_config_type, schema_for


@dataclass(frozen=True)
class Inner:
    port: int


@dataclass(frozen=True)
class Outer:
    host: str
    inner: Inner
    mode: Literal["fast", "slow"] = "fast"
    ratio: float = 0.5
    tags: List[str] = field(default_factory=list)
    env: Dict[str, str] = field(default_factory=dict)
    note: Optional[str] = None
    level: Annotated[int, {"minimum": 0}] = 1


class Settings(TypedDict, total=False):
    verbose: bool
    retries: int


def test_a_dataclass_reads_as_an_object_with_its_required_fields() -> None:
    schema = schema_for(Outer)
    assert schema["type"] == "object"
    assert schema["additionalProperties"] is False
    assert schema["required"] == ["host", "inner"]
    props = schema["properties"]
    assert props["host"] == {"type": "string"}
    assert props["inner"]["properties"] == {"port": {"type": "integer"}}
    assert props["mode"] == {"enum": ["fast", "slow"], "default": "fast"}
    assert props["ratio"] == {"type": "number", "default": 0.5}
    assert props["tags"] == {"type": "array", "items": {"type": "string"}}
    assert props["env"] == {"type": "object", "additionalProperties": {"type": "string"}}
    assert props["note"]["anyOf"] == [{"type": "string"}, {"type": "null"}]
    assert props["level"] == {"type": "integer", "minimum": 0, "default": 1}


def test_a_typeddict_reads_its_required_keys() -> None:
    schema = schema_for(Settings)
    assert schema["properties"] == {"verbose": {"type": "boolean"}, "retries": {"type": "integer"}}
    assert "required" not in schema
    assert is_config_type(Settings) and is_config_type(Outer) and not is_config_type(dict)


def test_plain_data_builds_the_dataclass_back() -> None:
    built = build(Outer, {"host": "h", "inner": {"port": 5900}, "ratio": 1, "tags": ["a"]})
    assert built == Outer(host="h", inner=Inner(5900), ratio=1.0, tags=["a"])
    assert isinstance(built.ratio, float)
    assert build(Settings, {"verbose": True}) == {"verbose": True}


def test_a_type_no_schema_describes_is_named() -> None:
    with pytest.raises(TypeError, match="set"):
        schema_for(set)
    with pytest.raises(TypeError, match="text"):
        schema_for(Dict[int, str])
    assert schema_for(Any) == {}


@dataclass(frozen=True)
class Partly:
    name: str
    note: Optional[str]
    seen: int = field(default=0, init=False)


def test_an_optional_field_without_a_default_may_be_left_out() -> None:
    schema = schema_for(Partly)
    assert schema["required"] == ["name"]
    assert "seen" not in schema["properties"], "a field __init__ does not take is not configured"
    built = build(Partly, {"name": "a"})
    assert built == Partly(name="a", note=None)
    assert build(Partly, {"name": "a", "note": "n"}).note == "n"
