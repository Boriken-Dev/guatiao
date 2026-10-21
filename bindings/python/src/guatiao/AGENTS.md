# guatiao (Python) — API header

ctypes bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships inside the wheel, so it names no repo path.

## Finding the native library

Importing `guatiao` touches no native library. A native call is resolved
lazily, on first use, in this order:

1. `GUATIAO_LIBRARY` — a file, or a directory holding
   `guatiao.dll` / `libguatiao.so` / `libguatiao.dylib`.
2. `ctypes.util.find_library("guatiao")`.

The package is pure Python and carries no native library.

`guatiao.use_library(path)` names the library instead, before the first
native call (a file whatever it is called, or a directory); it wins over
`GUATIAO_LIBRARY`, and `RuntimeError` when one is already loaded from
elsewhere.

Not found raises `guatiao.LibraryNotFound` naming both places it
looked. `guatiao_serde` and `guatiao_intake` resolve the same way,
independently, only when `guatiao.serde` / `guatiao.intake` is first used.

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
    table, size, ctx = reg.provider_table("acme_hello", kind="greeter")
    instance = reg.create("acme_shouter", {"prefix": "hey"})
    instance.close()
```

Also: `load_file`, `scan_path`, `register_entry(name, EntryFn(...))`, `libraries()`,
`provider(key)`, `available(kind)`, `provider_available(key)`,
`keyed_by`/`libraries_keyed_by`, `set_priority`/`priority`,
`provider_config(key)` (a borrowed, read-only `Ref` onto the schema --
writing through it raises `TypeError`, and it is never `.close()`d), `host()` (the raw `host_info` pointer, for driving
a `guatiao_library_entry` by hand). Every answer that is data comes back
as a plain `dict`/`list`, already freed.

`provider_table(key, kind="greeter")` answers the table the provider
speaks that kind through (its per-kind table, else its single table when
it claims the kind); `provider_table(key)` answers its single table only.
A null table and size 0 mean there is none.

`Registry.retire(library_key)` takes a library out of the registry and
leaves it mapped until nothing uses it: `Registry.collect()` closes every
retired library whose own `unload` function agrees and answers their
keys, as does the registry's next load, retire or unload; one that
refuses is asked again next time, one with no such function stays
mapped. An `Instance` holds its library open until it is closed, and
`close()` reaches the provider that built it even after a retire, or a
refresh that loaded a new build under the same key.
`Registry.unload(library_key)` unmaps it **when the
library agrees**: `Busy` while another library holds a pinned answer
from it, then its own `unload` function is asked, its promise that
what it can account for is released: `Busy` while an
instance or object it handed out is alive; `Unsupported` when it has no
such function; `WrongKind` when it is linked into the host rather than
mapped; `NotFound` for an unknown key. A refusal leaves it loaded.
`unload(key, unchecked=True)` is the caller insisting on a library with no
`unload` function. **Both take the LIBRARY key** -- `libraries_keyed_by`'s
template, `%id` by default -- not a provider key. Either way no table,
`ctx` or borrowed schema taken from the library is used again, after an
unload or a collect that closed it.

**Sources** -- libraries that come and go while the host runs:

```python
reg.add_source_file("/opt/app/guatiao_py.so", {"dirs": ["/opt/app/py"]})  # answers its name, the path
reg.watch_dir("/opt/app/plugins", rules="kind=greeter")
changed = reg.refresh()   # {"loaded": [...], "retired": [...], "skipped": [...], "refused": [...]}
```

`add_source_file(path, config=None) -> str` maps a file exporting
`guatiao_source` (a bridge to another language, say) and adds the source
it answers for `config`; the file is never unmapped and is not a library.
Raises with the source's words: `NotFound` (cannot map), `WrongKind` (no
source), `BadValue` (refused table, or already added), or the source's
own status. `watch_dir(directory, *, rules=None)`: each refresh loads a
library file that appeared (by `rules`), retires one that vanished, and
reloads one that changed. `refresh() -> dict`: `loaded` and `retired`
(`{source, id, stamp, key}`), `skipped` (`{source, id, skipped, ...}`),
`refused` (`{source, id?, why}`). A changed candidate loads before its
old build is retired, and an `Instance` of the old keeps working; a
library retired by hand stays out until its stamp changes; refusals are
part of the answer, never an exception. Nothing refreshes on its own.

`Registry.bind(key, kind)` (a kind class or a `kinds.Kind`) answers the provider as a `kinds.Remote`
calling its table with its own `ctx`; `Instance.bind(kind)` the same for
an instance. `NotFound` when it serves no table for the kind,
`kinds.FloorMismatch` when the table is not the one `kind` describes.

## Kinds: calling a table, serving one

A kind is a typed `Protocol` class written from its C header: a method per
slot, in the header's order, the shapes read from the annotations.

```python
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

