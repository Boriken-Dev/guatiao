# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Who has a value's tree: a call lending it has it alone, a call
borrowing it and every read share it, and whatever cannot have it is
refused with `BufferError` at once, from any thread."""

from __future__ import annotations

import threading
from typing import Any, Callable

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

from guatiao import Value  # noqa: E402
from guatiao import value as value_module  # noqa: E402
from guatiao.kinds import Kind, Method, _serve  # noqa: E402
from guatiao.kinds.shapes import LIST, MAP, VALUE, lent  # noqa: E402

PAIRS = Kind(
    "pairs",
    floor_hash=4242,
    methods=[
        Method("lend_then_borrow", [lent(MAP), MAP], fallible=True),
        Method("borrow_then_lend", [MAP, lent(MAP)], fallible=True),
        Method("lend_twice", [lent(MAP), lent(MAP)], fallible=True),
        Method("lend_then_borrow_value", [lent(VALUE), VALUE], fallible=True),
        Method("lend_then_borrow_list", [lent(LIST), LIST], fallible=True),
        Method("borrow_then_lend_list", [LIST, lent(LIST)], fallible=True),
        Method("borrow_twice", [MAP, MAP], fallible=True),
        Method("pass_on", [lent(MAP)], fallible=True),
    ],
)


class Pairs:
    def __init__(self) -> None:
        self.calls = 0
        self.remote: Any = None
        self.outcomes: "list[object]" = []

    def lend_then_borrow(self, a: Any, b: Any) -> None:
        self.calls += 1
        a["seen"] = len(b)

    def borrow_then_lend(self, a: Any, b: Any) -> None:
        self.calls += 1
        b["seen"] = len(a)

    def lend_twice(self, a: Any, b: Any) -> None:
        self.calls += 1

    def lend_then_borrow_value(self, a: Any, b: Any) -> None:
        self.calls += 1

    def lend_then_borrow_list(self, a: Any, b: Any) -> None:
        self.calls += 1
        a.append(len(b))

    def borrow_then_lend_list(self, a: Any, b: Any) -> None:
        self.calls += 1
        b.append(len(a))

    def borrow_twice(self, a: Any, b: Any) -> None:
        self.calls += 1
        assert a.to_python() == b.to_python()

    def pass_on(self, m: Any) -> None:
        # The map this call was lent, passed on to another call twice.
        for call in ("lend_then_borrow", "borrow_then_lend", "lend_twice"):
            try:
                getattr(self.remote, call)(m, m)
                self.outcomes.append("called")
            except BufferError:
                self.outcomes.append(BufferError)
        self.remote.borrow_twice(m, m)
        m["passed"] = True


@pytest.fixture()
def pairs():
    target = Pairs()
    raw = _serve.serve(PAIRS, target)
    remote = PAIRS.bind(raw.table, raw.size, raw.ctx)
    target.remote = remote
    yield remote, target
    _serve.release(raw.ctx)


@pytest.mark.parametrize(
    "slot, words",
    [
        ("lend_then_borrow", "lent to a call: it cannot be read"),
        ("borrow_then_lend", "being read: it cannot be lent to a call"),
        ("lend_twice", "lent to a call: it cannot be lent to a call"),
    ],
)
def test_a_value_is_not_lent_and_borrowed_or_lent_twice_in_one_call(pairs, slot, words):
    remote, target = pairs
    v = Value({"k": "v"})
    with pytest.raises(BufferError, match=words):
        getattr(remote, slot)(v, v)
    assert target.calls == 0, "refused before the call"
    # Every claim the refused call took is given back.
    v.as_map()["after"] = 1
    assert v.to_python() == {"k": "v", "after": 1}
    v.close()


@pytest.mark.parametrize(
    "slot, words",
    [
        ("lend_then_borrow_list", "lent to a call: it cannot be read"),
        ("borrow_then_lend_list", "being read: it cannot be lent to a call"),
    ],
)
def test_a_list_is_not_lent_and_borrowed_in_one_call(pairs, slot, words):
    remote, target = pairs
    v = Value(["a"])
    with pytest.raises(BufferError, match=words):
        getattr(remote, slot)(v, v)
    assert target.calls == 0, "refused before the call"
    v.as_list().append("after")
    assert v.to_python() == ["a", "after"]
    v.close()


def test_a_node_of_a_value_is_not_lent_while_its_value_is_borrowed(pairs):
    remote, target = pairs
    v = Value({"inner": {"k": "v"}})
    with pytest.raises(BufferError):
        remote.lend_then_borrow(v.as_map()["inner"].as_map(), v)
    with pytest.raises(BufferError):
        remote.lend_then_borrow_value(v, v.as_map()["inner"])
    assert target.calls == 0
    v.close()


def test_two_values_lend_and_borrow_in_one_call_and_one_value_is_borrowed_twice(pairs):
    remote, target = pairs
    with Value({"k": "v"}) as a, Value({"x": 1, "y": 2}) as b:
        remote.lend_then_borrow(a, b)
        remote.borrow_then_lend(a, b)
        assert a.to_python() == {"k": "v", "seen": 2}
        assert b.to_python() == {"x": 1, "y": 2, "seen": 2}
        remote.borrow_twice(b, b)
    assert target.calls == 3


def test_a_provider_cannot_pass_on_what_it_was_lent_as_lent_and_borrowed(pairs):
    remote, target = pairs
    with Value({"k": "v"}) as v:
        remote.pass_on(v)
        assert target.outcomes == [BufferError, BufferError, BufferError]
        assert v.to_python() == {"k": "v", "passed": True}


# -- a read on one thread, a close or a change on another ------------------


def _close(v: Value) -> None:
    v.close()


def _change(v: Value) -> None:
    v.as_map()["more"] = 1


def _racing(monkeypatch: Any, where: Any, name: str, v: Value, other: Callable[[Value], None]) -> "list[object]":
    """Patches `where.name` so that, the first time it runs, another thread
    tries `other(v)` while this one is inside the read; what the other
    thread came to is recorded. A read whose value was freed under it
    answers nothing rather than reading freed memory."""
    real = getattr(where, name)
    seen: "list[object]" = []

    def attempt() -> None:
        try:
            other(v)
            seen.append("done")
        except BufferError:
            seen.append(BufferError)

    def racing(*args: Any, **kwargs: Any) -> Any:
        if not seen:
            seen.append("started")
            thread = threading.Thread(target=attempt)
            thread.start()
            thread.join()
            seen.remove("started")
            if v._closed:
                return None
        return real(*args, **kwargs)

    monkeypatch.setattr(where, name, racing)
    return seen


_READS = {
    "to_python": (lambda: value_module, "_node_to_python", lambda v: v.to_python()),
    "clone": (lambda: _lib.core(), "guatiao_value_clone", lambda v: v.clone().to_python()),
    "len": (lambda: value_module, "_walk", lambda v: len(v.as_map())),
    "getitem": (lambda: value_module, "_map_find", lambda v: v.as_map()["k"].to_python()),
}


@pytest.mark.parametrize("other", [_close, _change], ids=["close", "change"])
@pytest.mark.parametrize("read", sorted(_READS))
def test_a_read_refuses_a_close_or_change_on_another_thread_until_it_returns(monkeypatch, read, other):
    where, name, use = _READS[read]
    v = Value({"k": "v"})
    expected = use(v)
    seen = _racing(monkeypatch, where(), name, v, other)
    assert use(v) == expected
    assert seen == [BufferError]
    monkeypatch.undo()
    other(v)


_COPIES = {
    "setitem": ("guatiao_value_clone", lambda into, w: into.as_map().__setitem__("copy", w.as_map()["k"])),
    "append": ("guatiao_value_clone", lambda into, w: into.as_map()["items"].as_list().append(w.as_map()["k"])),
    "Value": ("guatiao_value_clone", lambda into, w: Value(w.as_map()["k"]).close()),
    "copy_from": ("guatiao_map_copy_from", lambda into, w: into.as_map().copy_from(w.as_map())),
}


@pytest.mark.parametrize("copy", sorted(_COPIES))
def test_copying_from_another_value_refuses_its_close_on_another_thread(monkeypatch, copy):
    name, use = _COPIES[copy]
    into = Value({"items": []})
    w = Value({"k": "v"})
    seen = _racing(monkeypatch, _lib.core(), name, w, _close)
    use(into, w)
    assert seen == [BufferError]
    monkeypatch.undo()
    w.close()
    into.close()


def test_a_read_on_another_thread_while_a_value_changes_is_refused(monkeypatch):
    core = _lib.core()
    v = Value({"k": "v"})
    seen = _racing(monkeypatch, core, "guatiao_map_set", v, lambda v: v.to_python())
    v.as_map()["more"] = 1
    assert seen == [BufferError]
    assert v.to_python() == {"k": "v", "more": 1}
    v.close()


class Holder(Pairs):
    """Holds a value its caller borrows to the call, and tries to change
    and close it from inside the call."""

    held: "Value | None" = None

    def borrow_twice(self, a: Any, b: Any) -> None:
        held = self.held
        assert held is not None
        for attempt in (lambda: held.as_map().__setitem__("x", 1), held.close):
            try:
                attempt()
                self.outcomes.append("done")
            except BufferError:
                self.outcomes.append(BufferError)
        self.outcomes.append(held.to_python())


def test_a_value_borrowed_by_a_call_is_not_changed_or_closed_by_a_provider_holding_it():
    holder = Holder()
    raw = _serve.serve(PAIRS, holder)
    try:
        remote = PAIRS.bind(raw.table, raw.size, raw.ctx)
        v = Value({"k": "v"})
        holder.held = v
        remote.borrow_twice(v, {"a": 1})
        assert holder.outcomes == [BufferError, BufferError, {"k": "v"}]
        v.close()
    finally:
        _serve.release(raw.ctx)
