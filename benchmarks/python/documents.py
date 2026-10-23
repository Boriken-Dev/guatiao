# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A document to plain Python objects and back, three ways: through a
`Value` (`loads(...).to_python()`, `dumps(Value.from_python(...))`),
through the wire bytes as a tape decoded in Python (`parse`, `render`),
and through ctypes callbacks (a visitor table the library calls per
event, a walker calling the library's visitor per event).

    GUATIAO_LIBRARY=<target>/release python benchmarks/python/documents.py [--save NAME]

with both Python packages importable (their `src/` on `PYTHONPATH`).
Writes `benchmarks/results/NAME.json` in the runner's shape."""

from __future__ import annotations

import argparse
import ctypes
import json
import platform
import statistics
import sys
import time
from ctypes import CFUNCTYPE, POINTER, c_bool, c_uint32, c_uint64, c_void_p
from pathlib import Path
from typing import Any, Callable

import guatiao_serde as serde
from guatiao import Value, ffi

_status = c_uint32
_A0 = CFUNCTYPE(_status, c_void_p)
_AB = CFUNCTYPE(_status, c_void_p, c_bool)
_AS = CFUNCTYPE(_status, c_void_p, ffi.Str)
_AY = CFUNCTYPE(_status, c_void_p, ffi.Bytes)
_AN = CFUNCTYPE(_status, c_void_p, c_uint64)


class _Visitor(ctypes.Structure):
    _fields_ = [
        ("struct_size", c_uint32),
        ("ctx", c_void_p),
        ("absent", _A0),
        ("null", _A0),
        ("boolean", _AB),
        ("number", _AS),
        ("string", _AS),
        ("bytes", _AY),
        ("list_begin", _AN),
        ("list_end", _A0),
        ("map_begin", _AN),
        ("key", _AS),
        ("map_end", _A0),
    ]


class _Walker(ctypes.Structure):
    _fields_ = [
        ("struct_size", c_uint32),
        ("ctx", c_void_p),
        ("walk", CFUNCTYPE(_status, c_void_p, POINTER(_Visitor))),
    ]


def _cdll() -> ctypes.CDLL:
    lib = ctypes.CDLL(str(serde.library().path))
    lib.guatiao_json_visit.argtypes = [
        ffi.Str, c_uint32, POINTER(ffi.Alloc), POINTER(_Visitor), c_void_p
    ]
    lib.guatiao_json_visit.restype = _status
    lib.guatiao_json_emit_walker.argtypes = [
        POINTER(_Walker), c_uint32, POINTER(ffi.Alloc), POINTER(ffi.Node)
    ]
    lib.guatiao_json_emit_walker.restype = _status
    return lib


def _text(s: ffi.Str) -> bytes:
    return ctypes.string_at(s.ptr, s.len) if s.len else b""


def parse_callbacks(lib: ctypes.CDLL, text: str) -> Any:
    """`parse` built from a visitor table the library calls per event."""
    stack: "list[Any]" = []
    keys: "list[Any]" = []
    out: "list[Any]" = []
    key: "list[Any]" = [None]

    def put(v: Any) -> int:
        if not stack:
            out.append(v)
        elif type(stack[-1]) is dict:
            stack[-1][key[0]] = v
        else:
            stack[-1].append(v)
        return 0

    def begin(c: Any) -> int:
        keys.append(key[0])
        stack.append(c)
        return 0

    def end(_ctx: Any) -> int:
        c = stack.pop()
        key[0] = keys.pop()
        return put(c)

    def number(_ctx: Any, s: ffi.Str) -> int:
        t = _text(s)
        return put(float(t) if b"." in t or b"e" in t or b"E" in t else int(t))

    def set_key(_ctx: Any, s: ffi.Str) -> int:
        key[0] = _text(s).decode("utf-8")
        return 0

    v = _Visitor(
        ctypes.sizeof(_Visitor),
        None,
        _A0(lambda _c: put(None)),
        _A0(lambda _c: put(None)),
        _AB(lambda _c, b: put(bool(b))),
        _AS(number),
        _AS(lambda _c, s: put(_text(s).decode("utf-8"))),
        _AY(lambda _c, b: put(ctypes.string_at(b.ptr, b.len) if b.len else b"")),
        _AN(lambda _c, _n: begin([])),
        _A0(end),
        _AN(lambda _c, _n: begin({})),
        _AS(set_key),
        _A0(end),
    )
    view, _buf = ffi.text(text)
    alloc = serde.library().alloc
    ffi.check(lib.guatiao_json_visit(view, 0, ctypes.byref(alloc), ctypes.byref(v), None))
    return out[0]


def render_callbacks(lib: ctypes.CDLL, obj: Any) -> str:
    """`render` driven by a walker that calls the library's visitor per
    Python object."""

    def walk(_ctx: Any, into: Any) -> int:
        v = into.contents
        ctx = v.ctx

        def go(o: Any) -> None:
            if isinstance(o, str):
                b = o.encode("utf-8")
                ffi.check(v.string(ctx, ffi.Str(ctypes.cast(ctypes.c_char_p(b), c_void_p), len(b))))
            elif o is None:
                ffi.check(v.null(ctx))
            elif o is True or o is False:
                ffi.check(v.boolean(ctx, o))
            elif isinstance(o, (int, float)):
                b = (repr(o) if isinstance(o, float) else str(o)).encode("ascii")
                ffi.check(v.number(ctx, ffi.Str(ctypes.cast(ctypes.c_char_p(b), c_void_p), len(b))))
            elif isinstance(o, dict):
                ffi.check(v.map_begin(ctx, len(o)))
                for k, item in o.items():
                    b = k.encode("utf-8")
                    ffi.check(v.key(ctx, ffi.Str(ctypes.cast(ctypes.c_char_p(b), c_void_p), len(b))))
                    go(item)
                ffi.check(v.map_end(ctx))
            elif isinstance(o, list):
                ffi.check(v.list_begin(ctx, len(o)))
                for item in o:
                    go(item)
                ffi.check(v.list_end(ctx))
            else:
                raise TypeError(type(o))

        go(obj)
        return 0

    w = _Walker(ctypes.sizeof(_Walker), None, _Walker._fields_[2][1](walk))
    out = ffi.Node()
    alloc = serde.library().alloc
    ffi.check(lib.guatiao_json_emit_walker(ctypes.byref(w), 0, ctypes.byref(alloc), ctypes.byref(out)))
    try:
        return ffi.text_of(out)
    finally:
        ffi.free(out)


def _config() -> Any:
    return {
        "name": "fleet",
        "servers": [
            {
                "host": f"node-{i}.example",
                "port": 8000 + i,
                "weight": i / 7,
                "tags": ["edge", "eu", str(i)],
                "enabled": i % 3 != 0,
                "limits": {"rps": 1000 + i, "burst": 2.5, "note": None},
            }
            for i in range(50)
        ],
    }


def _cases() -> "dict[str, Any]":
    return {
        "config": _config(),
        "floats_10k": [i * 1.25 + 0.5 for i in range(10_000)],
        "text_1mb": {"blob": "x" * (1 << 20)},
    }


def _time(fn: Callable[[], Any], target_s: float, samples: int) -> "list[float]":
    fn()
    n = 1
    while True:
        t = time.perf_counter()
        for _ in range(n):
            fn()
        if time.perf_counter() - t >= target_s:
            break
        n *= 2
    out = []
    for _ in range(samples):
        t = time.perf_counter()
        for _ in range(n):
            fn()
        out.append((time.perf_counter() - t) / n * 1000.0)
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--save")
    ap.add_argument("--samples", type=int, default=15)
    ap.add_argument("--target-ms", type=float, default=20)
    args = ap.parse_args()
    lib = _cdll()
    metrics: "dict[str, Callable[[], Any]]" = {}
    for name, obj in _cases().items():
        text = serde.render(obj)
        # Every path reads and writes the same thing.
        assert serde.parse(text) == obj == serde.loads(text).to_python() == parse_callbacks(lib, text)
        assert serde.render(obj) == serde.dumps(Value.from_python(obj)) == render_callbacks(lib, obj)
        metrics[f"{name}_parse_value"] = lambda t=text: serde.loads(t).to_python()
        metrics[f"{name}_parse_tape"] = lambda t=text: serde.parse(t)
        metrics[f"{name}_parse_callbacks"] = lambda t=text: parse_callbacks(lib, t)
        metrics[f"{name}_render_value"] = lambda o=obj: serde.dumps(Value.from_python(o))
        metrics[f"{name}_render_tape"] = lambda o=obj: serde.render(o)
        metrics[f"{name}_render_callbacks"] = lambda o=obj: render_callbacks(lib, o)
    result: "dict[str, Any]" = {"name": args.save or "documents", "metrics": {}, "raw": {
        "processor": platform.machine().lower(),
        "os": platform.system().lower(),
        "python": platform.python_version(),
        "samples_ms": {},
    }}
    for name, fn in metrics.items():
        samples = _time(fn, args.target_ms / 1000.0, args.samples)
        result["metrics"][name] = {
            "min_ms": min(samples),
            "median_ms": statistics.median(samples),
            "max_ms": max(samples),
        }
        result["raw"]["samples_ms"][name] = samples
        print(f"{name:32} {statistics.median(samples) * 1000:12.1f} us", flush=True)
    if args.save:
        path = Path(__file__).resolve().parents[1] / "results" / f"{args.save}.json"
        path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8", newline="\n")
        print(f"saved {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
