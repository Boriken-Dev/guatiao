# Rhai: scripts served to any host

[Rhai](https://rhai.rs) is a small scripting language written in Rust, made
to be embedded in a program and to run code that program did not write.
`guatiao-rhai` makes a Rhai script a **provider** that any guatiao host
loads, in any language: a file the host adds once, whose libraries are
scripts, listed without running them, opened when they appear or change,
and run under limits the host sets.

Rhai has no users outside Rust hosts, so it has no host binding of its own
here, only this source. It is one of the
[language bridges](language-bridges.md), built from the pieces
`guatiao-bridge` shares: it is the engine and the conversion of values.

## Build the source

The crate is a workspace of its own, built from this repository and not
published. Rhai is Rust and is built into the file; libffi, which the kind
tables need, is built from C, so a C compiler is needed.

```bash
cargo build --release --manifest-path crates/guatiao-rhai/Cargo.toml
```

The file is `libguatiao_rhai.so`, `libguatiao_rhai.dylib` or
`guatiao_rhai.dll`.

## Write a library

A library is a script and a manifest beside it. The manifest says what
the source reads before running anything:

```toml
# greeters.guatiao-rhai.toml
module = "greeters"           # the library's id; the script is greeters.rhai
kinds = ["greeter"]           # every kind its providers serve
paths = ["."]                 # where the script is; an edit under it is noticed
```

The script's last expression is a map of providers. A provider is a map:
its function members are the slots of the kinds it serves, and its other
members are its state, which a slot reaches as `this`.

```js
const VERSION = "1.0.0";       // read from the parse, without running the script

let hello = #{
    name: "Hello, in Rhai",
    kinds: ["greeter"],
    asked: 0,
    greet: |name| {
        if name == "" { throw "nobody to greet"; }     // a refusal, with these words
        this.asked += 1;
        #{ greeting: `hello, ${name}` }
    },
};

#{ providers: #{ hello: hello } }                       // served as `greeters_hello`
```

[`examples/rhai_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/rhai_provider)
serves every slot of the example kinds this way: values in and out, a map
the host lends and the script completes, a listener it calls back, a
conversation it hands the host to drive, and a second provider refusing
with a status of its own.

What a script has beside Rhai's own is the `guatiao::` module: `refuse`
(`throw guatiao::refuse("not_found", "no such greeting")`), `number` (a
number whose exact text no float holds), `ABSENT`, and `STATUS`. A
failure that is not a `throw` is a bug in the library and crosses as
`GUATIAO_ERR_INTERNAL`.

## Add it from a host

The host adds the file with where the manifests and the kinds are, and
optionally the limits:

From Rust:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/rhai"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);       // {"operations": 1000000, "time_ms": 250, ..}
registry.add_source_file(Path::new("/opt/myapp/libguatiao_rhai.so"), &config)?;
let changed = registry.refresh();
let hello = registry.offer::<dyn Greeter>("greeters_hello");
```

From Python:

```python
reg = Registry("app", "1.0")
reg.add_source_file(
    "/opt/myapp/libguatiao_rhai.so",
    {"dirs": ["/opt/myapp/rhai"], "kinds": ["/opt/myapp/greeter_kind.kinds.json"],
     "limits": {"operations": 1_000_000, "time_ms": 250}},
)
reg.refresh()
hello = reg.bind("greeters_hello", Greeter)
hello.greet("ana")
```

Any other host does the same through `add_source_file` of its binding, or
`guatiao_registry_add_source_file` in C. A refresh after an edit serves the
new code while an instance taken from the old build still answers.

## Limits

A script is untrusted by default. The configuration's `limits` apply to
each call, opening a library included, and a runaway script is stopped
with a status that names the limit while the host carries on:

| limit | stops | status |
| --- | --- | --- |
| `operations` | a script past that many Rhai operations (at most 16 777 216) | `GUATIAO_ERR_INTERNAL` |
| `time_ms` | a call that runs longer | `GUATIAO_ERR_INTERNAL` |
| `depth` | recursion past that many calls (32 when unset, at most 8192), or any recursion past the stack the call has | `GUATIAO_ERR_INTERNAL` |
| `memory` | a string, array or map larger than the limit allows | `GUATIAO_ERR_ALLOC` |

A stop cannot be caught by the script: a `try`/`catch` around a loop does
not keep it running. A script reaches no file and no network, cannot
import another module, and its `print` and `debug` do nothing. A function
body or closure nests expressions at most 16 deep; a script with a deeper
one does not open.

**Set `operations` for a library that nests values deeply.** Rhai copies
and drops the values a script nests (arrays, maps, closures that capture
closures) by recursion on the native stack, and nothing but the
operations it takes bounds how deep they nest: each level costs at least
one. The engine's thread reserves a stack for the limits when the source
starts, 640 bytes for each operation (at least a hundred thousand) beside
32 KiB for each call level: about 66 MB with no limits, 1.3 GB at
`operations` 2 000 000, 10.7 GB at the cap, and about five times as much
when the source is a debug build. Memory is used only as deep as a script
goes, so a call costs the same under any limits; what the deepest call
touched stays in use until the source closes. On Linux the
reservation counts toward the system's commit limit, and a system that
will not reserve it refuses the source (`GUATIAO_ERR_UNSUPPORTED`, naming
the stack): lower `operations`. Comparing or writing a nested value
recurses through Rhai's own calls; when that nears the end of the stack
the call stops at its depth limit. `to_json` refuses a map nested deeper
than 512 levels or holding more than 1 000 000 values. Without
`operations`, a script that nests values more than a hundred thousand
levels deep can run past the stack and abort the host process. What a
provider builds up in its own state over many calls is not covered
either.

`memory` is the weakest of the four: Rhai counts each container, not the
sum of them. A script can hold many containers each under the limit, so
set `operations` and `time_ms` beside it. With a limit on arrays, each
`push` costs time in proportion to the array's length.

## What it cannot do

- **Keep a map's key order.** A Rhai map keeps its keys sorted, so a map
  crosses to a script and back with its keys in byte order. Everything
  else in the shared wire vectors comes back the same.
- **Build instances from a configuration**, **import a module**, or
  **pass a map to a host as an object** (a script hands on the handles it
  was given).
- **Run in parallel.** One engine serves every call, on a thread of its
  own; a call from another thread is queued.

The crate's
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-rhai/AGENTS.md)
is the full reference.
