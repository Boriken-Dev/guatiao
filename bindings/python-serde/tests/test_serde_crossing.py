# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A tree crosses between guatiao_serde and guatiao: built by one library,
read and freed through the other."""

from __future__ import annotations

import pickle

import pytest

import guatiao_serde as serde
from guatiao import LibraryNotFound, Value, WrongKind, ffi

try:
    serde.library()
except LibraryNotFound:
    pytest.skip("no guatiao_serde library built", allow_module_level=True)

from serde_counting import Counting


def test_a_tree_serde_built_is_freed_through_the_core():
    counting = Counting(serde.library().alloc)
    value = serde.loads('{"a": [1, "two", {"b": null}]}', alloc=counting.alloc)
    assert counting.live > 0
    value.as_map()["c"] = "added through the core"
    assert value.to_python() == {"a": [1, "two", {"b": None}], "c": "added through the core"}
    value.close()
    assert counting.live == 0


def test_a_tree_the_core_built_is_written_by_serde_and_freed_by_the_core():
    counting = Counting(serde.library().alloc)
    value = Value.from_python({"port": 5900, "tags": ["x"]}, alloc=counting.alloc)
    assert serde.dumps(value) == '{"port":5900,"tags":["x"]}'
    assert serde.dumps(value.as_map()["tags"]) == '["x"]'
    value.close()
    assert counting.live == 0


def test_a_tree_in_serdes_own_allocator_is_freed_by_the_core():
    for _ in range(3):
        with serde.loads('{"k": "v"}') as value:
            assert value.to_python() == {"k": "v"}


def test_parse_error_layout():
    import ctypes

    assert ctypes.sizeof(serde._CParseError) == 56
    assert serde._CParseError.format.offset == 0
    assert serde._CParseError.line.offset == 4
    assert serde._CParseError.column.offset == 8
    assert serde._CParseError.message.offset == 16


def test_a_parse_error_pickles():
    error = serde.ParseError("expected a value", format="json", line=3, column=7)
    back = pickle.loads(pickle.dumps(error))
    assert type(back) is serde.ParseError
    assert (back.format, back.line, back.column) == ("json", 3, 7)
    assert back.message == "expected a value"
    assert str(back) == str(error)


def test_a_refusal_that_is_not_a_syntax_error_keeps_its_message():
    import ctypes

    core = ffi.Library(
        ffi.find_library("guatiao"),
        {
            "guatiao_value_string": (
                [ctypes.POINTER(ffi.Alloc), ffi.Str, ctypes.POINTER(ffi.Node)],
                ctypes.c_uint32,
                None,
            ),
            "guatiao_alloc_default": ([ctypes.POINTER(ffi.Alloc)], ctypes.c_uint32, None),
        },
    )
    error = serde._CParseError()
    view, _keep = ffi.text("the library's own words")
    assert core.guatiao_value_string(
        ctypes.byref(core.alloc), view, ctypes.byref(error.message)
    ) == ffi.Status.OK
    with pytest.raises(WrongKind) as refusal:
        serde._finish_parse(ffi.Status.ERR_WRONG_KIND, error, "json")
    assert refusal.value.message == "the library's own words"
