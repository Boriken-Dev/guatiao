# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao.provide`: a greeter written in Python passes the behaviour the
Rust and C greeters pass, called from Python through the same registry;
and what a provider class and a module library are checked for."""

from __future__ import annotations

import os
import sys
import threading
import types
from dataclasses import dataclass
from typing import Any, Dict, List, Optional

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
from greeter_kind import Conversation, Counter, Greeter, Listener  # noqa: E402

from guatiao import BadValue, Internal, Unsupported, Value, WrongKind, kinds, provide  # noqa: E402
from guatiao.registry import Registry  # noqa: E402

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]


class Ear:
    def __init__(self) -> None:
        self.heard_so_far: List[str] = []

    def heard(self, what: str) -> None:
        self.heard_so_far.append(what)


@pytest.fixture(scope="module")
def c_greeter_dir(tmp_path_factory: Any) -> Any:
    return native.build_c_greeter(tmp_path_factory.mktemp("c-greeter"))


# Each greeter, with the greeting it answers "ana" with.
_GREETERS = {
    "rust": ("derived_greeter_hello", "hello, ana"),
    "c": ("c_greeter", "hello from C, ana"),
    "python": ("py_greeter_hello", "hello, ana"),
}


@pytest.fixture(params=sorted(_GREETERS))
def greeter(request: Any, c_greeter_dir: Any) -> Any:
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
            reg.register_library(py_greeter)
        yield which, reg.bind(key, Greeter), greeting


def test_it_greets(greeter: Any) -> None:
    _, remote, greeting = greeter
    with remote.greet("ana") as answer:
        assert answer.to_python() == {"greeting": greeting}


def test_it_refuses_with_its_own_words(greeter: Any) -> None:
    _, remote, _ = greeter
    with pytest.raises(BadValue) as refused:
        remote.greet("")
    assert "nobody to greet" in refused.value.message


def test_a_failure_inside_the_method_crosses_as_internal(greeter: Any) -> None:
    which, remote, _ = greeter
    if which == "c":
        pytest.skip("the C greeter has no failure to provoke")
    with pytest.raises(Internal):
        remote.greet("panic")


def test_it_fills_a_lent_map_in_place(greeter: Any) -> None:
    _, remote, _ = greeter
    with Value({"name": "bo"}) as config:
        assert remote.check(config) is True
        filled = config.to_python()
    assert filled["name"] == "bo"
    assert len(filled) == 2, f"one entry added: {filled}"


def test_it_greets_from_a_map_handed_over(greeter: Any) -> None:
    _, remote, greeting = greeter
    config = Value({"name": "ana"})
    with remote.greet_from(config) as answer:
        assert answer.to_python() == {"greeting": greeting}
    assert config.is_absent(), "the map went with the call"


def test_it_tells_a_lent_listener(greeter: Any) -> None:
    _, remote, greeting = greeter
    ear = Ear()
    remote.greet_into("ana", ear)
    assert ear.heard_so_far == [greeting]


def test_a_conversation_it_starts_tells_the_listener_handed_in(greeter: Any) -> None:
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


# ---- a module is a library --------------------------------------------------


def test_a_module_is_its_library() -> None:
    library = provide.library_of(py_greeter)
    assert library is provide.library_of(py_greeter), "built once per module"
    assert (library.id, library.version) == ("py_greeter", py_greeter.__version__)
    assert library.providers == (py_greeter.Hello, py_greeter.Shouter)
    assert library.kinds == ("greeter", "counter")


def _module(name: str, **attrs: Any) -> types.ModuleType:
    module = types.ModuleType(name)
    for key, value in attrs.items():
        setattr(module, key, value)
    return module


class Plain(provide.Provider, Counter):
    def count(self) -> int:
        return 1


def test_what_a_module_must_say_is_named() -> None:
    with pytest.raises(TypeError, match="__main__"):
        provide.library_of(_module("__main__", __version__="1", __all__=["Plain"], Plain=Plain))
    with pytest.raises(TypeError, match="__version__"):
        provide.library_of(_module("no_version", __all__=["Plain"], Plain=Plain))
    with pytest.raises(TypeError, match="exports no"):
        provide.library_of(_module("nothing", __version__="1", __all__=["x"], x=3))
    with pytest.raises(TypeError, match="_meta_"):
        provide.library_of(_module("bad_meta", __version__="1", __all__=["Plain"], Plain=Plain, _meta_={1: object()}))
    # Without __all__, only what the module itself defines is served.
    imported_only = _module("imports_only", __version__="1", Plain=Plain)
    with pytest.raises(TypeError, match="exports no"):
        provide.library_of(imported_only)


def test_a_module_library_says_its_meta() -> None:
    module = _module(
        "with_meta",
        __doc__="A tally of things.\n\nMore words.",
        __version__="2.0",
        __author__="Ana",
        __license__="MPL-2.0",
        __all__=["Plain"],
        Plain=Plain,
        _meta_={"license": "MIT", "build": 7},
    )
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(module)
        (library,) = [lib for lib in reg.libraries() if lib["id"] == "with_meta"]
        assert library["version"] == "2.0"
    from guatiao._module_meta import meta_of

    assert meta_of(module) == {
        "description": "A tally of things.",
        "authors": ["Ana"],
        "license": "MIT",
        "build": 7,
    }


# ---- what a provider class is checked for ------------------------------------


def test_a_provider_class_is_checked_when_it_is_written() -> None:
    with pytest.raises(TypeError, match="requires"):

        class Silent(provide.Provider, Greeter):
            pass

    with pytest.raises(TypeError, match="no kind"):

        class Kindless(provide.Provider):
            pass

    with pytest.raises(TypeError, match="object kind"):

        class Ear2(provide.Provider, Listener):
            def heard(self, what: str) -> None: ...

    with pytest.raises(TypeError, match="dataclass or a TypedDict"):

        class Untyped(provide.Provider, Counter):
            def __init__(self, config: Dict[str, Any]) -> None: ...

            def count(self) -> int:
                return 0

    assert Plain.__guatiao_provider__.name == "plain"


@dataclass(frozen=True)
class Limits:
    low: int
    high: int = 10
    label: Optional[str] = None


class Bounded(provide.Provider, Counter, named="bounded", display_name="Bounded"):
    def __init__(self, config: Limits) -> None:
        self.limits = config
        self.at = config.low

    def count(self) -> int:
        self.at += 1
        return min(self.at, self.limits.high)


def test_an_instance_receives_the_dataclass_its_schema_checked() -> None:
    library = provide.library("py_bounded", "0.1.0", providers=[Bounded])
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(library)
        assert reg.provider_config("py_bounded_bounded").to_python()["required"] == ["low"]
        with pytest.raises(BadValue):
            reg.create("py_bounded_bounded", {"low": "one"})
        assert library.live_instances() == 0, "the class never ran"
        with reg.create("py_bounded_bounded", {"low": 8, "high": 9}) as instance:
            counter = instance.bind(Counter)
            assert [counter.count(), counter.count()] == [9, 9]
            assert library.live_instances() == 1
        assert library.live_instances() == 0


def test_a_create_with_nowhere_to_put_the_instance_builds_nothing() -> None:
    import ctypes

    from guatiao import _abi

    library = provide.library("py_nowhere", "0.1.0", providers=[Bounded])
    info = _abi.LibraryInfo.from_address(library.info_address)
    provider = info.providers.ptr[0]
    with Value({"low": 1}) as config:
        err = _abi.ProviderError()
        status = provider.create(provider.ctx, ctypes.pointer(config._raw), None, ctypes.byref(err))
    assert status == int(_abi.Status.ERR_NULL)
    assert library.live_instances() == 0
    library.close()


def test_one_provider_serves_two_kinds() -> None:
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(py_greeter)
        provider = reg.provider("py_greeter_hello")
        assert provider["kinds"] == ["greeter", "counter"]
        assert provider["display_name"] == "Hello, in Python"
        counter = reg.bind("py_greeter_hello", Counter)
        first = counter.count()
        assert counter.count() == first + 1


def test_calls_from_many_threads_each_take_the_gil() -> None:
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(py_greeter)
        counter = reg.bind("py_greeter_hello", Counter)
        start = counter.count()
        threads = [threading.Thread(target=lambda: [counter.count() for _ in range(200)]) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert counter.count() == start + 801


def test_a_linked_python_library_is_retired_not_unloaded() -> None:
    before = kinds.live_objects()
    lib = provide.library("py_short_lived", "0.1.0", providers=[py_greeter.Hello])
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


class Flaky(provide.Provider, Counter):
    reasons: List[Optional[str]] = [None, "the device is gone"]

    @classmethod
    def available(cls) -> Optional[str]:
        return cls.reasons.pop(0)

    def count(self) -> int:
        return 0


def test_an_unavailable_provider_says_why() -> None:
    lib = provide.library("py_flaky", "0.1.0", providers=[Flaky])
    with Registry("py-provide", "0.1") as reg:
        reg.register_library(lib)
        assert reg.provider_available("py_flaky_flaky") == (True, None)
        assert reg.provider_available("py_flaky_flaky") == (False, "the device is gone")
    lib.close()


def test_two_providers_on_one_id_are_refused() -> None:
    class Again(provide.Provider, Counter, named="plain"):
        def count(self) -> int:
            return 2

    with pytest.raises(ValueError):
        provide.library("dup", "1", providers=[Plain, Again])
    with pytest.raises(TypeError):
        provide.library("dup", "1", providers=[Ear])  # type: ignore[list-item]


def test_the_conversation_kind_is_served_by_inheritance_or_duck_typing() -> None:
    assert Conversation.kind.implemented(py_greeter.Chat, "say")
    assert not Conversation.kind.implemented(Ear, "say")
    assert Listener.kind.implemented(Ear, "heard")
