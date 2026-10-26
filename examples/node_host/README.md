# A plugin host in TypeScript

`host.ts` loads greeter libraries written in any language and calls each
one through the `greeter` kind's table: a map back, a map lent and
changed in place, a map handed over, an object handed in and one handed
back (a conversation), an object lent for one call (a listener). A
provider that builds instances from a configuration is given one its
schema refuses, then one it accepts. It closes everything it opened and
prints what it did.

The kinds come from
[`../greeter_kind/node/greeter_kind.js`](../greeter_kind/node/greeter_kind.js)
and its `.d.ts`, which `guatiao-kind` wrote from the kinds' description
([`../greeter_kind/include/greeter_kind.kinds.json`](../greeter_kind/include/greeter_kind.kinds.json)),
committed beside their C header. Nothing here says what a slot takes.

## Run it

From the repository root, with Node 22.18 or later (it runs the
TypeScript as it is), after building the workspace, the Node package and
the examples' one dependency on it:

```bash
cargo build --workspace --all-features
(cd bindings/node && npm ci) && (cd bindings/node/guatiao && npm run build)
(cd examples && npm ci)
export GUATIAO_LIBRARY=target/debug
node examples/node_host/host.ts target/debug \
  --node-library examples/node_provider/node_greeter/greeters.ts
```

`target/debug` holds the Rust greeters (`derived_greeter`, written with
the derive macros) and `hello_library`, whose table is not this kind's,
which the host reports and passes over. `--node-library` registers the
greeters written in TypeScript ([`../node_provider`](../node_provider/)).

To add the greeter written in C, build it into a directory of its own
and name that directory too:

```bash
mkdir -p c-greeter
cc -shared -fPIC -std=c11 -I crates/guatiao/include -I examples/greeter_kind/include \
   -o c-greeter/libc_greeter.so crates/guatiao/tests/c_consumer/greeter_in_c.c
node examples/node_host/host.ts target/debug c-greeter
```

On Windows the output is `c-greeter/c_greeter.dll`, built with
`clang -shared` and no `-fPIC`; on macOS, `c-greeter/libc_greeter.dylib`.

To add the greeters written in Python, through the `guatiao-source-py` bridge
(built as [`../python_provider/README.md`](../python_provider/README.md)
says), name the bridge, where its manifests are, and what they import:

```bash
node examples/node_host/host.ts target/debug \
  --python-source crates/guatiao-source-py/target/debug/libguatiao_source_py.so \
  --python-dir examples/python_provider \
  --python-path bindings/python/guatiao/src --python-path examples/greeter_kind/python
```

## What it shows

- **One description of the kind serves every language.** The Rust, C,
  Python and TypeScript greeters are called through the same `Remote`; a
  table whose `FLOOR_HASH` is not the kind's is refused before anything
  in it runs.
- **A provider may leave optional slots out.** The C greeter fills four of
  six slots; `has(name)` asks before calling, and calling a missing slot
  throws `Unsupported`.
- **A refusal crosses with its words**: `greet('')` throws `BadValue`
  carrying the provider's message.
- **Objects cross both ways.** The listener is a plain class; the
  conversation a greeter hands back is closed, which calls its `destroy`
  slot.
- **Unmapped when the library agrees.** A Rust library counts what it
  handed out and agrees once nothing is alive; the C greeter has no say,
  and a library written in TypeScript is linked rather than mapped, so
  each is retired instead.
