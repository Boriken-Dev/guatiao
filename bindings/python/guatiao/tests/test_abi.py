# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The struct sizes are the ABI; a wrong one is silent corruption.

Every size is stated in words, and every struct is compared with the
layout this platform's C compiler gives the committed header. Also reads
every `GUATIAO_*` tag and status constant straight out of the header, so
the Python enums cannot drift from it unnoticed.
"""

from __future__ import annotations

import ctypes
import re
from pathlib import Path

import pytest

from guatiao import _abi

import native

_HEADER = (
    Path(__file__).resolve().parents[4]
    / "crates"
    / "guatiao"
    / "include"
    / "guatiao.h"
)

_ENUM_MEMBER = re.compile(r"^\s*GUATIAO_([A-Z_0-9]+) = (\d+),", re.MULTILINE)


def _header_constants() -> dict[str, int]:
    text = _HEADER.read_text(encoding="utf-8")
    return {name: int(value) for name, value in _ENUM_MEMBER.findall(text)}


_W = ctypes.sizeof(ctypes.c_void_p)


def test_sizes():
    assert _abi.WORD == _W
    assert ctypes.sizeof(_abi.Str) == 2 * _W
    assert ctypes.sizeof(_abi.Bytes) == 2 * _W
    assert ctypes.sizeof(_abi.Values) == 2 * _W
    assert ctypes.sizeof(_abi.Entries) == 2 * _W
    assert ctypes.sizeof(_abi.String) == 4 * _W
    assert ctypes.sizeof(_abi.Buffer) == 4 * _W
    assert ctypes.sizeof(_abi.List) == 4 * _W
    assert ctypes.sizeof(_abi.Map) == 4 * _W
    assert ctypes.sizeof(_abi.Payload) == 4 * _W
    assert ctypes.sizeof(_abi.Value) == 8 + 4 * _W
    assert ctypes.sizeof(_abi.Entry) == 8 + 8 * _W


def test_offsets():
    assert _abi.Value.tag.offset == 0
    assert _abi.Value._pad.offset == 4
    assert _abi.Value.payload.offset == 8
    assert _abi.Entry.key.offset == 0
    assert _abi.Entry.value.offset == 4 * _W


# Each ctypes mirror and the header struct it stands for.
_MIRRORS = {
    "guatiao_str": _abi.Str,
    "guatiao_bytes": _abi.Bytes,
    "guatiao_bytes_mut": _abi.BytesMut,
    "guatiao_values": _abi.Values,
    "guatiao_entries": _abi.Entries,
    "guatiao_string": _abi.String,
    "guatiao_buffer": _abi.Buffer,
    "guatiao_list": _abi.List,
    "guatiao_map": _abi.Map,
    "guatiao_payload": _abi.Payload,
    "guatiao_value": _abi.Value,
    "guatiao_entry": _abi.Entry,
    "guatiao_alloc": _abi.Alloc,
    "guatiao_provider_error": _abi.ProviderError,
    "guatiao_date": _abi.DateFields,
    "guatiao_time": _abi.TimeFields,
    "guatiao_datetime": _abi.DateTimeFields,
    "guatiao_timestamp": _abi.TimestampFields,
    "guatiao_span": _abi.SpanFields,
    "guatiao_text_error": _abi.TextError,
    "guatiao_wire_event": _abi.WireEvent,
    "guatiao_kind_header": _abi.KindHeader,
    "guatiao_object": _abi.Object,
    "guatiao_kinds": _abi.Kinds,
    "guatiao_kind_table": _abi.KindTable,
    "guatiao_kind_tables": _abi.KindTables,
    "guatiao_provider_info": _abi.ProviderInfo,
    "guatiao_providers": _abi.Providers,
    "guatiao_library_info": _abi.LibraryInfo,
    "guatiao_host_info": _abi.HostInfo,
}


class _LayoutEntry(ctypes.Structure):
    _fields_ = [("name", ctypes.c_char_p), ("value", ctypes.c_size_t)]


def _c_layout(into: Path) -> "dict[str, int]":
    built = native.build_layout(into)
    if built is None:
        pytest.skip("no C compiler to lay the header out with")
    lib = ctypes.CDLL(str(built))
    lib.guatiao_layout.argtypes = [ctypes.POINTER(ctypes.c_size_t)]
    lib.guatiao_layout.restype = ctypes.POINTER(_LayoutEntry)
    count = ctypes.c_size_t()
    entries = lib.guatiao_layout(ctypes.byref(count))
    return {entries[i].name.decode(): entries[i].value for i in range(count.value)}


def test_every_mirror_is_laid_out_as_the_c_compiler_lays_out_the_header(tmp_path):
    layout = _c_layout(tmp_path)
    for c_name, mirror in _MIRRORS.items():
        assert ctypes.sizeof(mirror) == layout[c_name], c_name
        for name, *_ in mirror._fields_:
            key = f"{c_name}.{name}"
            if key in layout:
                assert getattr(mirror, name).offset == layout[key], key
        header_fields = [k.split(".", 1)[1] for k in layout if k.startswith(c_name + ".")]
        assert [n for n, *_ in mirror._fields_] == header_fields or isinstance(mirror(), ctypes.Union), c_name


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

