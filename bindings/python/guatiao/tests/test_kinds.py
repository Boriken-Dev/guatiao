# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.kinds`: a kind described once, a table called through it, and
a table built over a Python object -- every shape a slot can take, both
ways round."""

from __future__ import annotations

import ctypes
import json
import logging
import os
import re
import sys
from typing import Protocol

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import greeter_table
import native

sys.path.insert(0, str(native.GREETER_KIND_PY))

from greeter_kind import Conversation, Counter, Greeter, Listener, Wide  # noqa: E402

from guatiao import ABSENT, BadValue, Internal, List, Unsupported, Value, kinds  # noqa: E402
from guatiao.kinds import FloorMismatch, Kind, KindProtocol, Lent, Method, kind_of  # noqa: E402
from guatiao.kinds.shapes import (  # noqa: E402
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
    lent,
    taken,
)
from guatiao.kinds import _description, _serve  # noqa: E402
from guatiao.registry import Registry  # noqa: E402

_TARGET_DEBUG = native.library_dir()
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
        "greeter_vtable": Greeter.kind.floor_hash,
        "counter_vtable": Counter.kind.floor_hash,
        "conversation_vtable": Conversation.kind.floor_hash,
        "listener_vtable": Listener.kind.floor_hash,
        "wide_vtable": Wide.kind.floor_hash,
    }


def test_a_built_struct_is_the_headers_layout():
    word = ctypes.sizeof(ctypes.c_void_p)
    assert ctypes.sizeof(Greeter.kind.struct) == 8 + 6 * word
    assert ctypes.sizeof(Listener.kind.struct) == 8 + 2 * word
    # The hand-written mirror of the first three slots agrees, slot by slot.
    for name, slot in greeter_table.GreeterVtable._fields_[1:]:
        assert list(slot._argtypes_) == list(Greeter.kind.slot_type(name)._argtypes_)
    # A struct written by hand is accepted when it agrees.
    Kind(
        "greeter",
        greeter_table.GreeterVtable,
        floor_hash=Greeter.kind.floor_hash,
        methods=Greeter.kind.methods[:3],
    )


def test_a_description_that_disagrees_with_its_struct_is_refused():
    with pytest.raises(TypeError, match="declares"):
        Kind("greeter", greeter_table.GreeterVtable, floor_hash=1, methods=Greeter.kind.methods[:2])
    swapped = [Method("greet", [STR], TEXT, fallible=True), *Greeter.kind.methods[1:3]]
    with pytest.raises(TypeError, match="prototype"):
        Kind("greeter", greeter_table.GreeterVtable, floor_hash=1, methods=swapped)
    with pytest.raises(TypeError, match="taken"):
        Method("start", [Listener.kind], Conversation.kind)
    with pytest.raises(TypeError):
        Method("bad", [TEXT])
    with pytest.raises(TypeError, match="hide"):
        Kind("sized", floor_hash=1, methods=[Method("size", [], I64)])


_DOCUMENT = native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json"


def _same_kind(a: Kind, b: Kind) -> None:
    assert (a.name, a.floor_hash, a.is_object) == (b.name, b.floor_hash, b.is_object)
    assert [f[0] for f in a.struct._fields_] == [f[0] for f in b.struct._fields_]
    for x, y in zip(a.methods, b.methods):
        assert (x.name, x.fallible, x.required, x.prototype()) == (y.name, y.fallible, y.required, y.prototype())


def test_a_kind_loaded_from_its_description_is_the_kind_its_class_declares():
    text = _DOCUMENT.read_text(encoding="utf-8")
    loaded = kinds.kinds_of(text)
    assert list(loaded) == ["greeter", "listener", "conversation", "counter", "wide"]
    for cls in (Greeter, Listener, Conversation, Counter, Wide):
        _same_kind(loaded[cls.kind.name], cls.kind)
    # One kind alone finds the object kinds it names among the known ones.
    greeter_only = {"format": 1, "kinds": [k for k in json.loads(text)["kinds"] if k["name"] == "greeter"]}
    with pytest.raises(TypeError, match="no kind listener"):
        kinds.kinds_of(greeter_only)
    _same_kind(kinds.kinds_of(greeter_only, known=[Listener, Conversation])["greeter"], Greeter.kind)


def test_the_committed_kinds_module_is_what_the_description_writes():
    written = kinds.generate(
        _DOCUMENT.read_text(encoding="utf-8"),
        source="../include/greeter_kind.kinds.json",
        command="python -m guatiao.kinds ../include/greeter_kind.kinds.json -o greeter_kind.py",
    )
    committed = (native.GREETER_KIND_PY / "greeter_kind.py").read_text(encoding="utf-8")
    assert written == committed, "regenerate: the command is in the module's first lines"


def test_a_description_that_is_not_one_is_refused_naming_where():
    def one(slots):
        return {"format": 1, "kinds": [{"name": "k", "floor_hash": 1, "object": False, "slots": slots}]}

    with pytest.raises(TypeError, match="format"):
        kinds.kinds_of({"format": 2, "kinds": []})
    with pytest.raises(TypeError, match=r"k\.x argument 0: 'u128'"):
        kinds.kinds_of(one([{"name": "x", "args": [{"name": "a", "type": "u128"}]}]))
    with pytest.raises(TypeError, match="answer cannot be 'str'"):
        kinds.kinds_of(one([{"name": "x", "answer": {"type": "str"}}]))
    with pytest.raises(TypeError, match="list cannot be passed borrowed"):
        kinds.kinds_of(one([{"name": "x", "args": [{"name": "a", "type": "list", "pass": "borrowed"}]}]))
    loop = {
        "name": "node",
        "floor_hash": 2,
        "object": True,
        "slots": [{"name": "next", "args": [], "answer": {"type": "object", "kind": "node"}}],
    }
    with pytest.raises(TypeError, match="refers to itself"):
        kinds.kinds_of({"format": 1, "kinds": [loop]})


def test_kind_classes_describe_as_the_document_they_were_written_from():
    committed = json.loads(_DOCUMENT.read_text(encoding="utf-8"))
    assert kinds.describe_kinds(Greeter, Counter, Wide) == committed
    # The hash a kind born here gets is the one #[guatiao::kind] wrote.
    for kind in committed["kinds"]:
        assert _description.floor_hash_of(kind["object"], kind["slots"]) == kind["floor_hash"], kind["name"]


def test_the_floor_hash_is_the_one_rust_gives_the_shared_vectors():
    path = native.REPO / "crates" / "guatiao-derive" / "tests" / "vectors" / "floor_hash.json"
    vectors = json.loads(path.read_text(encoding="utf-8"))["vectors"]
    assert len(vectors) >= 6
    for v in vectors:
        assert _description.floor_hash_of(v["object"], v["slots"]) == v["floor_hash"], v["name"]


def test_a_kind_born_in_python_is_written_as_its_description():
    import tally_kinds

    committed = json.loads(
        (native.REPO / "examples" / "tally_kind" / "include" / "tally_kind.kinds.json").read_text(encoding="utf-8")
    )
    assert kinds.describe_kinds(tally_kinds.Watcher, tally_kinds.Tally) == committed
    # And the document reads back as the kinds the classes are.
    loaded = kinds.kinds_of(committed)
    _same_kind(loaded["tally"], tally_kinds.Tally.kind)
    _same_kind(loaded["watcher"], tally_kinds.Watcher.kind)


def test_a_host_binds_a_kind_from_what_the_library_carries():
    with Registry("py-carried", "0.1") as registry:
        registry.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        carried = kinds.kinds_of(registry.kind_descriptions("derived_greeter_hello"))
        greeter = registry.bind("derived_greeter_hello", carried["greeter"])
        with greeter.greet("ana") as answer:
            assert answer.to_python() == {"greeting": "hello, ana"}


# ---- calling a table a library built --------------------------------------


@pytest.fixture()
def reg():
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        yield registry


def test_a_rust_greeter_is_called_through_the_description(reg):
    greeter = reg.bind("derived_greeter_hello", Greeter.kind)
    with greeter.greet("ana") as answer:
        assert answer.to_python() == {"greeting": "hello, ana"}
    assert greeter.shout("bo") == "HELLO, BO"
    with pytest.raises(BadValue) as refused:
        greeter.greet("")
    assert refused.value.message == "nobody to greet"
    counter = reg.bind("derived_greeter_hello", Counter)
    assert [counter.count(), counter.count()] == [1, 2]


_I64_LIMITS = (2**53 + 1, -(2**63), 2**63 - 1)
_U64_LIMITS = (2**53 + 1, 2**63, 2**64 - 1)


def _crosses_exactly(wide) -> None:
    for x in _I64_LIMITS:
        assert wide.echo_i64(x) == x
    for x in _U64_LIMITS:
        assert wide.echo_u64(x) == x


def _refuses_what_does_not_fit(wide) -> None:
    for x in (2**63, -(2**63) - 1, 2**64):
        with pytest.raises(OverflowError, match="does not fit"):
            wide.echo_i64(x)
    for x in (-1, 2**64, -(2**63)):
        with pytest.raises(OverflowError, match="does not fit"):
            wide.echo_u64(x)
    for x in (1.5, True, "5", None):
        with pytest.raises(TypeError, match="takes an int"):
            wide.echo_i64(x)
        with pytest.raises(TypeError, match="takes an int"):
            wide.echo_u64(x)


def test_64_bit_integers_cross_a_rust_slot_exactly(reg):
    wide = reg.bind("derived_greeter_hello", Wide.kind)
    _crosses_exactly(wide)
    _refuses_what_does_not_fit(wide)


def test_a_lent_map_is_changed_where_it_stands(reg):
    greeter = reg.bind("derived_greeter_hello", Greeter.kind)
    with Value({"outer": {"name": "cy"}}) as config:
        assert greeter.check(config.as_map()["outer"]) is True
        assert config.to_python() == {"outer": {"name": "cy", "greeting": "hello, cy"}}
    with pytest.raises(TypeError, match="somewhere to go"):
        greeter.check({"name": "cy"})


def test_a_map_handed_over_leaves_the_caller_holding_nothing(reg):
    greeter = reg.bind("derived_greeter_hello", Greeter.kind)
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
    greeter = reg.bind("derived_greeter_hello", Greeter.kind)
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
    greeter = reg.bind("derived_greeter_hello", Greeter.kind)

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
        reg.bind("hello_library_greeter", Greeter.kind)
    with pytest.raises(FloorMismatch):
        Greeter.kind.bind(None, 0, None)


@pytest.fixture(scope="module")
def c_greeter_dir(tmp_path_factory):
    built = native.build_c_greeter(tmp_path_factory.mktemp("c-greeter"))
    if built is None:
        pytest.skip("no C compiler: the greeter written in C is not built")
    return built


def test_a_slot_an_older_table_lacks_is_unsupported(c_greeter_dir):
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(str(c_greeter_dir), rules="kind=greeter")
        greeter = registry.bind("c_greeter", Greeter.kind)
        assert greeter.greet("hal").to_python() == {"greeting": "hello from C, hal"}
        assert not greeter.has("shout")
        with pytest.raises(Unsupported):
            greeter.shout("hal")
        with Value({"name": "ivy"}) as config:
            assert greeter.check(config)
            assert config.to_python() == {"name": "ivy", "checked_by": "C"}


def test_an_answer_built_in_the_librarys_allocator_outlives_the_unload(tmp_path):
    # This greeter ignores the carrier; the answer is moved home on
    # arrival, or freeing it after the unload calls into unmapped code.
    built = native.build_c_greeter(tmp_path, ignores_carrier=True)
    if built is None:
        pytest.skip("no C compiler: the greeter written in C is not built")
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(str(built), rules="kind=greeter")
        answer = registry.bind("c_greeter", Greeter.kind).greet("hal")
        # The C greeter has no `unload` function: nothing it handed out
        # is alive, which the caller states.
        registry.unload("c_greeter_library", unchecked=True)
        assert not registry.libraries()
        assert answer.to_python() == {"greeting": "hello from C, hal"}
        answer.close()


def test_what_a_slot_grew_a_lent_tree_with_outlives_the_unload(tmp_path):
    # This greeter's `check` rebuilds the lent map in its own allocator;
    # the map is settled into the one it recorded before the call.
    built = native.build_c_greeter(tmp_path, ignores_carrier=True)
    if built is None:
        pytest.skip("no C compiler: the greeter written in C is not built")
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(str(built), rules="kind=greeter")
        with Value({"name": "ivy"}) as config:
            assert registry.bind("c_greeter", Greeter.kind).check(config)
            registry.unload("c_greeter_library", unchecked=True)
            assert config.to_python() == {"name": "ivy", "checked_by": "C"}


def test_64_bit_integers_cross_a_slot_in_c_exactly(c_greeter_dir):
    with Registry("py-kinds", "0.1") as registry:
        registry.scan_dir(str(c_greeter_dir), rules="kind=greeter")
        wide = registry.bind("c_greeter", Wide.kind)
        _crosses_exactly(wide)
        _refuses_what_does_not_fit(wide)


class Echoes:
    def echo_i64(self, x):
        return x

    def echo_u64(self, x):
        return x


class Overshoots:
    def echo_i64(self, x):
        return x + 1

    def echo_u64(self, x):
        return x - 1


def test_64_bit_integers_cross_a_slot_written_in_python_exactly():
    raw = _serve.serve(Wide.kind, Echoes())
    try:
        wide = Wide.kind.bind(raw.table, raw.size, raw.ctx)
        _crosses_exactly(wide)
        _refuses_what_does_not_fit(wide)
    finally:
        _serve.release(raw.ctx)


def test_an_answer_that_does_not_fit_fails_the_call_that_asked():
    raw = _serve.serve(Wide.kind, Overshoots())
    try:
        wide = Wide.kind.bind(raw.table, raw.size, raw.ctx)
        with pytest.raises(Internal, match="OverflowError"):
            wide.echo_i64(2**63 - 1)
        with pytest.raises(Internal, match="OverflowError"):
            wide.echo_u64(0)
    finally:
        _serve.release(raw.ctx)


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


# ---- a kind written as a class ----------------------------------------------


def test_a_kind_class_refuses_what_has_no_shape_when_it_is_written() -> None:
    with pytest.raises(TypeError, match="width"):

        class Wide(KindProtocol, Protocol, kind="wide", floor_hash=1):
            def count(self) -> int: ...

    with pytest.raises(TypeError, match="no shape"):

        class Odd(KindProtocol, Protocol, kind="odd", floor_hash=1):
            def take(self, what: set) -> None: ...  # type: ignore[type-arg]

    with pytest.raises(TypeError, match="Taken"):

        class Bare(KindProtocol, Protocol, kind="bare", floor_hash=1):
            def take(self, listener: Listener) -> None: ...

    with pytest.raises(TypeError, match="never borrowed"):

        class Borrowed(KindProtocol, Protocol, kind="borrowed", floor_hash=1):
            def take(self, items: List) -> None: ...

    with pytest.raises(TypeError, match="annotation"):

        class Untyped(KindProtocol, Protocol, kind="untyped", floor_hash=1):
            def take(self, items): ...  # type: ignore[no-untyped-def]

    class Fine(KindProtocol, Protocol, kind="fine", floor_hash=5):
        def grow(self, items: Lent[List]) -> None: ...

    assert kind_of(Fine) is Fine.kind
    assert Fine.kind.methods[0].args[0] is not None
    with pytest.raises(TypeError):
        kind_of(3)