greeter = reg.bind("acme_greeter", Greeter)    # the class stands for its Kind
greeter.greet("ana").to_python()     # {"greeting": "hello, ana"}
greeter.has("shout")                 # whether this table carries the slot
```

`class K(KindProtocol, Protocol, kind="name", floor_hash=N, is_object=False,
struct=None)`: `floor_hash` is the `<table>_FLOOR_HASH` the header
defines, as written there (never computed here); `struct` a hand-written
`ctypes.Structure` to check the methods against field by field. Listing
`Protocol` makes it a protocol for mypy: a plain object with the methods
is a `Listener`, and a provider class that inherits `Greeter` is checked
against its signatures. `kinds.kind_of(K)` (or `K.kind`) is the `Kind`
built; every API taking a `Kind` takes the class. `@required` marks a slot
every table fills (the kind's original methods: it sets the smallest
table accepted); `@fallible` a slot with a `guatiao_provider_error *err`
last (a Rust method answering `Result`).

**Annotations to shapes**, checked when the class is created (a
`TypeError` names the method):

| annotation | as an argument | as an answer |
| --- | --- | --- |
| `str` | `guatiao_str` | `guatiao_string *out` (owned text) |
| `bytes` / `bytearray` | `guatiao_bytes` / `guatiao_bytes_mut` | -- |
| `bool`, `float` (F64), `kinds.I8`..`I64`, `U8`..`U64`, `USIZE`, `ISIZE`, `F32` | the scalar | the scalar |
| `int` | refused: say its width | refused |
| `Map`, `Ref` / `Value` / `Optional[Ref]` | `const guatiao_map *`, `const guatiao_value *` (null for `None`) | -- |
| `kinds.MapAnswer` (or `Map`), `kinds.ListAnswer` (or `List`), `kinds.ValueAnswer` (or `Value`) | -- | `guatiao_map/list/value *out`, a carrier |
| `Taken[Map]`, `Taken[List]`, `Taken[Value]` | a tree handed over | -- |
| `Lent[Map]`, `Lent[List]`, `Lent[Ref]` | a tree lent (`&mut`) | -- |
| `Taken[K]` / `Lent[K]`, `K` an object kind class | `guatiao_object` / `const guatiao_object *` | -- |
| `K`, an object kind class | refused bare | `guatiao_object *out` |
| `None` | -- | no answer |

A kind class's annotations are read in its module: define it at module
level. A method may not be named `has`, `close`, `kind`, `ctx`, `size` or
start with `_`; a slot is found by position, so name it otherwise.

What crosses, each way:

| shape | caller passes / gets | a Python method receives / returns |
| --- | --- | --- |
| scalars | `bool`/`int`/`float`, range-checked (`OverflowError`) | the same |
| text in | `str` | `str` (`BadValue` if not UTF-8) |
| bytes / out-buffer | bytes-like / a writable buffer | `bytes` / a `bytearray`, copied back when it returns |
| borrowed `Ref`, `Map` | a `Value`, `Ref` or plain data | a read-only `Ref`/`Map`, valid for the call |
| `Taken[..]` tree | a `Value` (moved: left ABSENT), a `Ref` (cloned) or plain data | a `Map`/`List` over a whole tree it owns, or a `Value` |
| `Lent[..]` tree | a `Value` or `Ref`, changed where it stands | a writable `Map`/`List`/`Ref`, valid for the call; what it adds is built in the allocator the tree records |
| tree answer | a `Value` the caller closes | a `Value` or a view of a whole one (moved), another `Ref`, or plain data, built in the allocator the carrier names |
| text answer | `str` | `str` |
| `Taken[K]` | a Python object implementing `K`, or a `kinds.Object` | a `kinds.Object` the method owns |
| `Lent[K]` | a Python object, or a `kinds.Object` | a `kinds.Remote`, valid for the call |
| `K` answer | a `kinds.Object`; `close()` calls `destroy` once | a Python object implementing `K`, or a `kinds.Object` |

**A view lent for one call** (a borrowed or lent tree, a lent object)
raises `ValueError` if used after the call returned.

**A Python object as an object of a kind**: any object with a method per
slot, inheriting the kind class or not. The table is built once per kind
and class and kept for the process; a slot for a method the class does
not define itself (the kind class's own placeholder does not count) is
null, so a Rust caller runs the kind's default; a missing `required`
method is `TypeError`. Its `ctx` is held until the holder calls `destroy`
(handed over) or the call returns (lent); `kinds.live_objects()` counts
what is held.

**A refusal**: a Python method raises a `guatiao.GuatiaoError`
(`BadValue("nobody to greet")` carries `BadValue`'s status), and the
caller reads that status and message. `NotImplementedError` crosses as
`GUATIAO_ERR_UNSUPPORTED`; anything else as `GUATIAO_ERR_INTERNAL` with
`"TypeName: message"`. A slot that cannot fail has no `err`: the
exception is logged on `logging.getLogger("guatiao.kinds")` and the
status crosses alone; a Rust caller panics on it, as its proxy does for
any provider failing a method that cannot fail.

**A `Remote` calling a table**: each method of the kind is a method of
the `Remote` (untyped: `reg.bind` answers a `Remote`). A slot past the
table's size or null raises `guatiao.Unsupported` (the Rust proxy would
run the kind's default body, which Python cannot); a refusal raises the
status's `GuatiaoError` with the provider's words; a slot that cannot
fail and fails raises its status too. Valid while the library is: until
unloaded, or, once retired, collected.

**Threads**: a C caller may call a table this binding built from any
thread; ctypes takes the GIL for the call, so calls into Python are
serialised by it.

**By hand**: `Kind(name, struct=None, *, floor_hash, methods,
is_object=False)` with `Method(name, args=(), returns=None, *,
fallible=False, required=False)` and the shapes of `guatiao.kinds.shapes`
(`STR`, `MAP`, `taken(MAP)`, `lent(kind)`, ...) is the same description
without a class; the class builds one.

## Providers written in Python

**The module is the library.** Its id is the module's `__name__` (in full:
`pkg.greeters`), its version `__version__` (required, text), its providers
the `provide.Provider` subclasses it exports: those in `__all__`, in its
order (other names are ordinary exports), or without `__all__` those it
defines itself, in definition order. `__main__` is refused: import it by
name. Provider ids are `{library id}_{name}`.

```python
# greeters.py
from dataclasses import dataclass
from typing import Dict

