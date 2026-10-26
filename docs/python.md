# Python: host and provider

Python plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a library
that a Python host registers directly, and that the `guatiao-source-py` bridge
serves to a host in Rust, C, Dart or Python. Both use the `guatiao`
package, ctypes bindings over the C ABI, and the same kind classes.

```bash
pip install guatiao
```

The package loads the guatiao library on its first native call: the one
`guatiao.use_library(path)` named, else `GUATIAO_LIBRARY` (a file or a
directory), else the system's library search.

## A kind, once

A kind is the table its C header declares. Python writes it once as a
typed `Protocol` class: its methods in table order, their shapes as
annotations, the header's `FLOOR_HASH` copied from its `#define`.

```python
# greeter_kind.py
from typing import Protocol

from guatiao import Map
from guatiao.kinds import I64, KindProtocol, Lent, MapAnswer, Taken, fallible, required

class Listener(KindProtocol, Protocol, kind="listener", floor_hash=1867921573, is_object=True):
    @required
    def heard(self, what: str) -> None: ...

class Greeter(KindProtocol, Protocol, kind="greeter", floor_hash=2695856491):
    @required
    @fallible
    def greet(self, name: str) -> MapAnswer: ...
    def shout(self, name: str) -> str: ...
    @fallible
    def greet_into(self, name: str, listener: Lent[Listener]) -> None: ...
    @fallible
    def check(self, config: Lent[Map]) -> bool: ...

class Counter(KindProtocol, Protocol, kind="counter", floor_hash=1227528937):
    @required
    def count(self) -> I64: ...
```

- `@required` marks the slots every table fills; a table shorter than
  them is refused. `@fallible` marks a slot that can refuse, with a
  `guatiao_provider_error *err` last.
- An integer says its width (`I8` to `I64`, `U8` to `U64`, `USIZE`,
  `ISIZE`), as C does; a bare `int` is a `TypeError`.
- `Lent[...]` is a tree or object lent for the call and changed where it
  stands; `Taken[...]` one handed over; `MapAnswer` a tree answered.
- A mistake is a `TypeError` when the class is created, naming the
  method; mypy and pyright check the code that calls or implements it.

The binding's
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/guatiao/src/guatiao/AGENTS.md)
lists every annotation and the C type it stands for.

A kind that has a description (the `<crate>.kinds.json` a kind crate
commits beside its header) need not be written by hand: `python -m
guatiao.kinds greeter_kind.kinds.json -o greeter_kind.py` writes these
classes from it, docs included (`--check` says whether a committed module
is current), and `kinds.kinds_of(document)` loads the same kinds at run
time. A library built with `#[derive(Provider)]` carries the document
itself, so a host can call it with nothing on its side:

```python
from guatiao import kinds

carried = kinds.kinds_of(reg.kind_descriptions("derived_greeter_hello"))
greeter = reg.bind("derived_greeter_hello", carried["greeter"])
```

## As a host

```python
from guatiao import Registry, Value
from greeter_kind import Greeter

with Registry("my-host", "1.0") as reg:
    reg.scan_dir("/opt/myapp/plugins", rules="kind=greeter")
    for p in reg.providers("greeter"):            # best first
        greeter = reg.bind(p["key"], Greeter)
        print(greeter.greet("ana").to_python())    # {'greeting': 'hello, ana'}
        with Value({"name": "bo"}) as config:
            greeter.check(config)                  # filled in, in place
```

`bind` checks the table's size and floor hash before anything in it
runs, and answers a `Remote` whose methods take and return Python values.
A refusal raises the status's error class (`guatiao.BadValue`, ...) with
the provider's own words; a slot an older table lacks raises
`guatiao.Unsupported`. An object a slot takes, such as a listener, is any
Python object with the kind's methods; an object a slot answers is closed
with `with`. A provider configured per instance is built with
`reg.create(key, config)`, after the registry checked the configuration
against its schema.

A **source** serves libraries that come and go while the host runs:

```python
reg.add_source_file("/opt/myapp/libguatiao_source_py.so", {"dirs": ["/opt/myapp/python"]})
reg.watch_dir("/opt/myapp/plugins", rules="kind=greeter")
changed = reg.refresh()     # {"loaded": [...], "retired": [...], "skipped": [...], "refused": [...]}
```

Nothing runs on its own: call `refresh()` when you want the registry to
agree with disk.

## As a provider

