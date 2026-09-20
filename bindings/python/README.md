# guatiao for Python

Python bindings for [guatiao](https://github.com/Boriken-Dev/guatiao/): one
value model for passing data between languages, a JSON Schema that
describes a value, and a registry that loads plugin libraries.

The package is pure Python over `ctypes`. It has no dependencies and
compiles nothing; it drives the same C ABI a C, C++ or Dart program
uses. With it a Python program can:

- build, read and change a guatiao value tree, and convert it to and from
  plain `dict`/`list` data;
- be a **host**: scan a directory for plugin libraries, list the
  providers they offer, build an instance from a configuration, and call
  a provider through its function table;
- read and write JSON, TOML and YAML, and check a form against its
  schema, through the `guatiao-serde` and `guatiao-form` libraries.

Python 3.9 or newer. Windows, Linux and macOS.

## Get the native library

The package does not bundle the native library yet, so build it from a
checkout of the repository:

```bash
cargo build --workspace --all-features
```

That leaves `guatiao`, `guatiao_serde` and `guatiao_form` in
`target/debug/` (`.dll`, `lib*.so` or `lib*.dylib`). `--all-features`
matters: the registry functions are only exported with the `load`
feature, and TOML and YAML only with theirs.

Then tell the package where they are:

```bash
export GUATIAO_LIBRARY=/path/to/guatiao/target/debug     # a directory, or the file itself
```

Without the variable the package tries the system's library search
(`ctypes.util.find_library`) and then its own `_native/` directory.
Nothing is loaded at import time: `import guatiao` always succeeds, and
the first native call raises `guatiao.LibraryNotFound`, naming every
place it looked, if there is nothing to load. `guatiao_serde` and
`guatiao_form` are found the same way, and only when `guatiao.serde` or
`guatiao.form` is first used.

## Install

```bash
pip install -e "bindings/python"          # from the repository root
pip install -e "bindings/python[dev]"     # with pytest and build
```

## Values

```python
from guatiao import Value

with Value.from_python({"host": "10.0.0.1", "port": 5900, "tags": ["a", "b"]}) as v:
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

`None` becomes null, `bytes` the bytes kind, a `dict` with `str` keys a
map. A missing value reads back as `guatiao.ABSENT`, which is falsy and
is not `None`. A `float` that is `nan` or infinite raises `ValueError`.

## Hosting plugins

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

`tests/greeter_table.py` and `tests/test_greeter.py` are a complete
worked example against `examples/hello_library`.

## JSON, TOML, YAML and forms

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

## Errors

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
- **JSON output loses the exact text of a non-integer number**: `1.10`
  is written as an object instead of a bare number. Integers, TOML and
  YAML are unaffected. This is a defect in `guatiao-serde`, pinned here
  by an expected-failure test.
- `List.insert` and `list[i] = x` rebuild the list, because the C ABI
  only appends and removes. `append` and `del list[i]` are direct.
- A `Registry` is used from one thread at a time, and a `Value` is not
  shared between threads without a lock of your own.
- No wheel carries the native library yet.

## Tests

```bash
cargo build --workspace --all-features
pytest bindings/python/tests -rs
```

`conftest.py` points `GUATIAO_LIBRARY` at the repository's `target/debug`
when the variable is unset. After a full build nothing should skip; a
test that needs a missing library skips and says which.

## More

[`src/guatiao/AGENTS.md`](src/guatiao/AGENTS.md) is the full API
reference, and ships inside the wheel. The C ABI itself is documented in
the repository's `crates/guatiao/AGENTS.md` and `include/guatiao.h`.

Licensed under MPL 2.0, as the rest of the repository.
