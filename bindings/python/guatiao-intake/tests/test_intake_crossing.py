# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A tree crosses between guatiao_intake and guatiao: built by one library,
read and freed through the other."""

from __future__ import annotations

import pytest

import guatiao_intake as intake
from guatiao import LibraryNotFound, Value

try:
    intake.library()
except LibraryNotFound:
    pytest.skip("no guatiao_intake library built", allow_module_level=True)

from intake_counting import Counting

SCHEMA = {
    "type": "object",
    "properties": {
        "auth": {
            "oneOf": [
                {"properties": {"kind": {"const": "token"}, "token": {"type": "string"}}},
            ],
            "x-tag": "kind",
        },
        "name": {"type": "string", "x-section": "who"},
    },
}


def test_a_tree_intake_built_is_freed_through_the_core():
    counting = Counting(intake.library().alloc)
    with Value.from_python({"type": "object", "properties": {"name": {"type": "string", "x-section": "who"}}}) as schema:
        form = intake.for_schema(schema, alloc=counting.alloc)
        assert counting.live > 0
        form.as_map()["note"] = "added through the core"
        assert form.as_map()["sections"].to_python() == [{"id": "who"}]
        form.close()
    assert counting.live == 0


def test_a_tree_the_core_built_is_read_by_intake_and_freed_by_the_core():
    counting = Counting(intake.library().alloc)
    value = Value.from_python({"agent": [{"name": "a"}, {"name": "b"}]}, alloc=counting.alloc)
    found = intake.at(value, "agent[1].name")
    assert found is not None and found.to_python() == "b"
    value.close()
    with pytest.raises(ValueError):
        found.to_python()
    assert counting.live == 0


def test_a_store_intake_unflattened_is_freed_by_the_core():
    with Value.from_python({"type": "object", "properties": {"n": {"type": "integer"}}}) as schema:
        for _ in range(3):
            back = intake.unflatten(schema, "n", {"n": "5"})
            assert back is not None
            assert back.to_python() == "5"
            back.close()
