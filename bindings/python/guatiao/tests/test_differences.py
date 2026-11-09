# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.differences` against the shared vectors every implementation
reads."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from guatiao import Value, WrongKind, differences

_DOC = json.loads(
    (
        Path(__file__).resolve().parents[4]
        / "crates"
        / "guatiao"
        / "tests"
        / "vectors"
        / "differences.json"
    ).read_text(encoding="utf-8")
)
_CASES = _DOC["cases"]


def test_the_vectors_are_read():
    assert len(_CASES) >= 15
    assert _DOC["placeholder"] == "***"


@pytest.mark.parametrize("case", _CASES, ids=[c["name"] for c in _CASES])
def test_every_vector(case):
    with differences(
        case["schema"], case["value"], case.get("baseline"), _DOC["placeholder"]
    ) as found:
        assert found.to_python() == case["expected"]


def test_values_and_refs_are_accepted():
    case = _CASES[0]
    with Value(case["schema"]) as schema, Value(case["value"]) as value:
        with differences(schema, value) as found:
            assert found.to_python() == case["expected"]


def test_the_placeholder_is_the_callers():
    secret = next(c for c in _CASES if c["name"].startswith("a secret that changed"))
    with differences(secret["schema"], secret["value"], None, "[hidden]") as found:
        texts = [d["value"] for d in found.to_python() if d["key"] == "password"]
        assert texts == ["[hidden]"]
        assert "hunter2" not in repr(found.to_python())


def test_a_schema_that_is_not_a_map_is_refused():
    with pytest.raises(WrongKind):
        differences(1, {})
