# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The struct sizes are the ABI; a wrong one is silent corruption.

Also reads every `GUATIAO_*` tag and status constant straight out of the
committed header, so the Python enums cannot drift from it unnoticed.
"""

from __future__ import annotations

import ctypes
import re
from pathlib import Path

from guatiao import _abi

_HEADER = (
    Path(__file__).resolve().parents[3]
    / "crates"
    / "guatiao"
    / "include"
    / "guatiao.h"
)

_ENUM_MEMBER = re.compile(r"^\s*GUATIAO_([A-Z_0-9]+) = (\d+),", re.MULTILINE)


def _header_constants() -> dict[str, int]:
    text = _HEADER.read_text(encoding="utf-8")
    return {name: int(value) for name, value in _ENUM_MEMBER.findall(text)}


def test_sizes():
    assert ctypes.sizeof(_abi.Str) == 16
    assert ctypes.sizeof(_abi.Bytes) == 16
    assert ctypes.sizeof(_abi.Values) == 16
    assert ctypes.sizeof(_abi.Entries) == 16
    assert ctypes.sizeof(_abi.String) == 32
    assert ctypes.sizeof(_abi.Buffer) == 32
    assert ctypes.sizeof(_abi.List) == 32
    assert ctypes.sizeof(_abi.Map) == 32
    assert ctypes.sizeof(_abi.Payload) == 32
    assert ctypes.sizeof(_abi.Value) == 40
    assert ctypes.sizeof(_abi.Entry) == 72


def test_offsets():
    assert _abi.Value.tag.offset == 0
    assert _abi.Value._pad.offset == 4
    assert _abi.Value.payload.offset == 8
    assert _abi.Entry.key.offset == 0
    assert _abi.Entry.value.offset == 32


def test_tags_match_header():
    constants = _header_constants()
    for member in _abi.Tag:
        assert constants[member.name] == member.value


def test_status_codes_match_header():
    constants = _header_constants()
    for member in _abi.Status:
        assert constants[member.name] == member.value


def test_header_declares_no_tag_or_status_this_binding_is_missing():
    constants = _header_constants()
    tag_names = {f"GUATIAO_{m.name}" for m in _abi.Tag}
    status_names = {f"GUATIAO_{m.name}" for m in _abi.Status}
    for name in constants:
        full = f"GUATIAO_{name}"
        if full.startswith("GUATIAO_ERR_") or full == "GUATIAO_OK":
            assert full in status_names, full
        elif full in (
            "GUATIAO_ABSENT",
            "GUATIAO_NULL",
            "GUATIAO_BOOL",
            "GUATIAO_NUMBER",
            "GUATIAO_STRING",
            "GUATIAO_BYTES",
            "GUATIAO_LIST",
            "GUATIAO_MAP",
        ):
            assert full in tag_names, full
