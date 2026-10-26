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

> **Status: alpha.** A pure Python package: it carries no native
> library, you point it at one.

## Features

- **Value trees as Python objects** — build, read and change a tree in
  place, or convert it to and from plain `dict`/`list` data.
- **Numbers keep their exact text** — `1.10` stays `1.10` and an integer
  of any size survives; you choose `int`/`float`, `Decimal` or `str` when
  reading.
- **Be a plugin host** — scan a directory, filter on what a library
  declares before loading it, list providers best first, build an
  instance from a configuration, call a provider through its table with
  Python values in and out.
- **Hand a library a Python object** — a plain class becomes an object of
  a kind (a listener, a callback) that a library written in any language
  calls, or holds until it is done with it.
- **Write a provider in Python** — classes become providers of a kind, in
  a library a host in any language calls the way it calls one written in
  Rust or C.
- **Dates, times, instants and spans as Python's own types** — `date`,
  `time`, naive and aware `datetime`, `timedelta`, and a `Span` class for
  calendar lengths, read and written by the library itself.
- **Values on the wire** — `value.to_wire()` and `Value.from_wire(data)`
  carry a tree exactly as bytes, and `guatiao.wire` announces a schema on
  a channel before its values, with a `Receiver` that checks each one.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **forms checked against a schema**, are
  the packages [`guatiao-serde`](https://pypi.org/project/guatiao-serde/)
  and [`guatiao-intake`](https://pypi.org/project/guatiao-intake/), each
  over its own library and built on this one.
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
(`ctypes.util.find_library("guatiao")`). Nothing is loaded at import time: `import guatiao` always
succeeds, and the first native call raises `guatiao.LibraryNotFound`,
naming every place it looked. A library that exports only part of the ABI
is fine: calling a function it lacks raises `guatiao.MissingSymbol`
naming it, and everything else works.

`guatiao.use_library(path)` names the library from code instead, before
the first native call.

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

    reg.retire("acme")                           # out of the registry, still mapped
    reg.collect()                                # ["acme"]: closed, once nothing of it is alive
    reg.unload("acme2")                          # out and unmapped at once, when it agrees
```

A library can also come from a **source**: a file that serves libraries
which come and go while the host runs, such as a bridge to another
language, or a directory the registry keeps in step with disk.
`refresh()` makes the registry agree with them, whenever you call it:

```python
reg.add_source_file("/opt/myapp/guatiao_source_py.so", {"dirs": ["/opt/myapp/py"]})
reg.watch_dir("/opt/myapp/plugins", rules="kind=greeter")
changed = reg.refresh()        # what loaded, retired, was skipped or refused
```

Every answer that is data comes back as a plain `dict` or `list`.
`rules` is one `[!]KEY=VALUE` per line and filters on what a library
declares about itself, before it is loaded.

To call a provider, describe its kind once, from the kind's C header,
as a typed `Protocol` class: its methods in table order, what each takes
and answers as annotations, and the `FLOOR_HASH` the header defines. The
package checks a table against it and converts every argument and
answer; mypy and pyright check the code that uses it:

```python
from typing import Protocol

from guatiao import Map, Value
from guatiao.kinds import KindProtocol, Lent, MapAnswer, fallible, required

class Greeter(KindProtocol, Protocol, kind="greeter", floor_hash=2695856491):
    @required
    @fallible
    def greet(self, name: str) -> MapAnswer: ...
    @fallible
    def check(self, config: Lent[Map]) -> bool: ...

greeter = reg.bind("acme_greeter", Greeter)
greeter.greet("ana").to_python()          # {"greeting": "hello, ana"}
with Value({"name": "bo"}) as config:
    greeter.check(config)                 # the provider fills it in, in place
```

A kind that has a description, the `<crate>.kinds.json` a kind crate
commits, is written out as these classes by `python -m guatiao.kinds
DOCUMENT -o module.py`, or loaded at run time by `kinds.kinds_of(document)`;
`reg.kind_descriptions(key)` answers the document a library carries.

An integer says its width (`kinds.I64`, `kinds.U32`, ...), as the C
header does. An object a slot takes, such as a listener, is any Python
object with the kind's methods; an object a slot answers closes with
`with`.
[`examples/python_host`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/python_host)
is a complete host, and its kind classes are in
[`examples/greeter_kind/python`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/greeter_kind/python).

### Writing a provider

A module is a library: its `__name__` is the id, its `__version__` the
version, and the provider classes it exports (`__all__`, or those it
defines) its providers. A provider class inherits `provide.Provider` and
the kinds it serves:

```python
# greeters.py
from dataclasses import dataclass
from typing import Dict

from guatiao import BadValue, provide
from greeter_kind import Greeter

__version__ = "1.0.0"
__all__ = ["Hello", "Shouter"]

class Hello(provide.Provider, Greeter):
    def greet(self, name: str) -> Dict[str, str]:
        if not name:
            raise BadValue("nobody to greet")   # the caller reads this status and message
        return {"greeting": f"hello, {name}"}

@dataclass(frozen=True)
class ShouterConfig:
    prefix: str

class Shouter(provide.Provider, Greeter):
    def __init__(self, config: ShouterConfig) -> None:   # the schema hosts check against
        self.prefix = config.prefix
    def greet(self, name: str) -> Dict[str, str]:
        return {"greeting": f"{self.prefix} {name.upper()}"}
```

```python
import greeters

reg.register_library(greeters)
reg.bind("greeters_hello", Greeter).greet("ana")
```

A method a kind requires and the class lacks is a `TypeError` when the
class is created. The library's `meta` comes from the installed
distribution's metadata, then the module's docstring, `__author__`,
`__license__` and `__copyright__`, then a `_meta_` dict.
[`examples/python_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/python_provider)
holds complete libraries, which the `guatiao-source-py` bridge also serves to
hosts in any language.

### Dates, times and secrets

```python
import datetime as dt
from guatiao import redact, time as gt

gt.parse_timestamp("2026-10-21T19:15:00+05:30")    # aware datetime in UTC
gt.format_date(dt.date(2026, 10, 21))              # "2026-10-21"
gt.Span.parse("P1M").add_to(dt.date(2026, 1, 31))  # date(2026, 2, 28)
gt.to_timedelta([90, 500_000_000])                 # timedelta(seconds=90.5)

shown = redact(schema, value)                      # secrets replaced by "[redacted]"
```

Text a form's grammar refuses raises `guatiao.Unparsed`, a `ValueError`
carrying the byte offset and what was expected there.

### Errors

A failed native call raises `guatiao.GuatiaoError`, as one of `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`, `Unsupported`, `Busy` or
`Internal`. `.status` is the C status code, and `.message` carries a
provider's own words when it gave any.

## Known limits

- `datetime`, `time` and `timedelta` count microseconds and start at
  year 1: reading a date or time drops what is below a microsecond, and
  the year 0000 raises `ValueError`. `normalize` and `Span` keep every
  nanosecond.
- `List.insert` and `list[i] = x` rebuild the list, because the C ABI
  only appends and removes. `append` and `del list[i]` are direct.
- Runs where CPython can load a native library, on 64-bit targets only:
  every pointer and `size_t` in the ABI is eight bytes. It does not run in
  a browser (Pyodide has no way to load the library) or on a 32-bit
  interpreter.
- A `Registry` is used from one thread at a time, and a `Value` is not
  shared between threads without a lock of your own. A library may call a
  Python object it was handed from any thread; each call takes the GIL.
- A `Remote` cannot run a kind's default body for a slot an older table
  lacks, as a Rust caller does: it raises `guatiao.Unsupported`.
- `Registry.unload` unmaps a library once its own `unload` function
  agrees, and `collect` a retired one; neither can see a table or `ctx`
  you took with `provider_table`. Let those go first: they are valid
  until the library is closed, by `unload`, or once retired by
  `collect` or the registry's next load, retire or unload.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `Map`, `List`, `ABSENT`, `Registry`, `Instance`, `redact`, the error classes |
| `guatiao.wire` | channel frames (`announce`, `send`, `close`) and a `Receiver` checking each value against its channel's schema |
| `guatiao.time` | dates, times, instants and exact lengths as `datetime` types; `Span`; `normalize` |
| `guatiao.registry` | the plugin host: load, scan, list, rank, configure, instantiate |
| `guatiao.kinds` | a kind as a typed `Protocol` class; call a provider's table; serve a table over a Python object |
| `guatiao.provide` | a module of provider classes as a library |
| `guatiao.ffi` | for a package over another guatiao library: the loader, the C structs, a tree crossing as its root node |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/guatiao/src/guatiao/AGENTS.md) is
the full API reference and ships inside the wheel. The C ABI is described
by [`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
pip install -e "bindings/python/guatiao[dev]"     # from the repository root, in a virtual environment
pytest bindings/python/guatiao/tests -rs
```

The tests need the library and the repository's example plugins in one
directory. `conftest.py` points `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset; the repository's
own `AGENTS.md` says how to produce it. With everything present nothing
skips; a test that needs a missing library skips and says which. CI runs
the suite on Python 3.9 and 3.14.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
