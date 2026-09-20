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

v = Value(host="10.0.0.1", port=5900, tags=["a", "b"])
m = v.as_map()
m["port"].to_python()          # 5900
m["port"] = 5901
list(m)                        # ["host", "port", "tags"]
m["tags"].as_list().append("c")
v.to_python()                  # back to a plain dict/list tree
v.close()
```

**Construction**: `Value(obj)` takes what the builtins would -- `Value(1)`,
`Value(True)`, `Value("x")`, `Value([1, 2])`, `Value({"host": "h"})` --
and, as `dict` does, `Value(host="h", port=5900)` and
`Value(mapping, port=1)`. `Value()` is absent. `alloc` is a reserved
keyword, so a field of that name goes in a mapping. `Value.from_python(obj,
*, alloc=None)` is the same conversion by name. Also `Value.null()`,
`.absent()`, `.bool(b)`, `.string(s)`, `.bytes_(b)`, `.number(text)` (the
exact text, no float/Decimal detour), `.map()`, `.list()`. Conversion:
`None` -> null, `bool`, `int` -> number, `float`
(`repr`) and `decimal.Decimal` (its own text) -> number (non-finite
raises `ValueError`), `str` -> string, `bytes`/`bytearray` -> bytes,
`list`/`tuple` -> list, a mapping with `str` keys -> map, a `Value`/`Ref` ->
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

## Registry

```python
from guatiao.registry import Registry

with Registry("my-host", "1.0") as reg:
    reg.scan_dir("/path/to/plugins", rules="kind=greeter")  # rules: newline-separated
    reg.providers("greeter")        # list of dicts, best first
    reg.best("greeter")             # dict: key, id, version, library, display_name,
                                     # from, kinds, has_config, vtable_size
    reg.why_not("codec")            # {"available": False, "why": "nothing-claims-it", ...}
    table, size, ctx = reg.provider_table("acme_hello")  # a single-`vtable` provider's table
    instance = reg.create("acme_shouter", {"prefix": "hey"})
    instance.close()
```

Also: `load_file`, `scan_path`, `register_entry(name, EntryFn(...))`, `libraries()`,
`provider(key)`, `available(kind)`, `provider_available(key)`,
`keyed_by`/`libraries_keyed_by`, `set_priority`/`priority`,
`provider_config(key)` (a borrowed, read-only `Ref` onto the schema --
never `.close()` it), `host()` (the raw `host_info` pointer, for driving
a `guatiao_library_entry` by hand). Every answer that is data comes back
as a plain `dict`/`list`, already freed.

**`provider_table` reads the provider's single `vtable` field only.** A
provider that files one table per kind (under `tables` in its descriptor)
is not reachable: the ABI exports no lookup by kind. Such a provider
answers a null table and size 0; listing, configuring and instantiating
it all work.

`Registry.retire`/`Registry.unload` are not implemented in this binding
until the library exports `guatiao_registry_unload`.

`guatiao.kinds.table(table_ptr, size, struct_type, floor_hash=...)`
turns a `provider_table()` answer into the `ctypes.Structure` a kind's
own C header declares (`greeter_vtable` and friends): checks `size`
against `sizeof(struct_type)` and the header's `floor_hash`, then casts.
Raises `guatiao.kinds.FloorMismatch` on either failure.

## serde: JSON, TOML, YAML

```python
from guatiao import serde

text = serde.dumps(value, format="json", pretty=True)   # or "toml", "yaml"
value = serde.loads(text, format="json")
```

`toml`/`yaml` raise `NotImplementedError` naming the feature when the
loaded `guatiao_serde` was not built with it. A TOML document must be a
map (`WrongKind` otherwise).

A number keeps its exact text through JSON, with one exception the
parser owns: an exponent is read back with its sign written (`1e400`
parses as `1e+400`).

## form

```python
from guatiao import form

form.check(schema, form_doc)             # None, or an error map
form.layout(schema, form_doc)            # [{"section": ..., "fields": [...]}, ...]
form.is_visible(schema, form_doc, "key", values)   # bool
```

## Errors

Every failing `guatiao_status` raises `guatiao.GuatiaoError` (`.status`
the raw code, `.message` a provider's own words when there were any), as
one of its subclasses: `BadValue`, `AllocFailed`, `WrongKind`,
`NotFound`, `NullArgument`, `Gone`, `Internal`.

## Threading

A `Registry` is used from one thread at a time, like the C handle
it wraps. A `Value` is not shared across threads without the
caller's own lock.
