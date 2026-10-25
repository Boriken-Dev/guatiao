# guatiao-source-rhai

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

A [guatiao](https://crates.io/crates/guatiao) **source of libraries
written in [Rhai](https://rhai.rs)**: any guatiao host, in Rust, C, Python
or any language with a binding, adds this one file and calls providers
written in Rhai as it calls the ones written in Rust or C. Rhai is pure
Rust and built into the file; the host needs nothing else. Rhai has no
host of its own beyond Rust, so this is the way a script reaches the other
languages, and a script runs under limits the host sets: it is meant for
code the host did not write.

> **Status: alpha, not published.** Built from this repository.

## Features

- **A library is a Rhai script.** Its last expression is a map of
  providers; each provider is a map whose function members are the
  kinds' slots, with its state in the same map (`this`). The kinds come
  from their descriptions (`<crate>.kinds.json`), so nothing is declared
  twice.
- **Libraries come and go while the host runs.** A refresh opens a new
  manifest, runs an edited script afresh while instances of the old build
  still answer, and retires a deleted one. Nothing runs on its own, and a
  script's version is read from its parse, without running it.
- **Limits.** Operations, time, call depth and the size of strings, arrays
  and maps, from the source's configuration: runaway code is stopped and
  the host carries on, even code that wraps the stopped loop in
  `try`/`catch`.
- **No way out of the sandbox.** A script reaches no file, no network and
  no other module: `import` is refused, and `print` and `debug` do
  nothing. It can still abort the host if it nests values more than a
  hundred thousand levels deep with no `operations` limit set, or over
  many calls (see below).
- **One engine on a thread of its own**, which every call reaches.

## Building

```bash
cargo build --release --manifest-path crates/guatiao-source-rhai/Cargo.toml
```

The output is `libguatiao_source_rhai.so`, `libguatiao_source_rhai.dylib` or
`guatiao_source_rhai.dll`. Building needs a C compiler (libffi, through
`guatiao-libffi`, for the kind tables; Rhai itself is Rust).

## Quick start

A library, `greeters.rhai`:

```js
const VERSION = "1.0.0";

let hello = #{
    kinds: ["greeter"],
    greet: |name| {
        if name == "" { throw "nobody to greet"; }
        #{ greeting: `hello, ${name}` }
    },
};

#{ providers: #{ hello: hello } }      // served as `greeters_hello`
```

its manifest beside it, `greeters.guatiao.toml`:

```toml
type = "rhai"
module = "greeters"
kinds = ["greeter"]
paths = ["."]
```

and a host adds the source with where the manifests and the kinds are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/rhai"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);        // optional: {"operations": 1000000, "time_ms": 250}
registry.add_source_file(Path::new("/opt/myapp/libguatiao_source_rhai.so"), &config)?;
let changed = registry.refresh();
```

[`examples/rhai_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/rhai_provider)
holds a greeter serving every slot of the example kinds.

## What Rhai cannot do here

- **Nest values without end when `operations` is unset.** Rhai copies
  and drops nested values by recursion on the native stack, and no size
  limit bounds how deep they nest. The engine thread's stack is reserved
  from the limits (about 66 MB with none, 640 bytes an operation with
  `operations`, at most 16 777 216 of them, five times as much in a debug
  build; memory is used only as deep as a script goes, and kept until the
  source closes), so whatever one call nests fits when `operations` is
  set, and a hundred thousand levels fit when it is not; deeper can abort
  the host. Depth built up in a provider's state over many calls is not
  covered. Function bodies nest expressions at most 16 deep.
- **Bound its memory as a whole.** `memory` becomes Rhai's own size
  limits: one string, array or map may not outgrow them, and nested ones
  are counted. A script can still hold many containers each under the
  limit, and a map filled entry by entry is not counted at all; pair
  `memory` with `operations` and `time_ms`. With an array limit set, every
  growth of an array costs time in proportion to its length.
- **Import another file.** A library is one script.
- **Build instances from a configuration.** A provider is one map; a
  kind's `create` is not served.
- **Pass a map to a host as an object.** A script hands on the handles it
  was given; a map it builds is an object only as a slot's answer.
- **Lend to a host object.** A script cannot pass a lent map, list or
  buffer to a slot of a handle: a lend arrives in the script, not from it.
- **Keep a map's key order.** A Rhai map keeps its keys in byte order.
- **Be called from another thread while it waits on one.** Calls are
  queued to the engine's thread; a library that calls the host, which
  calls back into Rhai from another thread, waits on itself.

## API overview

| Item | What it is |
| --- | --- |
| `guatiao_source` | the entry point a host's `add_source_file` calls; declares `source=rhai` |
| configuration | `{"dirs": [..], "manifests": [..], "kinds": [..], "limits": {..}}` |
| `*.guatiao.toml` | one Rhai library: `type = "rhai"`, `module`, `kinds`, `paths`, optional `version` |
| `guatiao::` | in a script: `refuse`, `number`, `ABSENT`, `STATUS`, `VERSION` |
| `RhaiEngine`, `RhaiValues` | the Rust side: the engine, and the value conversion |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-source-rhai/AGENTS.md)
is the full reference.

## Development

```bash
cargo build --workspace --all-features                       # the Rust greeter, the guatiao library
cargo build --manifest-path crates/guatiao-source-rhai/Cargo.toml   # the source, which its tests add as a file
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-source-rhai/Cargo.toml
```

The crate is a workspace of its own, with its own `target/`.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
