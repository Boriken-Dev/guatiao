# `guatiao-source-rhai` — API header

Header-file-style reference for the `guatiao-source-rhai` crate: a guatiao source
serving libraries written in [Rhai](https://rhai.rs), and the Rhai side of
guatiao (values, refusals, handles) it is built on. It ships inside the
package and is self-contained. Development documentation lives with the
source at <https://github.com/Boriken-Dev/guatiao>.

No features. MSRV 1.89. Depends on `guatiao` (`load`), `guatiao-bridge`,
`guatiao-libffi`, `rhai` 1.26 (default features, not `sync`) and `stacker`.
Not published; a workspace of its own. `crate-type = ["cdylib", "rlib"]`:
the cdylib is the source a host adds, the rlib is the engine for a Rust
host.

The root re-exports everything public: `RhaiEngine`, `RhaiValues`,
`Handle`, `Refusal`, `Absent`, `Exact`, and `rhai`. The root is
`deny(unsafe_code)`; `exports.rs` (`guatiao::source!`) alone allows it.

## A library (`*.guatiao-rhai.toml` and its script)

```toml
module = "pkg.greeters"           # required: the script is pkg/greeters.rhai; the library's id
kinds = ["greeter", "counter"]    # required: every kind its providers serve
paths = ["."]                     # where the script is, relative to this file
version = "1.0.0"                 # optional
```

The script is `pkg/greeters.rhai` or `pkg/greeters/init.rhai` on `paths`.
Without `version`, its top-level `const VERSION = "...";` is read from the
script's parse (a literal; nothing is run); none is a refusal asking for
`version`.

```js
const VERSION = "1.0.0";
const GREETING = "hello";

let greeting = |name| {
    if name == "" { throw "nobody to greet"; }       // a refusal: BAD_VALUE, these words
    #{ greeting: `${GREETING}, ${name}` }
};

let hello = #{
    name: "Hello, in Rhai",                           // shown to a person (default: the key)
    kinds: ["greeter", "counter"],
    asked: 0,                                         // state: any other member
    greet: |name| greeting.call(name),                // a slot: `this` is this map
    count: || { this.asked += 1; this.asked },
    start: |listener| #{                              // answers an object: a map of its kind's slots
        listener: listener,
        say: |what| { this.listener.heard(what); },
        turns: || 0,
    },
};

#{ providers: #{ hello: hello } }                     // served as `pkg_greeters_hello`
```

- **Opening** compiles the script, runs it once (under the limits) and
  reads the value it ends with: a map whose `providers` map holds
  provider maps. Opened afresh on every refresh that sees a new stamp
  (the manifest, or the newest `.rhai` file under `paths`).
- **Refused** (`GUATIAO_ERR_BAD_VALUE`, in words naming the library): a
  script that does not parse (with its line), one that ends with no
  `providers` map, a provider that is not a map or has no `kinds`, a kind
  the manifest does not list, a script `VERSION` other than the manifest's
  `version`, a required slot with no function (naming it), a kind with no
  description (`GUATIAO_ERR_NOT_FOUND`), and any error the script raises.
- **A provider** is a map. Its slots are the members that are functions,
  named for the kind's slots: closures (`|x| ..`, which see the script's
  variables and constants above them) or `Fn("name")` pointers to a
  script-level `fn` (which see neither; Rhai's rule). A slot it leaves out
  is null: a Rust caller runs the kind's default. Its id is
  `<module>_<key>` (dots made `_`).
- **A slot is called** with `this` bound to the provider's map, so state
  kept in `this` lasts. Arguments: numbers, booleans and text as Rhai's;
  `bytes` as a blob; a tree as an array or map (a lent one is changed in
  place and read back); `bytes_mut` as a blob the slot writes into
  (`dst.write_utf8(0, n, text)`, `dst[i] = b`); an object as a handle whose
  methods are its kind's slots (a lent one answers `gone` after the call).
- **Its answer** is the value the function returns (a text slot's may be a
  string or a char); `()` for none. An
  object kind's answer is a map of that kind's slots, held until the caller
  destroys it, or a handle handed back.
- **A refusal** is `throw "why"` (`GUATIAO_ERR_BAD_VALUE`) or
  `throw guatiao::refuse(status, why)` (`status` a name or a number). Any
  other error Rhai raises (a missing function, a bad index, a division by
  zero, `throw 42`) is `GUATIAO_ERR_INTERNAL` with its first line. A
  provider called again while it runs (through the host) answers
  `GUATIAO_ERR_BUSY`.

## The `guatiao::` module (`helpers`)

| name | is |
| --- | --- |
| `refuse(why)`, `refuse(status, why)` | a refusal to `throw`: `.status` (name), `.code`, `.message`, `to_string` |
| `number(text)` | an exact number from text: Rhai's integer or float when one holds it, else a `guatiao::number` |
| `ABSENT` | absent, inside a tree (`()` is null) |
| `STATUS` | every status by name (`STATUS.busy`): `ok`, `bad_value`, `alloc`, `wrong_kind`, `not_found`, `null`, `gone`, `unsupported`, `busy`, `internal` |
| `VERSION` | this crate's version |

A `guatiao::number` prints as its text, compares and computes through its
float (`+ - * /`, `< <= > >=`, `==` between two as text), and has
`.to_float()`.

### A handle (`Handle`)

`h.slot(args..)` calls the slot of the host's object. Arguments as above,
except those the host lends *to the callee* (a lent tree, a buffer): a
script cannot pass them, and says so. A handle can be handed on to a slot
taking an object of its kind (handed over: it is gone from the handle; or
lent). A failing slot raises a `Refusal` (`try { .. } catch (e) { e.status
}`). A copy of a handle is the same object. An object handed to the script is
used by one call at a time: a call into it from inside one of its own
(through the host) raises a `busy` refusal.

## Values (`RhaiValues`)

```rust
impl Convert for RhaiValues { type Native<'a> = rhai::Dynamic; type Error = String; }
```

| guatiao | Rhai | back |
| --- | --- | --- |
| absent | `guatiao::ABSENT` | absent |
| null | `()` | null |
| bool | bool | |
| number | an integer when its text is one (`i64`); a float when the float's shortest text is the text; else `guatiao::number` (its exact text) | an integer or float as its text; NaN and the infinities refused |
| string | string | a char goes as text |
| bytes | blob | a blob |
| list | array | an array |
| map | map | a map; its keys come back in byte order |

Refused: a function, a handle, any other custom value (naming the type),
trees nested deeper than 512. **A guatiao map's own key order does not
survive**: a Rhai map keeps keys in byte order. Every shared wire vector
comes back the same except the three whose maps hold keys out of order
(`the documented map`, `a map keeps its order`, `nested`), which come back
sorted; a test checks that is the only difference.

## The engine (`RhaiEngine`)

`impl guatiao_bridge::Engine`: `ThreadModel::Pinned` (one `rhai::Engine`,
not `Send`, on the bridge's thread), manifests `*.guatiao-rhai.toml` with
`id_key = "module"` and code extension `rhai`, `Opened::Described`. Every
provider of a library shares the library's one compiled script.

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<RhaiEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "guatiao-source-rhai")?.with_mechanism(Arc::new(Libffi)))
}
guatiao::source!(serves = "rhai", make = start);
```

## Limits

From the configuration's `limits` (the bridge's `Limits`), per call,
opening a library included:

| limit | kept by |
| --- | --- |
| `operations` | Rhai's operation count (`set_max_operations`), at most 16 777 216; a `try`/`catch` cannot catch the stop |
| `time_ms` | a progress callback reading the clock every 64 operations; not catchable either; a call inside another has the shorter of the two |
| `depth` | Rhai's call levels (`set_max_call_levels`), at most 8192. Unset: 32. Also any recursion that reaches the end of the stack the call was given (below) |
| `memory` | in bytes, turned into Rhai's size limits: a string may be that long, an array holds `memory / 16` items (a blob counts a byte as one), a map `memory / 64` |

A stop is `GUATIAO_ERR_INTERNAL` ("stopped at its operations limit
(200000)", "stopped at its depth limit (32)"), or `GUATIAO_ERR_ALLOC` for
memory; the engine answers the next call. Arguments the host passes are
checked against the size limits too.

**The native stack.** Rhai recurses on the native stack, for calls and for
copying and dropping the values a script nests, and a script that
overflows it aborts the host process. So function bodies and closures nest
expressions at most 16 deep (`set_max_expr_depths(64, 16)`; a deeper
script does not open: "Expression exceeds maximum complexity"), which
bounds a level of calls at 14 KiB in a release build and 85 KiB in a debug
one. The engine thread's stack (`Engine::stack_size`) is sized from the
limits once, when the source starts: 32 KiB (192 KiB in debug) for each
call level, and below them 640 bytes (3 KiB in debug) for each
operation, at least a hundred thousand of them: the nesting a script can
build, since every level costs at least one operation, and copying,
dropping or formatting a nested value recurses within one operation (at
most 496 bytes a level measured in a release build). That is about 66 MB
with nothing set, 1.3 GB at `operations` 2 000 000 and 10.7 GB at its cap
(about five times as much in a debug build). It is reserved, not used:
memory goes to the stack only as deep as a script reaches, and a call
costs the same whatever the limits (a few microseconds through the whole
path, release). A stack does not shrink: what the deepest call touched
stays the thread's until the source closes. On Linux the reservation
counts toward the commit limit, and a system that will not reserve it
refuses the source (`GUATIAO_ERR_UNSUPPORTED`, "the engine thread did not
start with a stack of N bytes"): lower `operations`. A 32-bit process
holds the reservation for `operations` up to about a million (about
200 000 in a debug build). Where the
system does not report a thread's stack (Android, iOS), what is left is
measured from where the engine started.
Everything a call makes is read and dropped within its call: its answer,
a value it throws, the lent arguments it changed, and what a script's last
expression holds beyond its providers.

Rhai also recurses through its own function calls over a nested value:
comparing one (`==`, `contains`), writing it (`to_string`, `to_debug`,
string interpolation), about 2 KiB a level in a release build. Every
operation checks the stack left (`stacker::remaining_stack`), and below
what the nesting needs plus 256 KiB (1 MiB in debug) the call stops at its
depth limit, as if it had recursed that deep. `to_json` writes a map
nested at most 512 levels and holding at most 1 000 000 values (a value
two closures share counted in each); a larger one is
`GUATIAO_ERR_BAD_VALUE`.

## Errors

| Rhai | status |
| --- | --- |
| `throw "why"` | `GUATIAO_ERR_BAD_VALUE` |
| `throw guatiao::refuse(s, w)` | `s` |
| `throw` of anything else, any other error Rhai raises | `GUATIAO_ERR_INTERNAL`, its first line |
| a limit | as above |

## The C surface

```c
guatiao_status guatiao_source(const guatiao_host_info *host,
                              const guatiao_value *config,      /* a map, or null */
                              const guatiao_source_info **out,
                              guatiao_provider_error *err);