from guatiao import BadValue, provide
from greeter_kind import Counter, Greeter

__version__ = "1.0.0"
__all__ = ["Hello", "Shouter"]

class Hello(provide.Provider, Greeter, Counter, display_name="Hello, in Python"):
    def greet(self, name: str) -> Dict[str, str]:
        if not name:
            raise BadValue("nobody to greet")
        return {"greeting": f"hello, {name}"}
    def count(self) -> int: ...

@dataclass(frozen=True)
class ShouterConfig:
    prefix: str

class Shouter(provide.Provider, Greeter):
    def __init__(self, config: ShouterConfig) -> None:
        self.prefix = config.prefix
    def greet(self, name: str) -> Dict[str, str]:
        return {"greeting": f"{self.prefix} {name.upper()}"}

reg.register_library(greeters)    # files greeters_hello, greeters_shouter
```

**`class P(provide.Provider, K1, K2, ...)`** serves every kind class it
inherits (provider kinds, not object kinds), through the methods it
defines; a method a kind requires and the class lacks is a `TypeError`
when the class is created, naming both. Class keywords: `named` (default
the class name in snake_case), `id`, `display_name`, `version` (empty:
the library's), `meta`, `default`, `config_schema`.

**Configuration**: an `__init__` taking nothing makes the provider its
one instance, built with the library. An `__init__` taking one `config`
builds an instance per configuration the host checked first; its schema
is read from the annotation, a dataclass or a `TypedDict` whose fields are
`str`, `int`, `float`, `bool`, `Literal[...]`, `Optional[T]`, `list[T]`,
`dict[str, T]`, nested dataclasses, with defaults (not required),
`NotRequired` and `Annotated[T, {"minimum": 0}]` keywords. The instance
receives the dataclass (or the dict, for a `TypedDict`). `config_schema=`
says a schema the types cannot, and the instance then receives plain
data. `default=True` also offers `cls()` when the configuration is
optional.

**`available()`**, a classmethod to override: `None` when the provider
can run here, else why not; asked every time a host asks, the reason
kept for the process.

**Meta**: the library's `meta` map merges, later winning, the installed
distribution's metadata (`summary`, `license`, `authors`, `urls`, and each
licence file as a `notices` entry, the convention every host shows), the
module's `__doc__` first paragraph (`description`), `__author__`,
`__license__` and `__copyright__`, and a module-level `_meta_` mapping of
plain data (anything else is a `TypeError`). A module not installed
contributes only the last two.

`provide.library_of(module) -> Library` builds a module's library, once
per module object. `provide.library(id, version, *, providers,
meta=None)` is the same without a module: for a test, a library made at
run time, or meta of its own. `Library`: `.entry` (a
`guatiao_library_entry`), `.info_address`, `.id`, `.version`, `.kinds`,
`.providers`, `.live_instances()` (built by `create` and not destroyed),
`.close()` (lets go of the providers' one instances; the entry answers
null from then on). Its `unload` slot answers `GUATIAO_ERR_BUSY` while an
instance is alive; objects its methods hand out are not counted.

`Registry.register_library(module_or_library)` files it under `<id>`,
keeps it alive with the registry, and is linked, not mapped: `unload`
answers `WrongKind`, `retire` takes it out.

`guatiao.kinds.table(table_ptr, size, struct_type, floor_hash=...)`
casts a `provider_table()` answer to a hand-written struct after
checking `size >= sizeof(struct_type)` and the `floor_hash`
(`kinds.FloorMismatch` otherwise), for calling slots with raw ctypes.

## serde: JSON, TOML, YAML

```python
from guatiao import serde