A module is a library. Its `__name__` is the library's id, its
`__version__` its version, and the provider classes it exports (those in
`__all__`, or without it those it defines) its providers. A provider is a
class inheriting `guatiao.provide.Provider` and the kinds it serves:

```python
# greeters.py
"""Greeters written in Python."""
from dataclasses import dataclass
from typing import Dict

from guatiao import BadValue, provide
from greeter_kind import Counter, Greeter

__version__ = "1.0.0"
__all__ = ["Hello", "Shouter"]

class Hello(provide.Provider, Greeter, Counter, display_name="Hello, in Python"):
    def __init__(self) -> None:
        self.asked = 0
    def greet(self, name: str) -> Dict[str, str]:
        if not name:
            raise BadValue("nobody to greet")
        self.asked += 1
        return {"greeting": f"hello, {name}"}
    def count(self) -> int:
        return self.asked

@dataclass(frozen=True)
class ShouterConfig:
    prefix: str = "hey"

class Shouter(provide.Provider, Greeter):
    def __init__(self, config: ShouterConfig) -> None:
        self.prefix = config.prefix
    def greet(self, name: str) -> Dict[str, str]:
        return {"greeting": f"{self.prefix} {name.upper()}"}
```

- **Ids.** A provider's name is its class name in snake_case (`named=`
  says otherwise), and its key `{library}_{name}`: `greeters_hello`.
- **Checked at class creation.** A method a kind requires and the class
  lacks is a `TypeError` naming both. A slot the class leaves out is
  null, and a Rust caller runs the kind's default for it.
- **One instance, or one per configuration.** An `__init__` taking
  nothing makes the provider its one instance. One taking a `config`
  annotated with a dataclass or a `TypedDict` declares the schema hosts
  check a configuration against, and receives the configuration as that
  type. `config_schema=` states a schema the types cannot.
- **Refusing.** Raise a `guatiao.GuatiaoError` (`BadValue("...")`): the
  caller reads its status and words. Any other exception crosses as
  `GUATIAO_ERR_INTERNAL` naming it.
- **Available or not.** Override the classmethod `available()` to answer
  why the provider cannot run here, or `None`.
- **Meta.** The library's `meta` is merged from the installed
  distribution's metadata (summary, licence, authors, project URLs, each
  licence file as a `notices` entry), then the module's docstring,
  `__author__`, `__license__` and `__copyright__`, then a `_meta_` dict
  of plain data, the later winning.

A Python host registers the module itself:

```python
import greeters

reg.register_library(greeters)
reg.bind("greeters_hello", Greeter).greet("ana")
```

## Serving Python to any host

`guatiao-source-py` is a source file, built from this repository against the
Python it will run with:

```bash
PYO3_PYTHON=python3 cargo build --release --manifest-path crates/guatiao-source-py/Cargo.toml
```

Each library beside it gets a manifest, `greeters.guatiao.toml`:

```toml
type = "python"                     # the source that serves it
module = "greeters"                 # the library: its id, imported by this name
kinds = ["greeter", "counter"]      # checked against what the module serves
paths = ["."]                       # put on sys.path, relative to this file
```

A host in any language adds the bridge once, naming where the manifests
are (`dirs`, `manifests`) and what goes on `sys.path` for every library
(`paths`: where `guatiao` and the kind modules are, unless installed).
`refresh()` lists the manifests without importing anything, imports a
new or changed module, and closes an old build once nothing holds it, so
an edited `.py` file is served at the next refresh while instances of the
old build still answer.

| OS | one bridge build serves |
| --- | --- |
| Windows | every CPython from 3.9 on (it links `python3.dll`, found through `PATH` by a host that is not Python) |
| Linux, macOS | the CPython minor it was built with: build one per minor |

[`examples/python_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/python_provider)
holds two libraries and the commands that serve them to the Rust and
Python example hosts. `guatiao-source-py` is CPython as the engine of a
[language bridge](language-bridges.md): the manifests, stamps and refresh
are the bridge's.

## What Python cannot do

- **Run a kind's default body.** A Rust caller runs one for a slot an
  older table lacks; a Python caller raises `guatiao.Unsupported`.
- **Run calls in parallel.** Every call into Python takes the GIL.
- **Keep libraries' modules apart.** The bridge keeps one interpreter for
  the program: libraries share `sys.modules`, so two that need different
  versions of one package need separate processes, and a compiled
  extension module stays loaded when its library closes.
- **Run in a browser, or on 32-bit x86.** The package needs a CPython
  that can load a native library, 64-bit or 32-bit ARM.
