# guatiao-js

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

A [guatiao](https://crates.io/crates/guatiao) **source of libraries
written in JavaScript**, run by [QuickJS](https://github.com/quickjs-ng/quickjs)
built into the file: any guatiao host, in Rust, C, Python or any
language with a binding, adds this one file and calls providers written
in JavaScript as it calls the ones written in Rust or C. The host needs no
Node, no browser and no JavaScript engine of its own; it suits a program
that wants JavaScript scripts and cannot ship Node (a mobile app, an
embedded device, a small daemon).

> **Status: alpha, not published.** Built from this repository.

## Features

- **The Node binding's authoring shape.** A library is an ES module whose
  exported classes extend `Provider`, implement the kinds they list in
  `static kinds`, and throw `BadValue` to refuse. The same file runs under
  the Node binding (`libraryOf`) and here.
- **The kinds come from their descriptions** (`<crate>.kinds.json`, or the
  module `guatiao-kind` writes from it), so nothing is declared twice.
- **Libraries come and go while the host runs.** A refresh opens a new
  manifest, runs an edited module afresh in a context of its own while
  instances of the old build still answer, and retires a deleted one.
- **Limits.** Operations, time, call depth and memory, from the source's
  configuration: runaway code is stopped and the host carries on, even
  code that catches the stop.
- **A pure language.** QuickJS is given no files, network or processes;
  a library imports `guatiao`, `guatiao/kinds`, `guatiao/provide` and its
  own files.

## Building

```bash
cargo build --release --manifest-path crates/guatiao-js/Cargo.toml
```

The output is `libguatiao_js.so`, `libguatiao_js.dylib` or
`guatiao_js.dll`. Building needs a C compiler (QuickJS, and libffi through
`guatiao-libffi` for the kind tables).

## Quick start

A library, `greeters.js`, in a package (`package.json` names it and gives
its version):

```js
import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';
import { Greeter } from './greeter_kind.js';

export class Hello extends Provider {
  static kinds = [Greeter];
  greet(name) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello, ${name}` };
  }
}
```

its manifest beside it, `greeters.guatiao-js.toml`:

```toml
name = "greeters"
kinds = ["greeter"]
```

and a host adds the source with where the manifests and the kinds are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/js"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);        // optional: {"operations": 1000000, "memory": 67108864}
registry.add_source_file(Path::new("/opt/myapp/libguatiao_js.so"), &config)?;
let changed = registry.refresh();
```

[`examples/js_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/js_provider)
holds a greeter that runs under Node and here, and one taking trees.

## What it cannot do here

- **Build instances from a configuration.** A provider is one instance,
  built when the library opens; `static config` is refused.
- **Define a kind in JavaScript.** A kind is a description the host gives.
- **Wait.** There is no event loop and no timer: promise reactions run
  after each call, and a slot answers before it returns.
- **Stop native work mid-call.** A limit is checked once per 10 000
  calls and loop iterations: one long `repeat` or sort runs to its end
  (the memory limit bounds what it can build).
- **Hand the host an object of its own.** A script passes the host a
  handle the host gave it, never a script object.

## API overview

| Item | What it is |
| --- | --- |
| `guatiao_source` | the entry point a host's `add_source_file` calls; declares `source=js` |
| configuration | `{"dirs": [..], "manifests": [..], "kinds": [..], "limits": {..}}` |
| `*.guatiao-js.toml` | one JavaScript library: `name`, `kinds`, optional `version`, `main`, `paths` |
| `guatiao`, `guatiao/kinds`, `guatiao/provide` | in a library: the errors, `kindsOf`, `Provider` |
| `JsEngine`, `JsValues` | the Rust side: the engine, the value conversion |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-js/AGENTS.md)
is the full reference, with what differs from the Node binding.

## Development

```bash
cargo build --manifest-path crates/guatiao-js/Cargo.toml    # the source, which its tests add as a file
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-js/Cargo.toml
```

`GUATIAO_LIBRARY` names a build of the guatiao library with `load`
(`cargo build -p guatiao --features load`); the C host test links it. The
test that runs the example greeter under Node needs the Node binding built
too (`npm ci` in `bindings/node` and in `examples`, `npm run build` in
`bindings/node/guatiao`). The crate is a workspace of its own.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