text = serde.dumps(value, format="json", pretty=True)   # or "toml", "yaml"
value = serde.loads(text, format="json")
```

`toml`/`yaml` raise `NotImplementedError` naming the feature when the
loaded `guatiao_serde` was not built with it. A TOML document must be a
map (`WrongKind` otherwise).

A document that does not parse raises `guatiao.errors.ParseError`: a
`BadValue` and a `ValueError` with `.format` (`"json"`, `"toml"` or
`"yaml"`), `.line` and `.column` (from 1, the column in characters; 0 when
the parser did not say where; for text that is not UTF-8, its first bad
byte) and `.message`, which never quotes the document.

A number keeps its exact text through JSON, with one exception the
parser owns: an exponent is read back with its sign written (`1e400`
parses as `1e+400`).

## intake

```python
from guatiao import intake

intake.check(schema, form_doc)             # None, or an error map
intake.layout(schema, form_doc)            # [{"section": ..., "fields": [...]}, ...]
intake.is_visible(schema, form_doc, "key", values)   # bool
intake.at(value, "agent[1].name")          # a borrowed Ref, or None
intake.for_schema(schema)                  # the form a schema implies (a Value)
intake.form_for(form_doc, schema, "path")  # a Value, or None
```

The flat projection keeps a value as `key -> text`, one entry per scalar
leaf (a tagged field's discriminant under its key and its payload under
`key.member`, a list as `agent[0].name`, an open map as `env[PATH]`):

```python
intake.resolve(schema, "auth.password")    # borrowed Ref onto the field, or None
intake.flat_keys(schema, "auth")           # list[str]; WrongKind for an unknown key
intake.flatten(schema, "auth", value)      # dict[str, str]; WrongKind when it does not apply
intake.unflatten(schema, "auth", store)    # Value (close it), or None; store is a
                                           # Mapping[str, str] or a Value/Ref onto one
```

Every `schema`, `form`, `value` and `values` argument is a `Value` or a
`Ref`. What `resolve` and `at` return is a read-only view: it keeps the
schema or value it came from alive, is found again from it on every
access, and raises `ValueError` once that is closed. Nothing can be
changed through it (`TypeError` on any write), and `.clone()` copies it
out. An empty container flattens to nothing, and an entry the schema
does not declare is dropped by `unflatten`.

`unflatten` answers `None` only for "nothing stored": no such field, no
entry under it, or a discriminant that names no arm. A `store` that is not
text to text -- a mapping, `Value` or `Ref` holding anything but `str`
values, or not a map at all -- raises `TypeError`.

## time: dates, times, instants, spans, exact lengths

The forms are text in a value (an exact length is `[seconds,
nanoseconds]`), fixed at <https://boriken-dev.github.io/guatiao/encodings/>.
Every text is read and written by the library's own functions.

```python
from guatiao import time as gt     # the module, not the standard library's

