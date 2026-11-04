# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A view `at()` answers borrows from its value, so what holds that value
holds the view: a call cannot lend the value and borrow the view, and a
read through the view keeps a close or a change on another thread out."""

from __future__ import annotations

import threading
from typing import Any, Callable

import pytest

import guatiao_intake as intake
from guatiao import LibraryNotFound, Value, _lib, wire
from guatiao import value as value_module
from guatiao.kinds import Kind, Method, _serve
from guatiao.kinds.shapes import MAP, lent

try:
    intake.library()
except LibraryNotFound:
    pytest.skip("no guatiao_intake library built", allow_module_level=True)

PAIRS = Kind(
    "intake_pairs",
    floor_hash=4243,
    methods=[
        Method("lend_then_borrow", [lent(MAP), MAP], fallible=True),
        Method("borrow_then_lend", [MAP, lent(MAP)], fallible=True),
    ],
)


class Pairs:
    calls = 0

    def lend_then_borrow(self, a: Any, b: Any) -> None:
        self.calls += 1

    def borrow_then_lend(self, a: Any, b: Any) -> None:
        self.calls += 1


@pytest.mark.parametrize("slot", ["lend_then_borrow", "borrow_then_lend"])
def test_a_call_refuses_a_value_lent_beside_a_view_at_borrows_from_it(slot):
    target = Pairs()
    raw = _serve.serve(PAIRS, target)
    try:
        remote = PAIRS.bind(raw.table, raw.size, raw.ctx)
        with Value({"a": {"k": "v"}}) as v:
            view = intake.at(v, "a")
            assert view is not None
            args = (v, view) if slot == "lend_then_borrow" else (view, v)
            with pytest.raises(BufferError):
                getattr(remote, slot)(*args)
            assert target.calls == 0, "refused before the call"
            v.as_map()["after"] = 1
    finally:
        _serve.release(raw.ctx)


def _racing(monkeypatch: Any, where: Any, name: str, v: Value) -> "list[object]":
    """Patches `where.name` so that, the first time it runs, another thread
    tries to close `v`; what that came to is recorded. A read whose value
    was freed under it answers nothing rather than reading freed memory."""
    real = getattr(where, name)
    seen: "list[object]" = []

    def attempt() -> None:
        try:
            v.close()
            seen.append("closed")
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


_READS: "dict[str, tuple[Callable[[], Any], str, Callable[[Any], Any]]]" = {
    "to_python": (lambda: value_module, "_node_to_python", lambda view: view.to_python()),
    "wire.send": (lambda: _lib.core(), "guatiao_wire_send", lambda view: wire.send(1, view)),
}


@pytest.mark.parametrize("read", sorted(_READS))
def test_a_read_through_a_view_at_answers_refuses_a_close_on_another_thread(monkeypatch, read):
    where, name, use = _READS[read]
    v = Value({"a": {"k": "v"}})
    view = intake.at(v, "a")
    expected = use(view)
    seen = _racing(monkeypatch, where(), name, v)
    assert use(view) == expected
    assert seen == [BufferError]
    monkeypatch.undo()
    v.close()
