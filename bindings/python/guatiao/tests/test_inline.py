# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`Ref.to_inline_text` against the shared vectors every implementation
reads."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from guatiao import Value, wiping_alloc

_CASES = json.loads(
    (
        Path(__file__).resolve().parents[4]
        / "crates"
        / "guatiao"
        / "tests"
        / "vectors"
        / "inline.json"
    ).read_text(encoding="utf-8")
)["cases"]


def build(tree):
    """The value a vector tree names."""
    tag, *rest = tree
    if tag == "absent":
        return Value.absent()
    if tag == "null":
        return Value.null()
    if tag == "bool":
        return Value.bool(rest[0])
    if tag == "number":
        return Value.number(rest[0])
    if tag == "string":
        return Value.string(rest[0])
    if tag == "bytes":
        return Value.bytes_(bytes.fromhex(rest[0]))
    if tag == "list":
        value = Value.list()
        for item in rest[0]:
            with build(item) as child:
                value.as_list().append(child)
        return value
    value = Value.map()
    for key, item in rest[0]:
        with build(item) as child:
            value.as_map()[key] = child
    return value


def test_the_vectors_are_read():
    assert len(_CASES) >= 20


@pytest.mark.parametrize("case", _CASES, ids=[c["name"] for c in _CASES])
def test_every_vector(case):
    with build(case["tree"]) as value:
        assert value.to_inline_text() == case["text"]


def test_a_child_prints_alone():
    with Value({"outer": {"inner": [1, "two"]}}) as value:
        assert value.as_map()["outer"].to_inline_text() == '{"inner":[1,"two"]}'


def test_a_secret_is_not_shown():
    with Value("hunter2", alloc=wiping_alloc()) as secret:
        assert secret.to_inline_text() == "[sensitive]"
    with Value({"user": "alice"}) as value:
        with Value("hunter2", alloc=wiping_alloc()) as secret:
            value.as_map()["password"] = secret
        shown = value.to_inline_text()
        assert "hunter2" not in shown
        assert shown == '{"user":"alice","password":[sensitive]}'
