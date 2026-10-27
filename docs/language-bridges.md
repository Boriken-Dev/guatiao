# Writing a bridge to another language

A **language bridge** makes libraries written in another language
providers that any guatiao host loads, in any language. It is a
[source](python.md#serving-python-to-any-host): a file the host adds once,
whose libraries are that language's modules, listed without running them,
opened when they appear or change, and closed once nothing holds them.
`guatiao-source-py` is one for Python, [`guatiao-source-lua`](lua.md) one for Lua,
`guatiao-source-js` is QuickJS as one, serving libraries written as the Node
binding's providers ([JavaScript without Node](js.md)), and
`guatiao-source-wasm` one for WebAssembly modules compiled from any language
([providers compiled to WebAssembly](wasm.md#providers-compiled-to-webassembly)).
`guatiao-bridge` is what every such bridge shares, so a bridge for a new
language is its runtime and its value conversion, and nothing else.
The Rhai source ([rhai.md](rhai.md)) is a worked example of a pinned
engine with limits.

## The pieces

| piece | what it does |
| --- | --- |
| `Engine` | the runtime: start from the host's configuration, open the library a manifest describes, answer calls into it, close it |
| `ThreadModel` | where the runtime may be called from |
| `manifest` | libraries described without running them, and the stamps that tell a refresh they changed |
| `BridgeSource` | the engine as guatiao's `Source`: what the bridge's file serves |
| `Kinds` | kinds known from their descriptions (`<crate>.kinds.json`) |
| `Limits`, `BridgeError` | what a library's code may use, and the status every refusal crosses as |
| `Convert`, `conformance` | the runtime's values both ways, checked against the shared vectors |

## An engine

```rust
use guatiao::library::{Host, ProviderError};
use guatiao::Map;
use guatiao_bridge::{BridgeError, BridgeSource, Call, CallArg, Config, Engine, Manifest,
                     ManifestFormat, Opened, Provided, Reply, ThreadModel};

struct Toy { /* the runtime */ }

impl Engine for Toy {
    type Library = u64;                         // the engine's key for an open library
    const THREADS: ThreadModel = ThreadModel::Pinned;
    const MANIFESTS: ManifestFormat = ManifestFormat {
        serves: "toy",                          // type = "toy" in greeters.guatiao.toml
        id_key: "module",                       // module = "greeters": the library's id
        code_extensions: &["toy"],              // what a stamp follows
        skip_dirs: &[],
    };

    fn start(config: &Config) -> Result<Toy, BridgeError> { /* .. */ }

    fn open(&self, manifest: &Manifest, version: &str) -> Result<Opened<u64>, BridgeError> {
        // Run the module, then say what it serves:
        Ok(Opened::Described {
            library: 1,
            providers: vec![Provided::new("greeters_hello").serves("greeter", ["greet", "check"])],
        })
    }

    fn call(&self, library: &u64, call: &mut Call) -> Result<Reply, ProviderError> {
        match call.method() {
            "greet" => { /* call.args[0] is the name, as a value */ }
            _ => {}
        }
        /* .. */
    }

    fn close(&self, library: u64) { /* drop the module */ }
}
```

The bridge's file exports it with core's macro, which writes the
`guatiao_source` entry any host adds:

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<Toy>, ProviderError> {
    Ok(BridgeSource::pinned(config, "toy")?)
}
guatiao::source!(serves = "toy", make = start);
```

`guatiao::source!` serves any `Source`, not only a bridge's:
[`examples/source_library`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/source_library)
is a source written in Rust that a Rust host and a C host add.

### Where it is called from

| `ThreadModel` | the runtime | started with |
| --- | --- | --- |
| `Any` | is thread-safe: called from any thread, at once | `BridgeSource::shared` |
| `Locked` | serialises itself (CPython's GIL): called from any thread, a call waits on its lock | `BridgeSource::shared` |
| `Pinned` | lives on one thread (Lua, QuickJS, Rhai): the bridge starts it there; a call from another thread is queued and waited for, and a call it makes into itself through the host runs there and then | `BridgeSource::pinned` |

A pinned runtime need not be `Send`: it is created on its thread and
never leaves it. A runtime that recurses on the native stack asks for
that thread's stack with `Engine::stack_size(config)`, from the limits
it was given: the size is reserved, and memory is used only as deep as
the runtime goes.

## Manifests and stamps

A library is a manifest, a small TOML file named `<name>.guatiao.toml`
beside its code ([the format](encodings.md#a-library-manifest)):

```toml
type = "toy"                     # the format's `serves`: the source that reads it
module = "greeters"              # the format's id key: the library's id
version = "1.0.0"                # optional: else Engine::version_of reads it from the code
kinds = ["greeter"]              # every kind its providers serve
paths = ["."]                    # where its code is, relative to this file
```

The host's configuration names where they are:

| key | holds |
| --- | --- |
| `dirs` | directories whose manifests, and those of their immediate subdirectories, are libraries |
| `manifests` | manifest files |
| `kinds` | kind description documents |
| `limits` | `{"operations": n, "memory": bytes, "time_ms": n, "depth": n}` |
| anything else | the engine's own (`guatiao-source-py`'s `paths`) |

A source lists the manifests of its own type and passes over those of
guatiao's sources (`manifest::TYPES`: `python`, `js`, `lua`, `wasm`,
`rhai`); one with no `type`, or a type that is neither, is listed and
refused when opened. A bridge of your own picks a type no other source
uses, as `guatiao::source!` names it; guatiao's sources report its
manifests as naming none of theirs, so keep its libraries in directories
of their own.

A refresh lists the manifests, running nothing; the stamp of each is the
manifest's content and the newest modification of a code file under its
paths. A new stamp opens the new build first and retires the old one,
which closes once nothing holds it: an edit is served at the next
refresh while an instance taken from the old build still answers. A
manifest that does not parse, or a version that cannot be read, is
listed and refused when opened, in words naming the file.

## Kind tables

A runtime with an FFI of its own builds its tables itself, as CPython
does through ctypes, and answers `Opened::Built` with the library's
descriptor.

A runtime with none answers `Opened::Described`: each provider, the kinds
it serves and the slots it implements. The bridge builds each kind's
table from its description, every slot a closure that becomes a `Call`
into the engine. Rust cannot make a function pointer for a signature it
learns at run time, so the closures come from a mechanism the source is
given, `guatiao-libffi`'s `Libffi` over libffi:

```rust,ignore
BridgeSource::pinned(config, "lua")?.with_mechanism(Arc::new(Libffi))
```

Without one, a described library is refused when opened. What the
closures do:

- **arguments arrive owned**, so a call crosses to an engine thread:
  numbers, text and bytes as values, a borrowed tree copied, a handed-over
  one moved. A lent tree or buffer the engine changes is written back over
  the caller's after the call; a lent object is callable until the call
  returns.
- **the reply** is a value, converted to the slot's answer (a number
  narrowed to its width, or refused when it does not fit), or an object
  the engine holds, served through a table built when the library opened;
  its destruction reaches `Engine::release`.
- **a slot left out** is null in the table, so a Rust caller runs the
  kind's default body; a required one left out refuses the library,
  naming it.

A provider can also be built from a configuration and asked whether it
can run, as one written in Rust or C can:

```rust,ignore
Provided::new("greeters_custom")
    .serves("greeter", ["greet"])
    .config(schema)          // a JSON Schema, as a value
    .instances_only()        // offered only through what it builds
    .may_be_unavailable()    // a host asks the engine before using it
```

- **an instance**: a host's `Offer::instantiate(&config)` checks the
  configuration against the schema, then calls `Engine::create(library,
  provider, config)`; the id the engine answers is what the instance's
  slots name (`Target::Instance(id)`), and `Engine::destroy(library, id)`
  hears once when its last owner drops it.
- **availability**: every time a host asks, `Engine::available(library,
  provider)` answers `Ok` or the words a person reads (`no device
  attached`). Every distinct reason is kept until the library closes, so
  answer with words that do not change from one call to the next.

The tables come from the kinds' descriptions, the same documents the
bindings read. Core offers them to any Rust code, bridge or not, with no
feature: `ServedTable` builds a table of a described kind over one
handler, and `Caller` calls any table of one, so an engine can call an
object it was handed or a provider a registry offers (`DescribedRemote`).
Both take the mechanism (`&Libffi`); another one, written against
`guatiao::library::kind::Mechanism`, plugs in the same way.

A script calling such an object goes through guatiao-bridge as well:
`Held` is what the script's handle calls, and `PreparedArg` holds the
call's arguments while it runs (`with_args`), lent trees and buffers kept
to write back after it. The engine reads only its own values: a number
into a `Reading` for `PreparedArg::scalar`, a tree into a `Value` for
`PreparedArg::tree`, text and bytes as they are, so a whole number, a
map or an object of the wrong kind is refused in the same words by every
engine.

## Limits and refusals

`Limits` comes from the configuration; the engine enforces what it can
(an instruction count, fuel, a memory limit, an interrupt) and
`Limits::budget()` counts for one that keeps its own. A refusal crosses
as:

| | status |
| --- | --- |
| a configuration, manifest or library that does not load | `GUATIAO_ERR_BAD_VALUE` |
| an id no longer listed, a kind with no description | `GUATIAO_ERR_NOT_FOUND` |
| a runtime that cannot start here | `GUATIAO_ERR_UNSUPPORTED` |
| a memory limit | `GUATIAO_ERR_ALLOC` |
| any other limit, a panic, an engine gone | `GUATIAO_ERR_INTERNAL` |

with words a person reads: `stopped at its operations limit (10000)`.

## Values

`Convert` is the runtime's values both ways. `conformance::round_trip`
runs the shared wire vectors through it and back, the ones every
language's implementation is checked against, and says for each whether
it came back the same, changed or was refused: a runtime whose numbers
are doubles says which vectors it cannot keep, and why.
