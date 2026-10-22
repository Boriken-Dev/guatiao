# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao_serde` against a real `--all-features` build."""

from __future__ import annotations

import pytest

import guatiao_serde as serde
from guatiao import LibraryNotFound, Value

try:
    serde.library()
except LibraryNotFound:
    pytest.skip("no guatiao_serde library built", allow_module_level=True)


def test_json_round_trip_preserves_key_order_and_integers():
    doc = Value.from_python({"z": 3, "a": 1, "list": [1, "two", None, True]})
    try:
        text = serde.dumps(doc)
        parsed = serde.loads(text)
        try:
            m = parsed.as_map()
            assert list(m.keys()) == ["z", "a", "list"]
            assert m["z"].to_python() == 3
            assert m["a"].to_python() == 1
            assert m["list"].to_python() == [1, "two", None, True]
        finally:
            parsed.close()
    finally:
        doc.close()


def test_json_round_trip_preserves_exact_number_text():
    with Value.from_python({"n": Value.number("1.10")}) as doc:
        text = serde.dumps(doc)
        with serde.loads(text) as parsed:
            assert parsed.as_map()["n"].to_python(numbers="str") == "1.10"


def test_pretty_json_is_longer_than_compact():
    with Value.from_python({"a": 1}) as doc:
        compact = serde.dumps(doc)
        pretty = serde.dumps(doc, pretty=True)
        assert len(pretty) > len(compact)


def test_toml_round_trip():
    with Value.from_python({"a": 1, "b": "x"}) as doc:
        text = serde.dumps(doc, format="toml")
        with serde.loads(text, format="toml") as parsed:
            assert parsed.to_python() == {"a": 1, "b": "x"}


def test_yaml_round_trip():
    with Value.from_python({"a": 1, "b": "x"}) as doc:
        text = serde.dumps(doc, format="yaml")
        with serde.loads(text, format="yaml") as parsed:
            assert parsed.to_python() == {"a": 1, "b": "x"}


def test_toml_document_must_be_a_map():
    from guatiao import WrongKind

    with Value.from_python([1, 2]) as doc:
        with pytest.raises(WrongKind):
            serde.dumps(doc, format="toml")


def test_unknown_format_raises():
    with Value.null() as doc:
        with pytest.raises(ValueError):
            serde.dumps(doc, format="xml")

def test_a_malformed_document_carries_its_format_and_position():
    from guatiao import BadValue
    from guatiao_serde import ParseError

    with pytest.raises(ParseError) as caught:
        serde.loads('{"a":\n 1,,}')
    error = caught.value
    assert isinstance(error, ValueError) and isinstance(error, BadValue)
    assert (error.format, error.line, error.column) == ("json", 2, 4)
    assert error.message and "line 2, column 4" in str(error)

    with pytest.raises(ParseError) as caught:
        serde.loads("a = = 1", format="toml")
    assert (caught.value.format, caught.value.line, caught.value.column) == ("toml", 1, 5)

    with pytest.raises(ParseError) as caught:
        serde.loads("a: [1", format="yaml")
    assert (caught.value.format, caught.value.line) == ("yaml", 1)


def test_a_parse_error_leaves_no_block_allocated():
    from guatiao_serde import ParseError
    from serde_counting import Counting

    counting = Counting(serde.library().alloc)
    with pytest.raises(ParseError):
        serde.loads('{"a": [1,, 2]}', alloc=counting.alloc)
    assert counting.live == 0