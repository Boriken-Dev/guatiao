# Providers written in JavaScript, served without Node

Two libraries for the [guatiao-source-js](../../crates/guatiao-source-js/) source, which
runs them with QuickJS built into one file:

- `quickjs_greeter/`: `Hello` serves the `greeter` and `counter` kinds
  (its `start` hands back a conversation that tells the listener it was
  given), `Polite` the `greeter` kind. **The same `greeters.js` runs under
  Node**, through the Node binding's `libraryOf`: it uses nothing the two
  do not share.
- `quickjs_trees/`: `Trees` takes a tree handed over and a tree lent (`greet_from`,
  `check`) as plain data, which the Node binding hands over differently, so
  it is for this source only.

Each is a package (`package.json` names it and gives its version), a module
and a manifest, `*.guatiao.toml` of type `js`, which the source reads before it runs
anything. The kinds are loaded from the module `guatiao-kind` wrote from
their description, [`../greeter_kind/node/greeter_kind.js`](../greeter_kind/node/greeter_kind.js).

## Run it

From the repository root, with the guatiao library built:

```bash
cargo build -p guatiao --features load
cargo build --manifest-path crates/guatiao-source-js/Cargo.toml        # libguatiao_source_js.so
export GUATIAO_LIBRARY=$PWD/target/debug
cargo test --manifest-path crates/guatiao-source-js/Cargo.toml          # Rust and C hosts, and Node
```

The Python host, with `guatiao` installed:

```bash
python -m pytest bindings/python/guatiao/tests/test_js_source.py -rs
```

The same greeter under Node, through the example host (Node 22.18 or
later, after `npm ci` in `bindings/node` and `examples`, and `npm run
build` in `bindings/node/guatiao`):

```bash
node examples/node_host/host.ts --node-library examples/js_provider/quickjs_greeter/greeters.js
```
