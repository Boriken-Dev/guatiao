# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Round trips every value kind against a real build of `guatiao`."""

from __future__ import annotations

import decimal

import pytest

pytest.importorskip("guatiao._lib")
from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

from guatiao import ABSENT, Value


def test_round_trips_every_kind():
    assert Value.null().to_python() is None
    assert Value.bool(True).to_python() is True
    assert Value.bool(False).to_python() is False
    assert Value.string("hola").to_python() == "hola"
    assert Value.bytes_(b"\x00\x01\xff").to_python() == b"\x00\x01\xff"
    assert Value.absent().to_python() is ABSENT

    nested = Value.from_python({"a": [1, 2, {"b": "c"}], "d": None})
    assert nested.to_python() == {"a": [1, 2, {"b": "c"}], "d": None}


def test_exact_number_text_survives():
    v = Value.number("1.10")
    assert v.to_python(numbers="str") == "1.10"
    assert v.to_python(numbers="decimal") == decimal.Decimal("1.10")


def test_large_integers_survive_as_int():
    big = 2**64 - 1
    assert Value.from_python(big).to_python() == big

    digits = int("7" * 200)
    assert Value.from_python(digits).to_python() == digits


def test_nan_and_infinity_raise():
    with pytest.raises(ValueError):
        Value.from_python(float("nan"))
    with pytest.raises(ValueError):
        Value.from_python(float("inf"))
    with pytest.raises(ValueError):
        Value.from_python(decimal.Decimal("nan"))


def test_ref_survives_growth_past_reallocation():
    root = Value.map()
    m = root.as_map()
    m["a"] = "first"
    ref = m["a"]
    for i in range(64):
        m[f"k{i}"] = i
    assert ref.to_python() == "first"
    assert len(m) == 65


def test_closed_value_raises_on_use():
    v = Value.string("x")
    v.close()
    with pytest.raises(ValueError):
        v.to_python()
    v.close()  # idempotent


def test_context_manager_closes():
    with Value.string("x") as v:
        assert v.to_python() == "x"
    with pytest.raises(ValueError):
        v.to_python()


def test_map_protocol():
    root = Value.map()
    m = root.as_map()
    m["one"] = 1
    m["two"] = 2
    assert set(m) == {"one", "two"}
    assert m["one"].to_python() == 1
    assert "one" in m
    del m["one"]
    assert "one" not in m
    assert [k for k, _ in m.items()] == ["two"]
    m.clear()
    assert len(m) == 0


def test_map_copy_from():
    src = Value.from_python({"a": 1, "b": 2})
    dst = Value.map()
    dst.as_map().copy_from(src)
    assert dst.to_python() == {"a": 1, "b": 2}


def test_list_protocol():
    root = Value.list()
    lst = root.as_list()
    lst.append(1)
    lst.append(2)
    lst.append(3)
    assert len(lst) == 3
    assert [r.to_python() for r in lst] == [1, 2, 3]
    del lst[1]
    assert [r.to_python() for r in lst] == [1, 3]
    lst.insert(1, "mid")
    assert [r.to_python() for r in lst] == [1, "mid", 3]
    lst[0] = "first"
    assert [r.to_python() for r in lst] == ["first", "mid", 3]
    lst.clear()
    assert len(lst) == 0


def test_readers_with_fallback():
    from guatiao.value import Ref

    root = Value.from_python({"n": 5, "s": "hi", "b": True})
    m = root.as_map()
    assert m["n"].int_or(-1) == 5
    assert m["s"].int_or(-1) == -1
    assert m["s"].str_or("") == "hi"
    assert m["b"].bool_or(False) is True
    missing = Ref(root, ("missing",))
    assert missing.is_absent()
    assert missing.int_or(-1) == -1


def test_clone_is_independent():
    src = Value.from_python([1, 2, 3])
    clone = src.clone()
    clone.as_list().append(4)
    assert src.to_python() == [1, 2, 3]
    assert clone.to_python() == [1, 2, 3, 4]


def test_value_constructs_like_the_builtins():
    from guatiao import Value

    with Value({"host": "h", "tags": ["a"]}) as v:
        assert v.to_python() == {"host": "h", "tags": ["a"]}
    with Value(host="h", port=5900) as v:
        assert v.to_python() == {"host": "h", "port": 5900}
        assert list(v.as_map()) == ["host", "port"]
    with Value({"host": "h"}, port=1) as v:
        assert v.to_python() == {"host": "h", "port": 1}
    for plain in (1, True, False, 1.5, "x", b"\x00", None, [1, 2]):
        with Value(plain) as v:
            assert v.to_python() == plain and type(v.to_python()) is type(plain)
    with Value() as v:
        assert v.is_absent()
    with pytest.raises(TypeError):
        Value([1], port=1)


def test_a_nested_value_is_copied_in():
    from guatiao import Value

    inner = Value(a="b")
    with Value(host="h", test=inner, more=[Value(1)]) as v:
        inner.as_map()["a"] = "changed"
        inner.close()
        assert v.to_python() == {"host": "h", "test": {"a": "b"}, "more": [1]}
