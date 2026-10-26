# guatiao-bridge — shipped API header

What every guatiao bridge to a language runtime shares: the engine trait
and its thread, manifests and stamps, the glue to guatiao's `Source`,
kinds served from their descriptions, limits and error mapping, and value
conversion checked against shared vectors. `#![forbid(unsafe_code)]`.
Depends on `guatiao` and `guatiao-serde` (`json`, `toml`). No features.

Providers an engine declares are served through tables built from their
kinds' descriptions by a `guatiao::library::kind::Mechanism` the source is
given (`with_mechanism`); `guatiao-libffi`'s `Libffi` is one, and needs a
C build. A runtime that builds its own tables (CPython does, through
ctypes) needs none.

## Writing a bridge

A language crate implements `Engine` and `Convert`, and exports the
source with core's macro:

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<MyEngine>, ProviderError> {
    Ok(BridgeSource::pinned(config, "my-engine")?)   // or ::shared for Any/Locked
}
guatiao::source!(serves = "mylang", make = start);
```

`guatiao::source!` writes `unsafe extern "C"` code, so the crate calling
it cannot `forbid(unsafe_code)` at its root.

## `Engine`

```rust
pub trait Engine: 'static {
    type Library: Clone + Send + Sync + 'static;     // the engine's key for an opened library
    const THREADS: ThreadModel;                       // Any | Locked | Pinned
    const MANIFESTS: ManifestFormat;
    fn start(config: &Config) -> Result<Self, BridgeError>;            // on the thread it is called from
    fn stack_size(config: &Config) -> Option<usize>;                   // Pinned thread's stack; default: None
    fn version_of(&self, m: &Manifest) -> Result<String, BridgeError>; // default: refused
    fn open(&self, m: &Manifest, version: &str) -> Result<Opened<Self::Library>, BridgeError>;
    fn call(&self, lib: &Self::Library, call: &mut Call) -> Result<Reply, ProviderError>; // default: UNSUPPORTED
    fn release(&self, lib: &Self::Library, object: u64);                // default: nothing
    fn close(&self, lib: Self::Library);
}
pub enum Opened<L> {
    Built { library: L, descriptor: LocalLibrary<'static> },   // the engine built its own tables
    Described { library: L, providers: Vec<Provided> },        // the bridge builds them (with_mechanism)
}
Provided::new(id).name(n).serves(kind, ["slot", ..]).meta(map)
```

- **`ThreadModel`**: `Any` (thread-safe, called at once), `Locked` (the
  engine serialises itself, as CPython's GIL), both through
  `BridgeSource::shared` (needs `E: Send + Sync`); `Pinned` through
  `BridgeSource::pinned(config, thread_name)`, which starts `E::start` on a
  thread it owns for the life of the source: calls from other threads
  are queued and waited for; a call made **on** that thread (the engine
  calling one of its own providers through the host) runs there and then.
  A `Pinned` engine need not be `Send`. Its thread's stack is
  `E::stack_size(&config)` bytes (`None`, the default: the platform's):
  reserved address space, used only as deep as the engine goes (on Linux
  the reservation counts toward the commit limit, and one larger than
  memory and swap is refused). A size the system will not reserve refuses
  the source, `Unavailable` (`GUATIAO_ERR_UNSUPPORTED`).
- **Every method takes `&self`**, and is re-entered when the engine calls
  itself through the host.
- **A panic** in any call is caught; it answers `EngineGone`
  (`GUATIAO_ERR_INTERNAL`) and the engine carries on.
- `Built` (CPython): the engine keeps what the descriptor points into
  until `close`. `Described`: a provider's ctx is its position; a slot left
  out of `serves` is null in the table, so the caller runs the kind's
  default; a required one left out refuses the library, naming the slot;
  a kind with no description given refuses it too.

## A call (`Described`)

```rust
pub struct Call { pub args: Vec<CallArg>, .. }      // target(), kind(), slot(), slot_description(), method()
pub enum Target { Provider(String), Object(u64) }
pub enum CallArg { Value(Value), Lent(Value), Buffer(Vec<u8>),
                   Object(RawObject), LentObject(LentHandle) }      // guatiao::library::kind's
pub enum Reply { Nothing, Value(Value), Object { kind: String, id: u64 }, Handed(RawObject) }
```

- **Arguments are owned**, so a call crosses to an engine thread:
  scalars, text and bytes as values, a borrowed tree **copied**, a taken
  one moved. A `Lent` tree and a `Buffer` are written back over the
  caller's after the call (a lent map left as something else is
  `GUATIAO_ERR_WRONG_KIND`). A `LentHandle` is callable until the call
  returns, then answers `GUATIAO_ERR_GONE`.
- **A float that is not finite** has no value to cross as:
  `GUATIAO_ERR_BAD_VALUE`.
- **A `Reply::Value`** is narrowed to the slot's answer by core
  (`FromValue`, exact): a number that does not fit is refused.
- **`Reply::Object { kind, id }`**: an object the engine holds. Its table
  is built when the library opens, for every object kind a provider's
  slots answer (transitively); its slots arrive as `Target::Object(id)`
  and its destruction as `Engine::release`.
- **An error in a slot that cannot fail** answers its status alone; a
  Rust caller treats that as a broken contract (it panics).

## `BridgeSource<E>`

`impl Source`. `shared(config)`, `pinned(config, thread)`,
`with_mechanism(Arc<dyn Mechanism>)` (serve `Opened::Described`; without
one such a library is refused, `Unavailable`), `config()`, `kinds()`,
`threads()`, `run(|e| ..)` (reach the engine where its thread model says;
`T: Send + 'static`).

- **list**: every manifest of the engine's type in `dirs` (and their
  immediate subdirectories) and `manifests`, each once, in path order. Runs no code
  except `version_of` for a manifest without `version`. A manifest that
  does not parse is listed under its path; one whose version cannot be
  read, under its id with no kinds: opening either says why.
