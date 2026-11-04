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

class Greeter(KindProtocol, Protocol, kind="greeter", floor_hash=2032737757):
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
the provider's own words; a slot the provider leaves out raises
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
for note in changed.get("notes", []):
    print(f"{note['source']}: {note['text']}")
```

Nothing runs on its own: call `refresh()` when you want the registry to
agree with disk. `notes` is there only when a source has something worth
showing that is neither a refusal nor a skip, such as the TLS settings a
remote source's policy weakens; each is told once, by the first refresh
that finds it.

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

Each library beside it gets a manifest, `greeters.guatiao`:

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
| Windows | every CPython from 3.9 on (it links `python3.dll`: a host that is not Python names the Python installation in the bridge's `import_dirs`, or puts it on `PATH` and sets its registry's import search to `path`) |
| Linux, macOS | the CPython minor it was built with: build one per minor |

The interpreter the bridge starts in a host that is not Python takes
nothing from the environment by default. Its configuration's `isolation`
says how much it takes:

| `isolation` | the interpreter reads |
| --- | --- |
| `shipped` (default) | what is shipped beside the host's executable (a `._pth`, a `pyvenv.cfg`) and the configuration's `home`; no `PYTHONPATH`, `PYTHONHOME` or user site-packages |
| `inherit` | the environment as well, as a `python` started there would |
| `isolated` | only the configuration's `home` (else the installation libpython is in) and `paths` |

`inherit`, and the registry's import search `path` that reaches
`python3.dll` through `PATH`, are the weaker settings; naming the
installation in the bridge's `import_dirs` keeps the default search. A
source cannot yet say to its host, in every language, that it runs under
one: that waits for a channel from a source to its host.

An interpreter already running (a Python host's own) is joined as it is:
asking it for `isolated`, or naming `shipped`, when its flags do not hold
to that is refused rather than ignored.
[The bridge's reference](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-source-py/AGENTS.md)
has what each Python version on Windows can keep out.

[`examples/python_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/python_provider)
holds two libraries and the commands that serve them to the Rust and
Python example hosts. `guatiao-source-py` is CPython as the engine of a
[language bridge](language-bridges.md): the manifests, stamps and refresh
are the bridge's.

## Secrets

A secret lives in the wiping allocator, which overwrites every block
before releasing it:

```python
from guatiao import Value, wiping_alloc

token = Value({"user": "alice", "token": "t0ken"}, alloc=wiping_alloc())
repr(token)                                   # '<guatiao.Value [sensitive]>'
with token.as_map()["token"].reveal() as view:  # a read-only memoryview, no copy
    use(view)
token.close()                                 # overwritten, then freed
```

- `Ref.is_sensitive` says whether any part of a tree lives there;
  `repr` then shows `[sensitive]`. `Value.wipe()` overwrites a tree in
  any allocator and frees it.
- The view `reveal()` answers, and every slice of it, holds the value
  alive: while any lives, `close()`, `wipe()`, handing the value to a
  slot, lending it and every change to it raise `BufferError`, so none
  reads memory that was freed or moved. Each of those holds the value
  while it runs (a lend for the whole call), and meanwhile a `reveal()`,
  a read, a close, a wipe or a change from any thread raises
  `BufferError`: a provider holding a value its caller lent can neither
  reveal, read nor change it. Of two threads closing one value, one frees
  it and the other's `close()` raises `BufferError`; a closed value's
  `close()` does nothing.
- A read (`to_python()`, `clone()`, `len()`, `m[key]`, `key in m`,
  `to_wire()`, `tag`, ...) shares the value with every other read while
  it runs, and a call borrowing the value reads it for the whole call;
  meanwhile a close, a wipe, a change, lending it and handing it over
  raise `BufferError`, from any thread. So one call never has a value
  lent and borrowed, or lent twice: for a slot
  `f(&self, a: &mut Map, b: &Map)`, `remote.f(v, v)` raises before the
  call, in either order, as it does for a node of `v` and `v` itself. A
  claim holds the whole value, so lending `v["a"]` and borrowing `v["b"]`
  in one call is refused too, though the two do not overlap: copy one
  first (`clone()`). A view `guatiao_intake.at(v, ...)` answers counts as
  `v`, for reads and for calls. A call's views are held the same way, so
  a provider cannot pass what it was lent on as both. Nothing waits for a
  claim, so nothing deadlocks.
  A package reading a value through `guatiao.ffi.node_of`
  (`guatiao-serde`, `guatiao-intake`) takes no claim: close or change a
  value only once no other thread is passing it to one of them.
- Of an argument a call lends or borrows to a provider, which the caller
  frees once the call returns, `reveal()` reads a copy in the wiping
  allocator, wiped when its last view and slice go: a slice the provider
  keeps stays readable and is the provider's.
- A slice kept in a reference cycle holds its value until the garbage
  collector frees the cycle, and `close()` raises until then. A view
  still alive when the interpreter exits keeps its tree from being freed,
  and so from being wiped, until the process ends.
- A slot its kind marks `@sensitive` (`kinds.sensitive`, or
  `"sensitive": true` in the description) answers a `Value` kept in the
  wiping allocator, a text answer included (`kinds.SensitiveText`), never
  a `str`.
- **What Python cannot wipe**: a `str` or `bytes`. `to_python()`,
  `bytes(view)` or `str(...)` makes one, and Python keeps it until it
  collects it, possibly interned; a provider's own `str` is the same. Keep
  a secret in a `Value` and read it through `reveal()`.

## What Python cannot do

- **Run a kind's default body.** A Rust caller runs one for a slot a
  provider leaves out; a Python caller raises `guatiao.Unsupported`.
- **Run calls in parallel.** Every call into Python takes the GIL.
- **Keep libraries' modules apart.** The bridge keeps one interpreter for
  the program: libraries share `sys.modules`, so two that need different
  versions of one package need separate processes, and a compiled
  extension module stays loaded when its library closes.
- **Run in a browser, or on 32-bit x86.** The package needs a CPython
  that can load a native library, 64-bit or 32-bit ARM.