```

As `guatiao.h` declares any source's entry. The file declares
`source=rhai`, so a scan reports it as a source and never maps it as a
library. A configuration that is not a map is `GUATIAO_ERR_WRONG_KIND`; a
key that is not a list of text, a misspelled limit, a `depth` above 8192
or `operations` above 16 777 216, `GUATIAO_ERR_BAD_VALUE`.

## Gotchas

- **Nesting is bounded by `operations`, per call.** Without `operations`
  the stack holds values nested a hundred thousand levels; a script that
  nests deeper (arrays, maps, closures that capture the last one: no size
  limit bounds how deep they nest) can run past the native stack and abort
  the host. With it, whatever one call nests fits. **Not covered**: depth
  built up over many calls in a provider's state (`this`), which no limit
  sees. Set `operations` when a library nests values deeply.
- **`memory` does not bound the whole**: it bounds each string, array and
  map, nested ones counted. A script can hold many strings each under the
  limit, and a map filled with `m[key] = value` is not counted at all (the
  operations limit bounds it). Set `operations` and `time_ms` beside it.
- **An array limit costs time**: with one set (any `memory`), every
  `push` re-counts the array, so building one of `n` items takes time
  proportional to `n` squared (80 000 pushes took 6 s against 50 ms
  without a limit, release build).
- **`print` and `debug` do nothing**: the host's standard output may be
  a protocol, and a script has no business writing to it.
- **No `import`**: a library is one script; the engine has no module
  resolver.
- **`try`/`catch` is a statement**: its value is not the closure's. Write
  `return` in the branches that answer.
- **A closure shares the variables it captures** with the script and with
  every other closure that captures them, across providers: state that
  belongs to one provider lives in `this`, not in a `let` above it.
- **Rhai's reserved words** (`use`, `import`, `new`, `static`, ..) cannot
  be unquoted member names or called as methods: a slot named so is
  written `"use": |..| ..` in the map and cannot be called by a script.
- **A limit is checked between operations**: one long call into a Rhai
  function (a big `sort`, a `pad`) runs to its end; the size limits bound
  what it can build.
- **A provider, or an object, is re-entered with `BUSY`**, not queued:
  the host calling back into the provider that is calling it, or into a
  handle the script is calling, is refused.
- **`cargo test` does not build the file**: `cargo build` first, or the
  tests adding it by file state a skip.
