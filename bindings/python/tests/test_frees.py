# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The binding frees what it allocates on every path, proved with an
allocator that counts the blocks it has handed out and not taken back."""

from __future__ import annotations

import ctypes

import pytest

from guatiao import _abi, _lib
from guatiao.value import Value


class Counting:
    """The library's default allocator, wrapped to count live blocks."""

    def __init__(self) -> None:
        base = _lib.core().alloc
        self.live = 0
        alloc_type = _abi.Alloc._fields_[2][1]
        free_type = _abi.Alloc._fields_[3][1]
        realloc_type = _abi.Alloc._fields_[4][1]

        def _alloc(_ctx, size, align):
            ptr = base.alloc(base.ctx, size, align)
            if ptr:
                self.live += 1
            return ptr

        def _free(_ctx, ptr, size, align):
            if ptr:
                self.live -= 1
            base.free(base.ctx, ptr, size, align)

        # Held on self: ctypes drops a callback nothing references.
        self._callbacks = (alloc_type(_alloc), free_type(_free))
        self.alloc = _abi.Alloc(
            ctypes.sizeof(_abi.Alloc),
            None,
            self._callbacks[0],
            self._callbacks[1],
            realloc_type(),
        )


@pytest.fixture
def counting() -> Counting:
    return Counting()


class Unconvertible:
    pass


def test_a_clean_build_and_close_leaves_nothing(counting):
    Value([[1, "x"], {"k": [2, 3]}], alloc=counting.alloc).close()
    assert counting.live == 0


@pytest.mark.parametrize(
    "bad",
    [
        [[1, "x", Unconvertible()]],
        {"a": [1, 2], "b": {"c": "d", "e": Unconvertible()}},
        [{"a": 1}, {"b": [True, None, Unconvertible()]}],
        {"a": 1, 2: "x"},
        [float("nan")],
    ],
)
def test_a_conversion_that_fails_part_way_frees_what_it_built(counting, bad):
    with pytest.raises((TypeError, ValueError)):
        Value(bad, alloc=counting.alloc)
    assert counting.live == 0


def test_a_failed_map_assignment_frees_the_partial_value(counting):
    with Value({"a": 1}, alloc=counting.alloc) as v:
        before = counting.live
        with pytest.raises(TypeError):
            v.as_map()["b"] = [1, "two", Unconvertible()]
        assert counting.live == before
        assert v.to_python() == {"a": 1}
    assert counting.live == 0


def test_a_failed_list_append_frees_the_partial_value(counting):
    with Value(["a"], alloc=counting.alloc) as v:
        before = counting.live
        with pytest.raises(TypeError):
            v.as_list().append({"k": ["x", Unconvertible()]})
        assert counting.live == before
    assert counting.live == 0


def test_a_failed_list_assignment_frees_the_cloned_siblings(counting):
    with Value(["a", ["b", "c"], {"d": "e"}], alloc=counting.alloc) as v:
        before = counting.live
        with pytest.raises(TypeError):
            v.as_list()[0] = Unconvertible()
        with pytest.raises(TypeError):
            v.as_list()[1] = ["x", Unconvertible()]
        with pytest.raises(IndexError):
            v.as_list()[9] = "x"
        assert counting.live == before
        assert v.to_python() == ["a", ["b", "c"], {"d": "e"}]
    assert counting.live == 0


def test_a_failed_list_insert_frees_the_cloned_siblings(counting):
    with Value(["a", ["b", "c"]], alloc=counting.alloc) as v:
        before = counting.live
        with pytest.raises(TypeError):
            v.as_list().insert(1, [Unconvertible()])
        assert counting.live == before
        assert v.to_python() == ["a", ["b", "c"]]
    assert counting.live == 0


def test_a_successful_list_assignment_and_insert_leave_nothing(counting):
    with Value(["a", ["b"]], alloc=counting.alloc) as v:
        v.as_list()[0] = {"k": [1]}
        v.as_list().insert(1, "mid")
        assert v.to_python() == [{"k": [1]}, "mid", ["b"]]
    assert counting.live == 0
