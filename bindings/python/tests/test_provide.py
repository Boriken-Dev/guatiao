# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.provide`: a greeter written in Python passes the behaviour the
Rust and C greeters pass, called from Python through the same registry."""

from __future__ import annotations

import os
import sys
import threading

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native

sys.path.insert(0, str(native.GREETER_KIND_PY))
sys.path.insert(0, str(native.REPO / "examples" / "python_provider" / "py_greeter"))

import py_greeter  # noqa: E402
from greeter_kind import COUNTER, GREETER  # noqa: E402

from guatiao import BadValue, Internal, Unsupported, Value, WrongKind, kinds, provide  # noqa: E402
from guatiao.registry import Registry  # noqa: E402

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]


class Ear:
    def __init__(self) -> None:
        self.heard_so_far: list = []

    def heard(self, what: str) -> None:
        self.heard_so_far.append(what)


@pytest.fixture(scope="module")
def c_greeter_dir(tmp_path_factory):
    return native.build_c_greeter(tmp_path_factory.mktemp("c-greeter"))


# Each greeter, with the greeting it answers "ana" with.
_GREETERS = {
    "rust": ("derived_greeter_hello", "hello, ana"),
    "c": ("c_greeter", "hello from C, ana"),
    "python": ("py_greeter_hello", "hello, ana"),
}


@pytest.fixture(params=sorted(_GREETERS))
def greeter(request, c_greeter_dir):
    which = request.param
    key, greeting = _GREETERS[which]
    with Registry("py-provide", "0.1") as reg:
        if which == "rust":
            reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        elif which == "c":
            if c_greeter_dir is None:
                pytest.skip("no C compiler: the greeter written in C is not built")
            reg.scan_dir(str(c_greeter_dir), rules="kind=greeter")
        else:
            reg.register_library(py_greeter.library)
        yield which, reg.bind(key, GREETER), greeting


def test_it_greets(greeter):
    _, remote, greeting = greeter
    with remote.greet("ana") as answer:
        assert answer.to_python() == {"greeting": greeting}


def test_it_refuses_with_its_own_words(greeter):
    _, remote, _ = greeter
    with pytest.raises(BadValue) as refused:
        remote.greet("")
    assert "nobody to greet" in refused.value.message


def test_a_failure_inside_the_method_crosses_as_internal(greeter):
    which, remote, _ = greeter
    if which == "c":
        pytest.skip("the C greeter has no failure to provoke")
    with pytest.raises(Internal):
        remote.greet("panic")


def test_it_fills_a_lent_map_in_place(greeter):
    _, remote, _ = greeter
    with Value({"name": "bo"}) as config:
        assert remote.check(config) is True
        filled = config.to_python()
    assert filled["name"] == "bo"
    assert len(filled) == 2, f"one entry added: {filled}"


def test_it_greets_from_a_map_handed_over(greeter):
    _, remote, greeting = greeter
    config = Value({"name": "ana"})
    with remote.greet_from(config) as answer:
        assert answer.to_python() == {"greeting": greeting}
    assert config.is_absent(), "the map went with the call"


def test_it_tells_a_lent_listener(greeter):
    _, remote, greeting = greeter
    ear = Ear()
    remote.greet_into("ana", ear)
    assert ear.heard_so_far == [greeting]


def test_a_conversation_it_starts_tells_the_listener_handed_in(greeter):
    which, remote, _ = greeter
    before = kinds.live_objects()
    ear = Ear()
    if which == "c":
        assert not remote.has("start")
        with pytest.raises(Unsupported):
            remote.start(ear)
        return
    with remote.start(ear) as conversation:
        conversation.say("hi")
        conversation.say("again")
        assert conversation.turns() == 2
        transcript = bytearray(16)
        n = conversation.read(transcript)
        assert bytes(transcript[:n]) == b"hi\nagain"
        with pytest.raises(BadValue):
            conversation.say("")
    assert ear.heard_so_far == ["hi", "again"]
    assert kinds.live_objects() == before


# ---- what only a Python library does here ---------------------------------


def test_an_instance_is_built_from_a_configuration_the_registry_checked():
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(py_greeter.library)
        with pytest.raises(BadValue) as refused:
            reg.create("py_greeter_shouter", {"prefx": "hey"})
        assert "prefx" in refused.value.message
        assert py_greeter.library.live_instances() == 0, "the class never ran"
        with reg.create("py_greeter_shouter", {"prefix": "hey"}) as instance:
            assert py_greeter.library.live_instances() == 1
            assert instance.bind(GREETER).greet("cy").to_python() == {"greeting": "hey CY"}
            assert not instance.bind(GREETER).has("shout"), "Shouter has no shout of its own"
        assert py_greeter.library.live_instances() == 0


def test_one_provider_serves_two_kinds():
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(py_greeter.library)
        provider = reg.provider("py_greeter_hello")
        assert provider["kinds"] == ["greeter", "counter"]
        assert provider["display_name"] == "Hello, in Python"
        counter = reg.bind("py_greeter_hello", COUNTER)
        first = counter.count()
        assert counter.count() == first + 1


def test_calls_from_many_threads_each_take_the_gil():
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(py_greeter.library)
        counter = reg.bind("py_greeter_hello", COUNTER)
        start = counter.count()
        threads = [threading.Thread(target=lambda: [counter.count() for _ in range(200)]) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert counter.count() == start + 801


def test_a_linked_python_library_is_retired_not_unloaded():
    before = kinds.live_objects()
    lib = provide.library(
        "py_short_lived",
        "0.1.0",
        providers=[provide.Provider("hello", py_greeter.Hello, kinds=[GREETER])],
    )
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(lib)
        assert kinds.live_objects() == before + 1, "its one instance"
        with pytest.raises(WrongKind):
            reg.unload("py_short_lived")
        reg.retire("py_short_lived")
        assert "py_short_lived_hello" not in {p["id"] for p in reg.providers("greeter")}
        lib.close()
        assert kinds.live_objects() == before
        lib.close()
        # A closed library declines a host that asks again.
        assert reg.register_library(lib) == {"skipped": "declined-this-host"}


def test_an_unavailable_provider_says_why():
    reasons = iter([None, "the device is gone"])
    lib = provide.library(
        "py_flaky",
        "0.1.0",
        providers=[
            provide.Provider("hello", py_greeter.Hello, kinds=[GREETER], available=lambda: next(reasons))
        ],
    )
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(lib)
        assert reg.provider_available("py_flaky_hello") == (True, None)
        assert reg.provider_available("py_flaky_hello") == (False, "the device is gone")
    lib.close()


def test_a_provider_must_serve_provider_kinds():
    from greeter_kind import LISTENER

    with pytest.raises(TypeError):
        provide.Provider("x", py_greeter.Hello, kinds=[LISTENER])
    with pytest.raises(TypeError):
        provide.Provider("x", py_greeter.Hello(), kinds=[GREETER])
    with pytest.raises(ValueError):
        provide.library(
            "dup",
            "1",
            providers=[
                provide.Provider("a", py_greeter.Hello, kinds=[GREETER]),
                provide.Provider("b", py_greeter.Hello, kinds=[GREETER], id="dup_a"),
            ],
        )
