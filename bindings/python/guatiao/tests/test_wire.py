# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The wire encoding and channels against the shared vectors every
implementation reads: each value both ways, each malformed input refused
at its byte, the frames the builders write, and a conversation with every
refusal."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from guatiao import BadValue, NotFound, Value, WrongKind, wire
from guatiao._abi import Tag

_VECTORS = json.loads(
    (
        Path(__file__).resolve().parents[4]
        / "crates"
        / "guatiao"
        / "tests"
        / "vectors"
        / "wire.json"
    ).read_text(encoding="utf-8")
)

_TAGS = {
    Tag.ABSENT: "absent",
    Tag.NULL: "null",
    Tag.BOOL: "bool",
    Tag.NUMBER: "number",
    Tag.STRING: "string",
    Tag.BYTES: "bytes",
    Tag.LIST: "list",
    Tag.MAP: "map",
}


def describe(ref):
    """A node as the vector file writes a tree."""
    tag = _TAGS[ref.tag]
    if tag in ("absent", "null"):
        return [tag]
    if tag == "bool":
        return [tag, ref.bool_or()]
    if tag == "number":
        return [tag, ref.to_python(numbers="str")]
    if tag == "string":
        return [tag, ref.str_or()]
    if tag == "bytes":
        return [tag, ref.to_python().hex()]
    if tag == "list":
        return [tag, [describe(item) for item in ref.as_list()]]
    return [tag, [[key, describe(item)] for key, item in ref.as_map().items()]]


def build(tree):
    """The value a vector tree names."""
    tag, *rest = tree
    if tag == "absent":
        return Value.absent()
    if tag == "null":
        return Value.null()
    if tag == "bool":
        return Value.bool(rest[0])
    if tag == "number":
        return Value.number(rest[0])
    if tag == "string":
        return Value.string(rest[0])
    if tag == "bytes":
        return Value.bytes_(bytes.fromhex(rest[0]))
    if tag == "list":
        value = Value.list()
        for item in rest[0]:
            with build(item) as child:
                value.as_list().append(child)
        return value
    value = Value.map()
    for key, item in rest[0]:
        with build(item) as child:
            value.as_map()[key] = child
    return value


@pytest.mark.parametrize("case", _VECTORS["values"], ids=lambda c: c["name"])
def test_every_value_round_trips(case):
    data = bytes.fromhex(case["hex"])
    with Value.from_wire(data) as value:
        assert describe(value) == case["tree"]
        assert value.to_wire() == data
    with build(case["tree"]) as built:
        assert describe(built) == case["tree"]
        assert built.to_wire() == data


@pytest.mark.parametrize("case", _VECTORS["malformed"], ids=lambda c: c["name"])
def test_every_malformed_input_is_refused(case):
    with pytest.raises(BadValue):
        Value.from_wire(bytes.fromhex(case["hex"]))


def test_a_child_encodes_alone():
    with Value({"outer": {"inner": [1, "two"]}}) as value:
        child = value.as_map()["outer"]
        with Value.from_wire(child.to_wire()) as back:
            assert back.to_python() == {"inner": [1, "two"]}


@pytest.mark.parametrize("case", _VECTORS["frames"], ids=lambda c: c["build"])
def test_the_frame_builders(case):
    expected = bytes.fromhex(case["hex"])
    if case["build"] == "close":
        assert wire.close(case["channel"]) == expected
        return
    builder = wire.announce if case["build"] == "announce" else wire.send
    with build(case["tree"]) as tree:
        assert builder(case["channel"], tree) == expected


def test_a_builder_takes_plain_python():
    assert wire.send(7, {"greeting": "hello"}) == bytes.fromhex(_VECTORS["frames"][1]["hex"])


_REFUSALS = {
    "malformed": (wire.Malformed, BadValue),
    "no_schema": (wire.NoSchema, NotFound),
    "not_a_schema": (wire.NotASchema, WrongKind),
    "invalid": (wire.Invalid, BadValue),
}


def test_the_conversation():
    with wire.Receiver() as receiver:
        for step in _VECTORS["conversation"]:
            frame = bytes.fromhex(step["frame"])
            channel = step.get("channel", 0)
            if "event" in step:
                event = receiver.accept(frame)
                assert event.kind.name.lower() == step["event"], step
                assert event.channel == channel
                if "tree" in step:
                    with event.value as value:
                        assert describe(value) == step["tree"]
                else:
                    assert event.value is None
                continue
            refused, status_class = _REFUSALS[step["refusal"]]
            with pytest.raises(refused) as excinfo:
                receiver.accept(frame)
            error = excinfo.value
            assert isinstance(error, status_class), step
            assert error.channel == channel, step
            assert error.message, step
            assert "hunter2" not in str(error)
            if "kind" in step:
                assert error.kind == step["kind"]
            if "at" in step:
                assert error.at == step["at"], step
            if "key" in step:
                assert error.key == step["key"], step


def test_a_closed_receiver_refuses():
    receiver = wire.Receiver()
    receiver.close()
    receiver.close()
    with pytest.raises(ValueError):
        receiver.accept(wire.close(1))


def test_the_constants_are_the_headers():
    header = (
        Path(__file__).resolve().parents[4] / "crates" / "guatiao" / "include" / "guatiao.h"
    ).read_text(encoding="utf-8")
    for frame in wire.Frame:
        assert f"#define GUATIAO_FRAME_{frame.name} {frame.value}\n" in header
    for name, code in [
        ("ACCEPTED", wire._ACCEPTED),
        ("MALFORMED", wire._MALFORMED),
        ("NO_SCHEMA", wire._NO_SCHEMA),
        ("NOT_A_SCHEMA", wire._NOT_A_SCHEMA),
        ("INVALID", wire._INVALID),
    ]:
        assert f"#define GUATIAO_WIRE_{name} {code}\n" in header