gt.parse_date("2026-10-21")                 # datetime.date
gt.format_date(date)                        # "2026-10-21"; a datetime is TypeError
gt.parse_time("13:45:00.5")                 # datetime.time, no tzinfo
gt.format_time(time)                        # a tzinfo is ValueError
gt.parse_datetime("2026-10-21T13:45:00.5")  # naive datetime.datetime
gt.format_datetime(naive)                   # an aware one is ValueError
gt.parse_timestamp("2026-10-21T19:15:00+05:30")   # aware, UTC: 13:45:00+00:00
gt.format_timestamp(aware)                  # "...Z"; a naive one is ValueError
gt.normalize(form, text)                    # canonical text, every nanosecond kept;
                                            # form: date|time|datetime|timestamp|span
gt.to_timedelta(v)                          # [s, ns], seconds (int/Decimal/float),
                                            # {"secs", "nanos"}; a Value, Ref or plain data
gt.from_timedelta(delta)                    # [seconds, nanoseconds]; negative is ValueError

span = gt.Span(days=1, hours=2)             # keyword units, one sign: Span(days=-1, hours=-2)
gt.Span.parse("1h 30m"); gt.Span.parse("P1DT2H")
str(span)                                   # "P1DT2H", canonical ISO 8601
span.years .. span.nanoseconds              # ints carrying the sign
span.is_zero(); span.is_negative(); -span   # immutable, hashable, == unit by unit
span.add_to(date_or_naive_datetime)         # same type back
```

- **Text the grammar refuses raises `guatiao.Unparsed`** (a `BadValue`
  and a `ValueError`) with `.form`, `.at` (the byte offset in the UTF-8
  text) and `.expected`; it never quotes the text.
- **Below a microsecond is dropped**, toward the past, reading into a
  `datetime`, `time` or `timedelta`; writing one is exact. `normalize` and
  `Span` keep nanoseconds.
- **Year 0000** is valid text but no `datetime` holds it: `parse_date`,
  `parse_datetime` and `parse_timestamp` raise `ValueError` for it.
- **`to_timedelta`** raises `BadValue` for anything not an exact length
  and `OverflowError` past 999999999 days.
- **`Span(...)`** raises `BadValue` for fields of both signs, a field past
  a signed 64-bit integer or `nanoseconds` of a second or more, and
  `TypeError` for a non-`int`. `Span(days=1) != Span(hours=24)`.
- **`add_to`**: years and months first (a day past the month's end
  becomes its last day), then weeks and days, then hours to nanoseconds
  on a `datetime`, carried across midnight. `BadValue` for time units on
  a `date` or a result outside the years 1 to 9999; `ValueError` for an
  aware `datetime`.

## Secrets

A secret crosses as the value it holds; its schema marks it with
`writeOnly: true` and `x-sensitive: true`.

```python
from guatiao import redact

shown = redact(schema, value, placeholder="[redacted]")   # a new Value; close it
```

`schema` and `value` are `Value`s, `Ref`s or plain data. Every field a
schema marks (`x-sensitive` or `writeOnly`) is replaced at any depth, and
whatever the schema does not describe, an undeclared key say, is hidden
too. Local `$ref`s are followed. `WrongKind` for a schema that is not a
map.

## Errors

Every failing `guatiao_status` raises `guatiao.GuatiaoError` (`.status`
the raw code, `.message` a provider's own words when there were any), as
one of its subclasses: `BadValue`, `AllocFailed`, `WrongKind`,
`NotFound`, `NullArgument`, `Gone`, `Unsupported`, `Busy`, `Internal`.
Each takes `(status, message)`, or a message alone and its own status
(`BadValue("why")`).
They pickle, `ParseError` and `Unparsed` included. A refusal the library words itself
(a `WrongKind` from `serde.loads`, say) carries those words in `.message`.

## Threading

A `Registry` is used from one thread at a time, like the C handle
it wraps. A `Value` is not shared across threads without the
caller's own lock.
