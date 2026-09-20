# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The C structs and enums of guatiao.h, as ctypes.

Mirrors `crates/guatiao/include/guatiao.h` field for field, in the
header's own order. No `_pack_`: the header is natural alignment, which
is what a C compiler also gives these structs, so ctypes' default
layout already matches.

This binding targets 64-bit platforms only (size_t and every pointer are
8 bytes) -- the same assumption the sizes below pin.
"""

from __future__ import annotations

import ctypes
import enum

c_void_p = ctypes.c_void_p
c_size_t = ctypes.c_size_t
c_uint8 = ctypes.c_uint8
c_bool = ctypes.c_bool
c_uint32 = ctypes.c_uint32


class Tag(enum.IntEnum):
    """`guatiao_tag`: which arm of a value's payload is live."""

    ABSENT = 0
    NULL = 1
    BOOL = 2
    NUMBER = 3
    STRING = 4
    BYTES = 5
    LIST = 6
    MAP = 7


class Status(enum.IntEnum):
    """`guatiao_status`: what an entry point reports."""

    OK = 0
    ERR_BAD_VALUE = 7
    ERR_ALLOC = 8
    ERR_WRONG_KIND = 9
    ERR_NOT_FOUND = 10
    ERR_NULL = 11
    ERR_GONE = 12
    ERR_INTERNAL = 100


# Forward declarations: `List` holds `Value *`, `Map` holds `Entry *`, and
# `Entry` holds a `Value` -- the same cycle the header breaks with a
# forward declaration. ctypes allows an incomplete Structure's pointer to
# be taken before `_fields_` is set, as long as `_fields_` lands before
# any instance is built.
class Value(ctypes.Structure):
    pass


class Entry(ctypes.Structure):
    pass


class Alloc(ctypes.Structure):
    """`guatiao_alloc`: an allocator vtable a foreign caller fills in."""

    _fields_ = [
        ("struct_size", c_uint32),
        ("ctx", c_void_p),
        ("alloc", ctypes.CFUNCTYPE(c_void_p, c_void_p, c_size_t, c_size_t)),
        ("free", ctypes.CFUNCTYPE(None, c_void_p, c_void_p, c_size_t, c_size_t)),
        ("release", ctypes.CFUNCTYPE(None, c_void_p)),
    ]


class Str(ctypes.Structure):
    """`guatiao_str`: borrowed UTF-8 text. Check `len` before `ptr`."""

    _fields_ = [("ptr", c_void_p), ("len", c_size_t)]


class Bytes(ctypes.Structure):
    """`guatiao_bytes`: borrowed bytes, any content, NULs included."""

    _fields_ = [("ptr", c_void_p), ("len", c_size_t)]


class String(ctypes.Structure):
    """`guatiao_string`: owned, growable UTF-8 text. `cap == 0` = not owned."""

    _fields_ = [
        ("ptr", c_void_p),
        ("len", c_size_t),
        ("cap", c_size_t),
        ("alloc", ctypes.POINTER(Alloc)),
    ]


class Buffer(ctypes.Structure):
    """`guatiao_buffer`: owned, growable bytes. Same `cap` rule as `String`."""

    _fields_ = [
        ("ptr", c_void_p),
        ("len", c_size_t),
        ("cap", c_size_t),
        ("alloc", ctypes.POINTER(Alloc)),
    ]


class List(ctypes.Structure):
    """`guatiao_list`: an owned, growable sequence of values."""

    _fields_ = [
        ("ptr", ctypes.POINTER(Value)),
        ("len", c_size_t),
        ("cap", c_size_t),
        ("alloc", ctypes.POINTER(Alloc)),
    ]


class Map(ctypes.Structure):
    """`guatiao_map`: an owned, growable sequence of entries, insertion order."""

    _fields_ = [
        ("ptr", ctypes.POINTER(Entry)),
        ("len", c_size_t),
        ("cap", c_size_t),
        ("alloc", ctypes.POINTER(Alloc)),
    ]


class Payload(ctypes.Union):
    """`guatiao_payload`: which arm is live is decided by the tag alone."""

    _fields_ = [
        ("b", c_bool),
        ("text", String),
        ("number", String),
        ("bytes", Buffer),
        ("list", List),
        ("map", Map),
    ]


Value._fields_ = [
    ("tag", c_uint32),
    ("_pad", c_uint32),
    ("payload", Payload),
]

Entry._fields_ = [
    ("key", String),
    ("value", Value),
]


class Values(ctypes.Structure):
    """`guatiao_values`: a borrowed sequence of values, in order."""

    _fields_ = [("ptr", ctypes.POINTER(Value)), ("len", c_size_t)]


class Entries(ctypes.Structure):
    """`guatiao_entries`: a borrowed sequence of entries, insertion order."""

    _fields_ = [("ptr", ctypes.POINTER(Entry)), ("len", c_size_t)]


class ProviderError(ctypes.Structure):
    """`guatiao_provider_error`: why a call across a kind failed."""

    _fields_ = [("status", c_uint32), ("message", String)]


class KindHeader(ctypes.Structure):
    """`guatiao_kind_header`: the first eight bytes of every kind table."""

    _fields_ = [("struct_size", c_uint32), ("floor_hash", c_uint32)]


class Object(ctypes.Structure):
    """`guatiao_object`: an object crossing the boundary -- table, size, ctx."""

    _fields_ = [("table", c_void_p), ("size", c_size_t), ("ctx", c_void_p)]


class BytesMut(ctypes.Structure):
    """`guatiao_bytes_mut`: borrowed writable bytes. Check `len` before `ptr`."""

    _fields_ = [("ptr", c_void_p), ("len", c_size_t)]


def _offset(struct: type, field: str) -> int:
    return getattr(struct, field).offset


# Sizes and offsets are the ABI: pinned by `value/types/mod.rs` and
# asserted again there with `_Static_assert`-equivalent Rust `const`
# checks. A mismatch here means this binding was built for the wrong
# pointer width, not that the crate changed.
assert ctypes.sizeof(Str) == 16
assert ctypes.sizeof(Bytes) == 16
assert ctypes.sizeof(Values) == 16
assert ctypes.sizeof(Entries) == 16
assert ctypes.sizeof(String) == 32
assert ctypes.sizeof(Buffer) == 32
assert ctypes.sizeof(List) == 32
assert ctypes.sizeof(Map) == 32
assert ctypes.sizeof(Payload) == 32
assert ctypes.sizeof(Value) == 40
assert ctypes.sizeof(Entry) == 72
assert _offset(Value, "tag") == 0
assert _offset(Value, "_pad") == 4
assert _offset(Value, "payload") == 8
assert _offset(Entry, "key") == 0
assert _offset(Entry, "value") == 32
