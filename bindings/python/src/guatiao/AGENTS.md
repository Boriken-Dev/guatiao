# guatiao (Python) — API header

ctypes bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the wheel, so it names no repo path.

## Finding the native library

Importing `guatiao` touches no native library. A native call is resolved
lazily, on first use, in this order:

1. `GUATIAO_LIBRARY` — a file, or a directory holding
   `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib`.
2. `ctypes.util.find_library("guatiao")`.
3. This package's own `_native/` directory (empty until a build step
   populates it).

Not found raises `guatiao.LibraryNotFound` naming the three places it
looked. `guatiao_serde` and `guatiao_form` resolve the same way,
independently, only when `guatiao.serde` / `guatiao.form` is first used.

## Values

`Value` owns a root `guatiao_value`; free it with `close()`, or use it as
a context manager. `Ref` addresses a node inside a `Value`'s tree by a
path of keys/indices, walked fresh on every access -- never a cached
pointer, since a sibling mutation can reallocate the array it points
into. `Map` and `List` are `Ref`s with the mapping and sequence
protocols. `Value` is itself a `Ref` onto its own root.

```python
from guatiao import Value, ABSENT

v = Value.from_python({"host": "10.0.0.1", "port": 5900, "tags": ["a", "b"]})
m = v.as_map()
m["port"].to_python()          # 5900
m["port"] = 5901
list(m)                        # ["host", "port", "tags"]
m["tags"].as_list().append("c")
v.to_python()                  # back to a plain dict/list tree
v.close()
```

**Construction**: `Value.null()`, `.absent()`, `.bool(b)`, `.string(s)`,
`.bytes_(b)`, `.number(text)` (the exact text, no float/Decimal detour),
`.map()`, `.list()`, and `Value.from_python(obj, *, alloc=None)` for a
whole tree at once: `None` -> null, `bool`/`int` -> number, `float`
(`repr`) and `decimal.Decimal` (its own text) -> number (non-finite
raises `ValueError`), `str` -> string, `bytes`/`bytearray` -> bytes,
`list`/`tuple` -> list, `dict` with `str` keys -> map, a `Value`/`Ref` ->
a deep copy.

**Reading**: `ref.to_python(numbers="auto")` walks the whole node --
`"auto"` gives `int` when the number's text has no `.`/`e`/`E`, else
`float`; `"decimal"` gives `Decimal`; `"str"` the exact text, always.
Absent reads back as `guatiao.ABSENT` (falsy, distinct from `None`).
`ref.bool_or/int_or/float_or/str_or(fallback)` read one field with the
header's own rules (`int_or` never truncates: wrong kind, a fractional
or exponent spelling, or a value outside a signed 64-bit range all fall
back). `ref.tag`, `.is_absent()`, `.is_null()`. `ref.clone()` deep-copies
the node into a new, independent `Value`.

**`Map`**: `__getitem__`/`__setitem__`/`__delitem__`/`__len__`/`__iter__`
(over keys)/`__contains__`, `keys()`, `items()`, `clear()`,
`copy_from(other)` (every entry of `other`, replacing collisions).

**`List`**: `__getitem__`/`__delitem__`/`__len__`/`__iter__`, `append`,
`clear`. `guatiao`'s C surface exports only append, discard-by-index and
clear -- no native "set" or "insert" at a position -- so `__setitem__`
and `insert` are built from what exists (clone every element, clear,
push back in the new order) and are O(n); `append` and `del list[i]` use
their own direct export.

## Errors

Every failing `guatiao_status` raises `guatiao.GuatiaoError` (`.status`
the raw code, `.message` a provider's own words when there were any), as
one of its subclasses: `BadValue`, `AllocFailed`, `WrongKind`,
`NotFound`, `NullArgument`, `Gone`, `Internal`.

## Threading

A `Registry` is used from one thread at a time, like the Rust and C
handles it wraps. A `Value` is not shared across threads without the
caller's own lock.
