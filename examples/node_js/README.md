# Providers and a host in plain JavaScript

Node runs these files as they are: no TypeScript, no build step.

- `greeters.js` is a library, the package `js_greeter`: `Hello` serves
  the `greeter` and `counter` kinds, `Parrot` the `echo` kind.
- `echo_kind.js` defines two kinds in JavaScript, `echo` and the object
  kind `page`, each a class extending `Kind` whose static fields say its
  name, floor hash and slots. That is only for a kind with nothing behind
  it.
- `describe.js` writes those two kinds as data, `echo_kind.kinds.json`:
  the description a Rust kind crate commits beside its header, which
  `kindsOf()` loads and Dart's `dart run guatiao:kind` writes classes
  from. `node describe.js --check` fails when the file is stale.
- The greeter kinds exist already, in Rust: they are loaded from the
  module `guatiao-kind` wrote from their description
  ([`../greeter_kind/node/greeter_kind.js`](../greeter_kind/node/greeter_kind.js)),
  and declared nowhere here.
- `main.js` is a host: it registers the library, calls every greeter it
  finds through the `greeter` kind, and lends a page to `echo`.

A provider missing a method its kind requires is a `TypeError` naming the
method and the kind when the library is built. With `// @ts-check` at the
top of a file, an editor also checks each `@implements` class against the
kind's interface, from the generated `greeter_kind.d.ts`.

## Run it

From the repository root, with Node 22 or later, after building the
workspace, the Node package and the examples' one dependency on it:

```bash
cargo build --workspace --all-features
(cd bindings/node && npm ci) && (cd bindings/node/guatiao && npm run build)
(cd examples && npm ci)
export GUATIAO_LIBRARY=target/debug
node examples/node_js/main.js target/debug
```

`target/debug` adds the greeter written in Rust. The TypeScript host,
[`../node_host`](../node_host/), loads this library too:
`--node-library examples/node_js/greeters.js`.