- **open**: `GUATIAO_ERR_NOT_FOUND` for an id no longer listed;
  `GUATIAO_ERR_BAD_VALUE` with the engine's or the manifest's words.
- **close**: by the stamp the id was listed under; `Engine::close`.

## Configuration (`Config::from_map`)

| key | holds |
| --- | --- |
| `dirs`, `manifests` | lists of paths |
| `kinds` | kind description documents to load (`Kinds::load`) |
| `limits` | `{"operations", "memory", "time_ms", "depth"}`, whole numbers; any other key is refused |
| anything else | the engine's (`config.map()`) |

A key holding anything but a list of text is `` `dirs` is not a list of
paths `` (`GUATIAO_ERR_BAD_VALUE`).

## Manifests (`manifest`)

```rust
pub struct ManifestFormat { pub serves, pub id_key, pub code_extensions, pub skip_dirs }
manifest::SUFFIX = ".guatiao.toml";  manifest::TYPES = ["python", "js", "lua", "wasm", "rhai", "remote"]
manifest::find(&format, dirs, files) -> Vec<Found>;  manifest::read(&format, path) -> Option<Found>
Manifest: path() id() version() kinds() paths() stamp() table()
Found::{Library(Manifest), Broken { path, stamp, why }}
```

A manifest is a file named `*.guatiao.toml`. `type` (required, text)
names the source that serves it, `serves`: a manifest of another type
in `TYPES` is passed over without a word (`read` answers `None`), and
one with no `type`, or a type neither in `TYPES` nor the format's own,
is `Found::Broken`. `id_key` (required, text), `version` (optional), `kinds`, `paths`
(default `["."]`, made absolute, not canonical). Stamp: FNV-1a of the
manifest's bytes, `:`, the newest modification time of a code file under
its paths (at most 10 000 files, 8 levels, `skip_dirs` skipped); a
format with no `code_extensions` walks nothing, its stamp the manifest's
bytes alone. `remote` is guatiao-source-remote's: a peer, not a library.

## Errors and limits

`BridgeError::{Config, Manifest, NotListed, Unavailable, Engine, Limit,
UnknownKind, Table, EngineGone}`; `status()` and `From<BridgeError> for
ProviderError`:

| error | status |
| --- | --- |
| `Config`, `Manifest`, `Engine`, `Table` | `BAD_VALUE` |
| `NotListed`, `UnknownKind` | `NOT_FOUND` |
| `Unavailable` | `UNSUPPORTED` |
| `Limit { Memory }` | `ALLOC` |
| `Limit { .. }`, `EngineGone` | `INTERNAL` |

A limit's words: `stopped at its operations limit (10000)`.
`Limits::budget()` counts operations and time for an engine that keeps its
own count: `budget.spend(n)?`.

## `Kinds`

`add(KindDescription)`, `add_value(&Value)` and `add_json(&str)` (a
document or one kind), `load(path)`, `get(name)`, `names()`. The same
description twice is a no-op; two different ones of one name are refused.

## `Convert` and `conformance`

```rust
pub trait Convert {
    type Native<'a> where Self: 'a;
    type Error: Display;
    fn to_native<'a>(&'a self, v: &Value) -> Result<Self::Native<'a>, Self::Error>;
    fn to_value<'a>(&'a self, n: Self::Native<'a>) -> Result<Value, Self::Error>;
}
conformance::values() -> Vec<(String, Value)>                  // the shared wire vectors
conformance::round_trip(&c) -> Vec<(String, Outcome)>         // Same | Changed(Value) | Refused(String)
```

`vectors/wire.json` is a copy of the core's `tests/vectors/wire.json`,
compared byte for byte by a test.

## Gotchas

- **A pinned engine waits on itself if it waits on another thread** that
  calls back into it: a call from that thread is queued behind the job
  that is waiting. Calls the engine makes on its own thread run inline.
- **The engine thread lives while anything can call it**: the source and
  every table a library it opened still has (a handle's lease keeps
  them). Dropping the registry closes nothing, as in core.
- **Arguments are copied to cross to the engine**: a borrowed tree is
  cloned once per call, even for a `Shared` engine.
- **A required slot an engine leaves out refuses the whole library** on
  open, in words naming the provider, the slot and the kind.
- **`guatiao-libffi` builds libffi from C**: see its header; the bridge
  itself builds no C.
