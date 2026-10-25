# Providers written in Python

Two guatiao libraries written in Python, each a module and a manifest:

- `py_greeter/` serves the `greeter` and `counter` kinds: `Hello`, its
  one instance, and `Shouter`, built per configuration from the
  `ShouterConfig` dataclass its `__init__` takes. Its manifest leaves the
  version to the module's `__version__`.
- `py_tally/` serves `counter` with `Tally`, configured by a `TypedDict`
  whose every key is optional. Its manifest names the version itself.

The kinds are the Python classes in
[`../greeter_kind/python/greeter_kind.py`](../greeter_kind/python/greeter_kind.py),
written from the kind's C header. Each module is a library as the
binding reads one: `__name__` is its id, `__version__` its version, the
provider classes in `__all__` (or those it defines) its providers.

A Python host imports a module and registers it directly. A host in any
other language reaches the same modules through the `guatiao-source-py` bridge,
a source it adds once: the bridge reads the manifests without importing
anything, imports a module when the registry asks for it, and drops it
again once nothing holds it, so a `refresh()` after an edit serves the
new code.

## A manifest

```toml
# py_greeter.guatiao-py.toml
module = "py_greeter"               # the library: its id, imported by this name
kinds = ["greeter", "counter"]      # checked against what the module serves
paths = ["."]                       # put on sys.path, relative to this file
# version = "1.0.0"                 # optional: else a literal __version__
```

## Run it

From the repository root. Build the guatiao library and the Rust example
host, then the bridge against the Python you will run it with:

```bash
cargo build --workspace --all-features
PYO3_PYTHON=python3 cargo build --manifest-path crates/guatiao-source-py/Cargo.toml
```

The bridge is `crates/guatiao-source-py/target/debug/libguatiao_source_py.so`
(`guatiao_source_py.dll` on Windows, `libguatiao_source_py.dylib` on macOS). The
Python libraries import `guatiao` and `greeter_kind`, so the source's
configuration puts both on `sys.path` (an installed `guatiao` package
needs only the second).

**From the Rust host:**

```bash
GREETER_HOST_SOURCE=crates/guatiao-source-py/target/debug/libguatiao_source_py.so \
GREETER_HOST_SOURCE_CONFIG='{"dirs": ["examples/python_provider"],
  "paths": ["bindings/python/guatiao/src", "examples/greeter_kind/python"]}' \
  cargo run -p greeter_host
```

It prints `served py_greeter as py_greeter` and `served py_tally as
py_tally`, then calls `py_greeter_hello` and `py_greeter_shouter` as it
calls the greeters written in Rust and C.

**From the Python host**, which serves the bridge from its own
interpreter:

```bash
export GUATIAO_LIBRARY=target/debug
export PYTHONPATH=bindings/python/guatiao/src:examples/greeter_kind/python
python examples/python_host/host.py target/debug \
  --python-source crates/guatiao-source-py/target/debug/libguatiao_source_py.so \
  --python-dir examples/python_provider
```

**On Windows**, a host that is not Python finds the interpreter through
`PATH`: put the directory holding `python3.dll` there first (`py -3 -c
"import sys; print(sys.base_prefix)"` prints it). `PYTHONPATH` takes
`;` between entries.

## What it shows

- **One module, two ways in.** The same `py_greeter` is registered
  directly by a Python host and served by the bridge to a Rust or C one.
- **Edit, refresh, served.** Change `GREETING` in `py_greeter.py` and
  call `refresh()`: the next instance greets the new way, while one built
  before still answers with the old code, and the old build closes when
  it is released.
- **Checked before served.** A manifest whose module, version or kinds do
  not match what the module declares is refused, and the refusal names
  both.
