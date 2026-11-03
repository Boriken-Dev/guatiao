# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A secret in Python: built in the wiping allocator, never shown by
`repr`, read as a view over the tree's own memory, and answered by a slot
its kind marks `@sensitive` as a `Value` kept there rather than a `str`.

That the wiping allocator overwrites what it frees is the library's, and
is proven by its own tests; these check that Python puts a secret there
and keeps it there."""

from __future__ import annotations

import ctypes
import threading
from typing import Protocol

import pytest

from guatiao import Map, Value, _abi, _lib, wiping_alloc
from guatiao.kinds import (
    KindProtocol,
    Lent,
    Taken,
    SensitiveText,
    ValueAnswer,
    _description,
    _serve,
    describe_kinds,
    fallible,
    generate,
    kinds_of,
    required,
    sensitive,
)

SECRET = "hunter2-SECRET"


def _lives_in_wiping(node: "_abi.Value") -> bool:
    arm = {_abi.Tag.STRING: "text", _abi.Tag.MAP: "map", _abi.Tag.BYTES: "bytes"}[node.tag]
    return bool(_lib.core().guatiao_alloc_is_wiping(getattr(node.payload, arm).alloc))


def test_the_wiping_allocator_is_the_librarys_and_the_default_is_not():
    core = _lib.core()
    assert core.guatiao_alloc_is_wiping(ctypes.byref(wiping_alloc()))
    assert not core.guatiao_alloc_is_wiping(ctypes.byref(core.alloc))
    assert wiping_alloc() is wiping_alloc(), "filled once, kept for every tree built there"


def test_a_value_built_in_the_wiping_allocator_is_never_shown():
    secret = Value({"user": "alice", "password": SECRET}, alloc=wiping_alloc())
    assert secret.is_sensitive
    assert _lives_in_wiping(secret._raw)
    shown = [repr(secret), str(secret), repr(secret.as_map()["password"]), repr(secret.clone())]
    for text in shown:
        assert "hunter2" not in text, text
    assert repr(secret) == "<guatiao.Value [sensitive]>"
    assert secret.clone().is_sensitive, "a copy stays in the wiping allocator"
    plain = Value({"user": "alice"})
    assert not plain.is_sensitive
    assert "alice" in repr(plain)


def test_a_secret_is_read_where_it_lives_and_the_view_ends_with_the_block():
    secret = Value(SECRET, alloc=wiping_alloc())
    with secret.reveal() as view:
        assert isinstance(view, memoryview)
        assert view.readonly
        assert view.tobytes() == SECRET.encode()
        assert ctypes.addressof(view.obj) == secret._raw.payload.text.ptr, "no copy was made"
    with pytest.raises(ValueError):
        view.tobytes()
    with Value(b"\x00\x01", alloc=wiping_alloc()).reveal() as raw:
        assert raw.tobytes() == b"\x00\x01"
    with pytest.raises(TypeError):
        with Value({"a": 1}).reveal():
            pass


class Vault(KindProtocol, Protocol, kind="py_vault"):
    """A store of secrets."""

    @required
    @fallible
    @sensitive
    def read(self, key: str) -> ValueAnswer: ...

    @required
    @sensitive
    def pin(self) -> SensitiveText: ...

    @required
    def name(self) -> str: ...

    @required
    def touch(self, map: Lent[Map]) -> None: ...

    @required
    def keep(self, value: Taken[Value]) -> None: ...


class PlainVault(KindProtocol, Protocol, kind="py_plain_vault"):
    """The same slots, none marked."""

    @required
    @fallible
    def read(self, key: str) -> ValueAnswer: ...

    @required
    def pin(self) -> str: ...

    @required
    def name(self) -> str: ...

    @required
    def touch(self, map: Lent[Map]) -> None: ...

    @required
    def keep(self, value: Taken[Value]) -> None: ...


class Keeper:
    # Whether `touch` keeps a slice of the lent map's token, and the slice.
    peek = False
    kept: "memoryview | None" = None

    def read(self, key: str) -> ValueAnswer:
        return {"key": key, "secret": SECRET}

    def pin(self) -> str:
        return SECRET

    def name(self) -> str:
        return "keeper"

    def touch(self, map: Map) -> None:
        if self.peek:
            with map["token"].reveal() as view:
                self.kept = view[:]

    def keep(self, value: Value) -> None:
        value.close()


def test_the_mark_is_not_in_the_floor_hash():
    assert Vault.kind.floor_hash == PlainVault.kind.floor_hash


def test_a_sensitive_slot_answers_a_value_kept_in_the_wiping_allocator():
    raw = _serve.serve(Vault.kind, Keeper())
    try:
        vault = Vault.kind.bind(raw.table, raw.size, raw.ctx)
        answer = vault.read("k")
        assert isinstance(answer, Value) and answer.is_sensitive
        assert _lives_in_wiping(answer._raw)
        assert "hunter2" not in repr(answer)
        with answer.as_map()["secret"].reveal() as view:
            assert view.tobytes() == SECRET.encode()
        pin = vault.pin()
        assert isinstance(pin, Value), "a sensitive text answer is not a str"
        assert _lives_in_wiping(pin._raw)
        with pin.reveal() as view:
            assert view.tobytes() == SECRET.encode()
        assert vault.name() == "keeper"
        answer.close()
        pin.close()
    finally:
        _serve.release(raw.ctx)

    raw = _serve.serve(PlainVault.kind, Keeper())
    try:
        plain = PlainVault.kind.bind(raw.table, raw.size, raw.ctx)
        assert plain.pin() == SECRET
        assert not plain.read("k").is_sensitive
    finally:
        _serve.release(raw.ctx)


def test_the_description_says_which_slots_are_sensitive_and_reads_back():
    document = describe_kinds(Vault)
    slots = {s["name"]: s for s in document["kinds"][0]["slots"]}
    assert slots["read"].get("sensitive") is True
    assert slots["pin"].get("sensitive") is True
    assert "sensitive" not in slots["name"]
    read = kinds_of(document)["py_vault"]
    assert [m.sensitive for m in read.methods] == [True, True, False, False, False]
    assert read.floor_hash == Vault.kind.floor_hash
    module = generate(document)
    assert "    @sensitive\n    def pin(self) -> SensitiveText" in module
    assert "sensitive" in module.split("from guatiao.kinds import ", 1)[1].splitlines()[0]
    assert "sensitive" not in str(describe_kinds(PlainVault))
    assert _description.slot_description(read.methods[2]) == slots["name"]


class Watching:
    """The library's default allocator, wrapped to count the blocks freed
    while still holding the secret."""

    def __init__(self) -> None:
        base = _lib.core().alloc
        self.held = 0
        alloc_type = _abi.Alloc._fields_[2][1]
        free_type = _abi.Alloc._fields_[3][1]
        release_type = _abi.Alloc._fields_[4][1]

        def _alloc(_ctx, size, align):
            return base.alloc(base.ctx, size, align)

        def _free(_ctx, ptr, size, align):
            if ptr and SECRET.encode() in ctypes.string_at(ptr, size):
                self.held += 1
            base.free(base.ctx, ptr, size, align)

        self._callbacks = (alloc_type(_alloc), free_type(_free))
        self.alloc = _abi.Alloc(ctypes.sizeof(_abi.Alloc), None, self._callbacks[0], self._callbacks[1], release_type())


def test_wipe_overwrites_a_tree_in_an_allocator_that_does_not():
    control = Watching()
    Value({"password": SECRET, SECRET: b"x"}, alloc=control.alloc).close()
    assert control.held >= 2, "the watcher sees a secret freed as it was"

    watching = Watching()
    secret = Value({"password": SECRET, SECRET: b"x"}, alloc=watching.alloc)
    secret.wipe()
    assert watching.held == 0
    assert secret.is_null()
    secret.close()


def test_a_secret_copied_into_a_plain_tree_stays_in_the_wiping_allocator():
    secret = Value(SECRET, alloc=wiping_alloc())
    plain = Value({"user": "alice"})
    plain.as_map()["token"] = secret
    items = Value(["a"])
    items.as_list()[0] = secret
    built = Value(secret)
    for holder in (plain, items, built):
        assert holder.is_sensitive, repr(holder)
        assert "hunter2" not in repr(holder)
    with plain.as_map()["token"].reveal() as view:
        assert view.tobytes() == SECRET.encode()
    for v in (plain, items, built, secret):
        v.close()


def test_a_revealed_view_keeps_its_bytes_and_refuses_what_would_free_them():
    v = Value({"token": SECRET, "user": "alice"}, alloc=wiping_alloc())
    with v.as_map()["token"].reveal() as view:
        kept = view[0:14]
        with pytest.raises(BufferError):
            v.close()
        with pytest.raises(BufferError):
            v.wipe()
        with pytest.raises(BufferError):
            v.as_map()["more"] = "x"
        with pytest.raises(BufferError):
            v.as_map()["user"] = "bob"
    # A slice outlives the block: the block stays, and the tree with it.
    with pytest.raises(BufferError):
        v.close()
    assert bytes(kept) == SECRET.encode()
    del kept
    v.as_map()["user"] = "bob"
    v.close()


def test_a_slice_keeps_the_tree_alive_after_its_value_is_dropped():
    def a_slice() -> memoryview:
        v = Value(SECRET, alloc=wiping_alloc())
        with v.reveal() as view:
            return view[0:7]

    kept = a_slice()
    import gc

    gc.collect()
    assert bytes(kept) == SECRET.encode()[0:7]


def test_a_secret_in_a_lent_map_stays_in_the_wiping_allocator():
    raw = _serve.serve(PlainVault.kind, Keeper())
    try:
        vault = PlainVault.kind.bind(raw.table, raw.size, raw.ctx)
        plain = Value({"user": "alice"})
        plain.as_map()["token"] = Value(SECRET, alloc=wiping_alloc())
        vault.touch(plain.as_map())
        assert plain.is_sensitive
        assert _lives_in_wiping(plain.as_map()["token"]._require_node())
        assert "hunter2" not in repr(plain)
        plain.close()
    finally:
        _serve.release(raw.ctx)


def test_a_value_is_not_handed_over_or_lent_while_a_slice_of_its_reveal_lives():
    raw = _serve.serve(PlainVault.kind, Keeper())
    try:
        vault = PlainVault.kind.bind(raw.table, raw.size, raw.ctx)
        secret = Value(SECRET, alloc=wiping_alloc())
        with secret.reveal() as view:
            kept = view[:]
        with pytest.raises(BufferError):
            vault.keep(secret)
        plain = Value({"user": "alice"})
        plain.as_map()["token"] = Value(SECRET, alloc=wiping_alloc())
        with plain.as_map()["token"].reveal() as view:
            lent = view[:]
        with pytest.raises(BufferError):
            vault.touch(plain.as_map())
        assert kept.tobytes() == SECRET.encode()
        assert lent.tobytes() == SECRET.encode()
        kept.release()
        lent.release()
        del kept, lent
        vault.keep(secret)
        assert secret.is_absent()
        vault.touch(plain.as_map())
        plain.close()
    finally:
        _serve.release(raw.ctx)


def test_a_slice_a_provider_keeps_of_a_lent_argument_reads_its_own_copy():
    keeper = Keeper()
    keeper.peek = True
    raw = _serve.serve(PlainVault.kind, keeper)
    try:
        vault = PlainVault.kind.bind(raw.table, raw.size, raw.ctx)
        plain = Value({"user": "alice"})
        plain.as_map()["token"] = Value(SECRET, alloc=wiping_alloc())
        vault.touch(plain.as_map())
        # The caller's tree goes; what the provider kept is its own.
        plain.close()
        assert keeper.kept is not None
        assert keeper.kept.tobytes() == SECRET.encode()
        keeper.kept.release()
        keeper.kept = None
    finally:
        _serve.release(raw.ctx)


@pytest.mark.parametrize("ending", ["guatiao_value_free", "guatiao_value_wipe"])
def test_a_reveal_on_another_thread_while_a_value_ends_is_refused(monkeypatch, ending):
    # The other thread reveals at the one moment that matters: after the
    # value was checked, before its tree is freed. ctypes lets go of the GIL
    # around the call, so this is when another thread can run.
    core = _lib.core()
    real = getattr(core, ending)
    secret = Value(SECRET, alloc=wiping_alloc())
    seen: "list[object]" = []

    def reveal() -> None:
        try:
            with secret.reveal() as view:
                seen.append(view.tobytes())
        except (BufferError, ValueError) as e:
            seen.append(type(e))

    def racing(ptr: object) -> int:
        if not seen:
            other = threading.Thread(target=reveal)
            other.start()
            other.join()
        return int(real(ptr))

    monkeypatch.setattr(core, ending, racing)
    if ending == "guatiao_value_free":
        secret.close()
    else:
        secret.wipe()
    assert seen == [BufferError]
