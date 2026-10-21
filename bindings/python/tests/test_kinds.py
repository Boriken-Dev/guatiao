# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.kinds`: a kind described once, a table called through it, and
a table built over a Python object -- every shape a slot can take, both
ways round."""

from __future__ import annotations

import ctypes
import logging
import os
import re
import sys

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import greeter_table
import native

sys.path.insert(0, str(native.GREETER_KIND_PY))

from greeter_kind import COUNTER, CONVERSATION, GREETER, LISTENER  # noqa: E402

from guatiao import ABSENT, BadValue, Internal, Unsupported, Value, kinds  # noqa: E402
from guatiao.kinds import (  # noqa: E402
    BOOL,
    BYTES,
    BYTES_MUT,
    F64,
    I8,
    I64,
    LIST,
    MAP,
    STR,
    TEXT,
    USIZE,
    VALUE,
    FloorMismatch,
    Kind,
    Method,
    lent,
    taken,
)
from guatiao.kinds import _serve  # noqa: E402
from guatiao.registry import Registry  # noqa: E402

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]
_HEADER = native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.h"


def _defines() -> dict:
    text = _HEADER.read_text(encoding="utf-8")
    return {m.group(1): int(m.group(2)) for m in re.finditer(r"#define (\w+)_FLOOR_HASH (\d+)", text)}


class Ear:
    def __init__(self) -> None:
        self.heard_so_far: list = []

    def heard(self, what: str) -> None:
        self.heard_so_far.append(what)


# ---- the description itself -----------------------------------------------


def test_the_greeter_kinds_carry_the_headers_literals():
    defines = _defines()
    assert defines == {
        "greeter_vtable": GREETER.floor_hash,
        "counter_vtable": COUNTER.floor_hash,
        "conversation_vtable": CONVERSATION.floor_hash,
        "listener_vtable": LISTENER.floor_hash,
    }


def test_a_built_struct_is_the_headers_layout():
    word = ctypes.sizeof(ctypes.c_void_p)
    assert ctypes.sizeof(GREETER.struct) == 8 + 6 * word
    assert ctypes.sizeof(LISTENER.struct) == 8 + 2 * word
    # The hand-written mirror of the first three slots agrees, slot by slot.
    for name, slot in greeter_table.GreeterVtable._fields_[1:]:
        assert list(slot._argtypes_) == list(GREETER.slot_type(name)._argtypes_)
    # A struct written by hand is accepted when it agrees.
    Kind(
        "greeter",
        greeter_table.GreeterVtable,
        floor_hash=GREETER.floor_hash,
        methods=GREETER.methods[:3],
    )


def test_a_description_that_disagrees_with_its_struct_is_refused():
    with pytest.raises(TypeError, match="declares"):
        Kind("greeter", greeter_table.GreeterVtable, floor_hash=1, methods=GREETER.methods[:2])
    swapped = [Method("greet", [STR], TEXT, fallible=True), *GREETER.methods[1:3]]
    with pytest.raises(TypeError, match="prototype"):
        Kind("greeter", greeter_table.GreeterVtable, floor_hash=1, methods=swapped)
    with pytest.raises(TypeError, match="taken"):
        Method("start", [LISTENER], CONVERSATION)
    with pytest.raises(TypeError):
        Method("bad", [TEXT])
    with pytest.raises(TypeError, match="hide"):
        Kind("sized", floor_hash=1, methods=[Method("size", [], I64)])


# ---- calling a table a library built --------------------------------------


@pytest.fixture()
def reg():
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        yield registry


def test_a_rust_greeter_is_called_through_the_description(reg):
    greeter = reg.bind("derived_greeter_hello", GREETER)
    with greeter.greet("ana") as answer:
        assert answer.to_python() == {"greeting": "hello, ana"}
    assert greeter.shout("bo") == "HELLO, BO"
    with pytest.raises(BadValue) as refused:
        greeter.greet("")
    assert refused.value.message == "nobody to greet"
    counter = reg.bind("derived_greeter_hello", COUNTER)
    assert [counter.count(), counter.count()] == [1, 2]


def test_a_lent_map_is_changed_where_it_stands(reg):
    greeter = reg.bind("derived_greeter_hello", GREETER)
    with Value({"outer": {"name": "cy"}}) as config:
        assert greeter.check(config.as_map()["outer"]) is True
        assert config.to_python() == {"outer": {"name": "cy", "greeting": "hello, cy"}}
    with pytest.raises(TypeError, match="somewhere to go"):
        greeter.check({"name": "cy"})


def test_a_map_handed_over_leaves_the_caller_holding_nothing(reg):
    greeter = reg.bind("derived_greeter_hello", GREETER)
    config = Value({"name": "dee"})
    with greeter.greet_from(config) as answer:
        assert answer.to_python() == {"greeting": "hello, dee"}
    assert config.to_python() is ABSENT
    config.close()
    # Plain data, and a node inside another tree (cloned, left in place).
    assert greeter.greet_from({"name": "eve"}).to_python() == {"greeting": "hello, eve"}
    with Value({"a": {"name": "fay"}}) as outer:
        assert greeter.greet_from(outer.as_map()["a"]).to_python() == {"greeting": "hello, fay"}
        assert outer.to_python() == {"a": {"name": "fay"}}
    with pytest.raises(TypeError):
        greeter.greet_from(Value([1, 2]))


def test_objects_cross_both_ways_and_are_let_go(reg):
    greeter = reg.bind("derived_greeter_hello", GREETER)
    before = kinds.live_objects()
    ear = Ear()
    with greeter.start(ear) as conversation:
        assert isinstance(conversation, kinds.Object)
        conversation.say("hi")
        conversation.say("again")
        transcript = bytearray(32)
        n = conversation.read(transcript)
        assert bytes(transcript[:n]) == b"hi\nagain"
        assert conversation.turns() == 2
        with pytest.raises(BadValue):
            conversation.say("")
        assert kinds.live_objects() == before + 1, "the library holds the listener"
    assert ear.heard_so_far == ["hi", "again"]
    assert kinds.live_objects() == before, "destroying the conversation let the listener go"
    conversation.close()
    with pytest.raises(ValueError):
        conversation.turns()

    lent_ear = Ear()
    greeter.greet_into("gus", lent_ear)
    assert lent_ear.heard_so_far == ["hello, gus"]
    assert kinds.live_objects() == before, "a lent object is let go when the call returns"


def test_a_listener_that_raises_fails_the_call_that_reached_it(reg, caplog):
    greeter = reg.bind("derived_greeter_hello", GREETER)

    class Deaf:
        def heard(self, what: str) -> None:
            raise RuntimeError("cannot hear")

    with caplog.at_level(logging.ERROR, logger="guatiao.kinds"):
        with greeter.start(Deaf()) as conversation:
            with pytest.raises(Internal):
                conversation.say("hi")
    assert any("cannot fail and raised" in r.getMessage() for r in caplog.records)


def test_a_table_that_is_not_the_kinds_is_refused(reg):
    with pytest.raises(FloorMismatch):
        reg.bind("hello_library_greeter", GREETER)
    with pytest.raises(FloorMismatch):
        GREETER.bind(None, 0, None)


@pytest.fixture(scope="module")
def c_greeter_dir(tmp_path_factory):
    built = native.build_c_greeter(tmp_path_factory.mktemp("c-greeter"))
    if built is None:
        pytest.skip("no C compiler: the greeter written in C is not built")
    return built


def test_a_slot_an_older_table_lacks_is_unsupported(c_greeter_dir):
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(str(c_greeter_dir), rules="kind=greeter")
        greeter = registry.bind("c_greeter", GREETER)
        assert greeter.greet("hal").to_python() == {"greeting": "hello from C, hal"}
        assert not greeter.has("shout")
        with pytest.raises(Unsupported):
            greeter.shout("hal")
        with Value({"name": "ivy"}) as config:
            assert greeter.check(config)
            assert config.to_python() == {"name": "ivy", "checked_by": "C"}


# ---- a table over a Python object, called from Python through C pointers --


PROBE = Kind(
    "probe",
    floor_hash=12345,
    methods=[
        Method("add", [I64, I64], I64, required=True),
        Method("flip", [BOOL], BOOL),
        Method("half", [F64], F64),
        Method("small", [I8], I8),
        Method("echo", [STR], TEXT),
        Method("length", [BYTES], USIZE),
        Method("fill", [BYTES_MUT], USIZE),
        Method("keys", [MAP], LIST, fallible=True),
        Method("peek", [VALUE], VALUE),
        Method("take", [taken(MAP)], MAP, fallible=True),
        Method("grow", [lent(MAP)], fallible=True),
        Method("push", [lent(LIST)]),
        Method("boom", [], fallible=True),
        Method("refuse", [], fallible=True),
        Method("keep", [MAP], fallible=True),
        Method("absent", [], I64),
    ],
)


class Prober:
    def __init__(self) -> None:
        self.kept = None
        self.taken = None

    def add(self, a, b):
        return a + b

    def flip(self, b):
        return not b

    def half(self, x):
        return x / 2

    def small(self, x):
        return x * 100

    def echo(self, text):
        return f"<{text}>"

    def length(self, data):
        return len(data)

    def fill(self, buf):
        buf[:3] = b"abc"
        return 3

    def keys(self, m):
        return list(m.keys())

    def peek(self, v):
        return ABSENT if v is None else v.to_python()

    def take(self, m):
        self.taken = m
        m.as_map()["seen"] = True
        return m

    def grow(self, m):
        m["grown"] = [1, 2]

    def push(self, items):
        items.append("more")

    def boom(self):
        raise ValueError("on purpose")

    def refuse(self):
        raise BadValue("not today")

    def keep(self, m):
        self.kept = m


@pytest.fixture()
def probe():
    target = Prober()
    raw = _serve.serve(PROBE, target)
    remote = PROBE.bind(raw.table, raw.size, raw.ctx)
    yield remote, target
    _serve.release(raw.ctx)


def test_scalars_and_text_cross_and_come_back(probe):
    remote, _ = probe
    assert remote.add(40, 2) == 42
    assert remote.flip(True) is False
    assert remote.half(3.0) == 1.5
    assert remote.echo("héllo") == "<héllo>"
    assert remote.length(b"\x00\x01\x02") == 3
    buf = bytearray(5)
    assert remote.fill(buf) == 3
    assert bytes(buf) == b"abc\x00\x00"
    with pytest.raises(OverflowError):
        remote.small(1000)
    with pytest.raises(TypeError):
        remote.add("1", 2)
    # A callee answering past I8 fails a slot that cannot fail.
    with pytest.raises(Internal):
        remote.small(2)


def test_trees_cross_in_every_mode(probe):
    remote, target = probe
    assert remote.keys({"a": 1, "b": 2}).to_python() == ["a", "b"]
    assert remote.peek(None).to_python() is ABSENT
    assert remote.peek([1, "x"]).to_python() == [1, "x"]
    with Value({"k": "v"}) as handed:
        answer = remote.take(handed)
        assert handed.to_python() is ABSENT
    assert answer.to_python() == {"k": "v", "seen": True}
    with Value({"x": 1}) as lent_map:
        remote.grow(lent_map)
        assert lent_map.to_python() == {"x": 1, "grown": [1, 2]}
    with Value(["a"]) as items:
        remote.push(items)
        assert items.to_python() == ["a", "more"]


def test_a_view_kept_past_its_call_raises(probe):
    remote, target = probe
    remote.keep({"secret": 1})
    with pytest.raises(ValueError, match="lent for one call"):
        target.kept.to_python()


def test_an_exception_crosses_as_a_status_with_its_words(probe):
    remote, _ = probe
    with pytest.raises(Internal) as boom:
        remote.boom()
    assert boom.value.message == "ValueError: on purpose"
    with pytest.raises(BadValue) as refused:
        remote.refuse()
    assert refused.value.message == "not today"
    with pytest.raises(AttributeError):
        remote.nothing


def test_a_method_the_class_lacks_leaves_its_slot_null(probe):
    remote, _ = probe
    assert not remote.has("absent")
    with pytest.raises(Unsupported):
        remote.absent()

    class Nothing:
        pass

    with pytest.raises(TypeError, match="requires"):
        _serve.serve(PROBE, Nothing())


def test_a_released_ctx_answers_gone(probe):
    remote, _ = probe
    _serve.release(remote.ctx)
    from guatiao import Gone

    with pytest.raises(Gone):
        remote.keys({})
