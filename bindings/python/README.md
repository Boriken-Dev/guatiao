# guatiao for Python

[![Status: alpha](https://img.shields.io/badge/status-alpha-orange.svg)](https://github.com/Boriken-Dev/guatiao)
[![Python 3.9+](https://img.shields.io/badge/python-3.9%2B-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/pyproject.toml)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Python bindings for [guatiao](https://github.com/Boriken-Dev/guatiao/): **one value model for passing data
between languages**, a JSON Schema that describes a value, and a registry
that loads plugin libraries. Pure Python over `ctypes`: no dependencies,
nothing to compile, driving the same C ABI a C, C++ or Dart program uses.

> **Status: alpha.** Not on PyPI yet, and no wheel carries the native
> library; you build that from the repository.

## Features

- **Value trees as Python objects** — build, read and change a tree in
  place, or convert it to and from plain `dict`/`list` data.
- **Numbers keep their exact text** — `1.10` stays `1.10` and an integer
  of any size survives; you choose `int`/`float`, `Decimal` or `str` when
  reading.
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-form` libraries when they are present.
- **Importing never fails** — a native library is looked for on first
  use, and a missing one says every place that was searched.

## Installation

```bash
pip install -e "bindings/python"          # from a checkout of the repository
```

The package needs the native library, built from the same checkout:

```bash
cargo build --workspace --all-features
export GUATIAO_LIBRARY=/path/to/guatiao/target/debug     # a directory, or the file itself
```

`--all-features` matters: the registry functions are only exported with
the `load` feature, and TOML and YAML only with theirs. Without
`GUATIAO_LIBRARY` the package tries the system's library search
(`ctypes.util.find_library`) and then its own `_native/` directory; with
nothing found, the first native call raises `guatiao.LibraryNotFound`.
`guatiao_serde` and `guatiao_form` are found the same way, only when
`guatiao.serde` or `guatiao.form` is first used.

| Native library | Needed for |
| --- | --- |
| `guatiao` | values, the registry, provider tables |
| `guatiao_serde` | `guatiao.serde` (JSON; TOML and YAML when built with those features) |
| `guatiao_form` | `guatiao.form` |

## Quick start

### Values

```python
from guatiao import Value

with Value(host="10.0.0.1", port=5900, tags=["a", "b"]) as v:
    m = v.as_map()
    m["port"] = 5901
    m["tags"].as_list().append("c")

    m["port"].to_python()        # 5901
    list(m)                      # ['host', 'port', 'tags'], insertion order
    "missing" in m               # False; m["missing"] raises KeyError
    m["port"].int_or(22)         # 5901, or 22 if it were not an integer
    v.to_python()                # back to a plain dict
```

A `Value` owns its tree and frees it on `close()` or at the end of the
`with` block. `m["tags"]` is a `Ref`: a path into the tree that is
walked again on every access, so it stays valid while the tree grows
around it.

**Numbers keep their exact text.** `1.10` stays `1.10`, and an integer of
any size survives. Choose what comes back when reading:

```python
v = Value.number("1.10")
v.to_python()                    # 1.1   ("auto": int when it looks like one, else float)
v.to_python(numbers="decimal")   # Decimal('1.10')
v.to_python(numbers="str")       # '1.10'
```

`Value(1)`, `Value(True)`, `Value("x")`, `Value([1, 2])` and
`Value({"host": "h"}, port=1)` all work, as the builtins would.
`None` becomes null, `bytes` the bytes kind, a `dict` with `str` keys a
map. A missing value reads back as `guatiao.ABSENT`, which is falsy and
is not `None`. A `float` that is `nan` or infinite raises `ValueError`.

### Hosting plugins

```python
from guatiao import Registry

with Registry("my-host", "1.0") as reg:
    report = reg.scan_dir("target/debug", rules="kind=greeter")
    for p in reg.providers("greeter"):           # best first
        print(p["key"], p["version"], p["from"])

    reg.why_not("codec")                         # why nothing serves a kind

    schema = reg.provider_config("derived_greeter_shouter")   # borrowed; do not close
    with reg.create("derived_greeter_shouter", {"prefix": "hey"}) as instance:
        ...
```

Every answer that is data comes back as a plain `dict` or `list`.
`rules` is one `[!]KEY=VALUE` per line and filters on what a library
declares about itself, before it is loaded.

To call a provider, describe its table with the `ctypes.Structure` the
kind's C header declares and let the package check it:

```python
from guatiao import kinds

table_ptr, size, ctx = reg.provider_table("hello_library_greeter")
greeter = kinds.table(table_ptr, size, GreeterVtable, floor_hash=FLOOR_HASH)
greeter.greet(ctx, name, out_map, err)
```

[`tests/test_greeter.py`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/tests/test_greeter.py)
is a complete worked example against `examples/hello_library`.

### JSON, TOML, YAML and forms

```python
from guatiao import serde, form

value = serde.loads('{"port": 5900}')                  # format="json" by default
text = serde.dumps(value, format="yaml")
pretty = serde.dumps(value, pretty=True)

problem = form.check(schema, form_doc)                 # None when the form fits the schema
sections = form.layout(schema, form_doc)
shown = form.is_visible(schema, form_doc, "tls.verify", values)
```

A format the loaded library was not built with raises
`NotImplementedError` naming the feature.

### Errors

A failed native call raises `guatiao.GuatiaoError`, as one of `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone` or
`Internal`. `.status` is the C status code, and `.message` carries a
provider's own words when it gave any.

## Known limits

- **A provider written with `#[derive(Provider)]` cannot be called from
  Python yet.** It files its tables per kind, and the C ABI has no
  function to fetch one by kind; `provider_table` only reaches the single
  table a hand-written provider declares. Listing, configuring and
  instantiating derived providers all work.
- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
  Everything else about a number's text survives a round trip.
- `List.insert` and `list[i] = x` rebuild the list, because the C ABI
  only appends and removes. `append` and `del list[i]` are direct.
- A `Registry` is used from one thread at a time, and a `Value` is not
  shared between threads without a lock of your own.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `Map`, `List`, `ABSENT`, `Registry`, `Instance`, the error classes |
| `guatiao.registry` | the plugin host: load, scan, list, rank, configure, instantiate |
| `guatiao.kinds` | check and cast a provider's function table |
| `guatiao.serde` | `loads` / `dumps` for JSON, TOML, YAML |
| `guatiao.form` | `check`, `layout`, `is_visible` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/src/guatiao/AGENTS.md) is
the full API reference and ships inside the wheel. The C ABI is described
by [`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

From the repository root, in a virtual environment:

```bash
cargo build --workspace --all-features
pip install -e "bindings/python[dev]"
pytest bindings/python/tests -rs
```

`conftest.py` points `GUATIAO_LIBRARY` at the repository's `target/debug`
when the variable is unset. After a full build nothing should skip; a
test that needs a missing library skips and says which. CI runs the suite
on Python 3.9 and 3.14.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
