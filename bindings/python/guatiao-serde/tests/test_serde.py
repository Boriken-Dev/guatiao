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


_DOCUMENTS = {
    "json": [
        '{"z": 3, "a": [1.5, true, null, "x", {}, []], "n": 1.10, "e": 1e400}',
        '[1, -0, 12345678901234567890123, 2.5e-3]',
        '"text"',
        "17",
        '{"blob": "data:;base64,AAEC", "deep": {"a": {"b": {"c": [[[]]]}}}}',
    ],
    "toml": ['a = 1\nb = "x"\n[t]\nc = [1.5, true]\n[[arr]]\nd = 2\n'],
    "yaml": ["a: 1\nb: [x, 2.5, null]\nc:\n  d: true\n", "- 1\n- two\n"],
}


@pytest.mark.parametrize(
    "format,text", [(f, t) for f, docs in _DOCUMENTS.items() for t in docs]
)
@pytest.mark.parametrize("how", [0, serde.READ_DATA_URIS])
def test_parse_answers_what_loads_then_to_python_answers(format, text, how):
    for numbers in ("auto", "decimal", "str"):
        with serde.loads(text, format=format, how=how) as value:
            expected = value.to_python(numbers=numbers)
        got = serde.parse(text, format=format, how=how, numbers=numbers)
        assert got == expected and repr(got) == repr(expected)


@pytest.mark.parametrize(
    "format,text", [(f, t) for f, docs in _DOCUMENTS.items() for t in docs]
)
@pytest.mark.parametrize("pretty", [False, True])
@pytest.mark.parametrize("how", [serde.BYTES_DATA_URI, serde.BYTES_BASE64, serde.BYTES_ARRAY])
def test_render_writes_what_dumps_of_from_python_writes(format, text, pretty, how):
    # Decimal keeps 1e400, which a float cannot.
    obj = serde.parse(text, format=format, how=serde.READ_DATA_URIS, numbers="decimal")
    with Value.from_python(obj) as value:
        expected = serde.dumps(value, format=format, pretty=pretty, how=how)
    assert serde.render(obj, format=format, pretty=pretty, how=how) == expected


def test_parse_and_render_refuse_as_loads_and_dumps_do():
    from guatiao import WrongKind
    from guatiao_serde import ParseError

    with pytest.raises(ParseError) as caught:
        serde.parse('{"a":\n 1,,}')
    assert (caught.value.format, caught.value.line, caught.value.column) == ("json", 2, 4)
    with pytest.raises(ParseError) as caught:
        serde.parse("a = = 1", format="toml")
    assert caught.value.format == "toml"
    with pytest.raises(WrongKind):
        serde.render([1, 2], format="toml")
    with pytest.raises(ValueError):
        serde.render(float("inf"))
    with pytest.raises(TypeError):
        serde.render({1: "x"})
    with pytest.raises(ValueError):
        serde.parse("1", format="xml")
    with pytest.raises(ValueError):
        serde.parse("1", numbers="float")


def test_bytes_render_as_dumps_spells_them():
    obj = {"b": b"\x00\x01\xff"}
    with Value.from_python(obj) as value:
        for how in (serde.BYTES_DATA_URI, serde.BYTES_BASE64, serde.BYTES_ARRAY):
            assert serde.render(obj, how=how) == serde.dumps(value, how=how)
        with pytest.raises(Exception) as refused:
            serde.dumps(value, how=serde.BYTES_REFUSE)
    with pytest.raises(type(refused.value)):
        serde.render(obj, how=serde.BYTES_REFUSE)