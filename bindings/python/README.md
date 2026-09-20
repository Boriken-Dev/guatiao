# guatiao for Python

[![PyPI version](https://img.shields.io/pypi/v/guatiao.svg)](https://pypi.org/project/guatiao/)
[![Python versions](https://img.shields.io/pypi/pyversions/guatiao.svg)](https://pypi.org/project/guatiao/)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Python bindings for the [guatiao](https://github.com/Boriken-Dev/guatiao/)
ABI: **one value model for passing data between languages**, a JSON
Schema that describes a value, and a registry that loads plugin
libraries. Pure Python over `ctypes`, with no dependencies and nothing to
compile. It works with any shared library that exports the ABI, whether
that is `guatiao` itself or an application that carries it.

> **Status: alpha.** The wheel does not carry a native library; you
> point it at one.

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
pip install guatiao
```

The package drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib             # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll       # or the file itself, whatever it is called
```

Without the variable the package tries the system's library search
(`ctypes.util.find_library("guatiao")`) and then its own `_native/`
directory. Nothing is loaded at import time: `import guatiao` always
succeeds, and the first native call raises `guatiao.LibraryNotFound`,
naming every place it looked. A library that exports only part of the ABI
is fine: calling a function it lacks raises `guatiao.MissingSymbol`
naming it, and everything else works.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, the registry, provider tables |
| `guatiao_serde` | `guatiao.serde`: JSON, and TOML and YAML when the library has them |
| `guatiao_form` | `guatiao.form` |

`guatiao_serde` and `guatiao_form` are looked for the same way, in the
same directory, only when `guatiao.serde` or `guatiao.form` is first used.

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
    report = reg.scan_dir("/opt/myapp/plugins", rules="kind=greeter")
    for p in reg.providers("greeter"):           # best first
        print(p["key"], p["version"], p["from"])

    reg.why_not("codec")                         # why nothing serves a kind

    schema = reg.provider_config("acme_shouter")   # borrowed; do not close
    with reg.create("acme_shouter", {"prefix": "hey"}) as instance:
        ...
```

Every answer that is data comes back as a plain `dict` or `list`.
`rules` is one `[!]KEY=VALUE` per line and filters on what a library
declares about itself, before it is loaded.

To call a provider, describe its table with the `ctypes.Structure` the
kind's C header declares and let the package check it:

```python
from guatiao import kinds

table_ptr, size, ctx = reg.provider_table("acme_greeter")
greeter = kinds.table(table_ptr, size, GreeterVtable, floor_hash=FLOOR_HASH)
greeter.greet(ctx, name, out_map, err)
```

[`tests/test_greeter.py`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/tests/test_greeter.py)
is a complete worked example.

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

A format the loaded library does not export raises
`NotImplementedError` naming it.

### Errors

A failed native call raises `guatiao.GuatiaoError`, as one of `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone` or
`Internal`. `.status` is the C status code, and `.message` carries a
provider's own words when it gave any.

## Known limits

- **A provider that files one table per kind cannot be called yet.**
  The ABI has no function to fetch a table by kind; `provider_table`
  reaches the single table a provider declares for everything. Listing,
  configuring and instantiating such providers all work.
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

```bash
pip install -e "bindings/python[dev]"     # from the repository root, in a virtual environment
pytest bindings/python/tests -rs
```

The tests need the three libraries and the repository's example plugins
in one directory. `conftest.py` points `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it. With everything present nothing
skips; a test that needs a missing library skips and says which. CI runs
the suite on Python 3.9 and 3.14.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
