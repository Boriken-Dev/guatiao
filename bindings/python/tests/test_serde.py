# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao_serde` against a real `--all-features` build."""

from __future__ import annotations

import pytest

from guatiao import _lib

try:
    _lib.serde()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao_serde library built", allow_module_level=True)

from guatiao import serde
from guatiao.value import Value


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


@pytest.mark.xfail(
    reason=(
        "crates/guatiao-serde's `json` feature turns on serde_json's "
        "arbitrary_precision but not raw_value; Numbers::RawText's "
        "sentinel struct (ser.rs RAW_NUMBER) then serialises literally "
        "as {\"$serde_json::private::RawValue\": \"1.10\"} instead of "
        "the bare token 1.10 -- a fractional or 200-digit number's "
        "exact text does not survive guatiao_json_emit as built today. "
        "Reproduced outside this binding: ctypes.CDLL(...).guatiao_json_emit "
        "on {\"b\": 1.5} emits that literal sentinel object. Not fixable "
        "from bindings/python/ -- the feature list is in "
        "crates/guatiao-serde/Cargo.toml."
    ),
    strict=True,
)
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
    from guatiao.errors import WrongKind

    with Value.from_python([1, 2]) as doc:
        with pytest.raises(WrongKind):
            serde.dumps(doc, format="toml")


def test_unknown_format_raises():
    with Value.null() as doc:
        with pytest.raises(ValueError):
            serde.dumps(doc, format="xml")
