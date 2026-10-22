# A plugin host in Python

`host.py` loads greeter libraries written in any language and calls each
one through the `greeter` kind's table: a map back, a map lent and
changed in place, a map handed over, an object handed in and one handed
back (a conversation), an object lent for one call (a listener). A
provider that builds instances from a configuration is given one its
schema refuses, then one it accepts. It closes everything it opened and
prints what it did.

The kinds are `typing.Protocol` classes in
[`../greeter_kind/python/greeter_kind.py`](../greeter_kind/python/greeter_kind.py),
written by hand from the kind's C header
([`../greeter_kind/include/greeter_kind.h`](../greeter_kind/include/greeter_kind.h)):
each method in table order, its shapes as annotations, each
`FLOOR_HASH` copied from its `#define`. Nothing generates them.

## Run it

From the repository root, after building the workspace:

```bash
cargo build --workspace --all-features
export GUATIAO_LIBRARY=target/debug
export PYTHONPATH=bindings/python/guatiao/src        # or: pip install -e bindings/python/guatiao
python examples/python_host/host.py target/debug
```

`target/debug` holds the Rust greeters (`derived_greeter`, written with
the derive macros) and `hello_library`, whose table is not this kind's,
which the host reports and passes over.

To add the greeter written in C, build it into a directory of its own
and name that directory too:

```bash
mkdir -p c-greeter
cc -shared -fPIC -std=c11 -I crates/guatiao/include -I examples/greeter_kind/include \
   -o c-greeter/libc_greeter.so crates/guatiao/tests/c_consumer/greeter_in_c.c
python examples/python_host/host.py target/debug c-greeter
```

On Windows the output is `c-greeter/c_greeter.dll`, built with
`clang -shared` and no `-fPIC`; on macOS, `c-greeter/libc_greeter.dylib`.

To add the greeters written in Python, through the `guatiao-py` bridge
(built as [`../python_provider/README.md`](../python_provider/README.md)
says), name the bridge and where the manifests are:

```bash
export PYTHONPATH=bindings/python/guatiao/src:examples/greeter_kind/python
python examples/python_host/host.py target/debug \
  --python-source crates/guatiao-py/target/debug/libguatiao_py.so \
  --python-dir examples/python_provider
```

## What it shows

- **One description of the kind serves every language.** The C greeter
  and the Rust ones are called through the same `Remote`;
  a table whose `FLOOR_HASH` is not the kind's is refused before
  anything in it runs.
- **A table may be older than the kind.** The C greeter fills four of
  six slots; `has(name)` asks before calling, and calling a missing slot
  raises `guatiao.Unsupported`.
- **A refusal crosses with its words**: `greet("")` raises
  `guatiao.BadValue` carrying the provider's message.
- **Objects cross both ways.** The listener is a plain Python class; the
  conversation a greeter hands back is closed with `with`, which calls
  its `destroy` slot.
- **Unmapped when the library agrees.** A Rust library counts what it
  handed out and agrees once nothing is alive; the C greeter has no say,
  so it is retired and stays mapped.
