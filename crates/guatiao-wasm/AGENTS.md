# `guatiao-wasm` — API header

Header-file-style reference for the `guatiao-wasm` crate: a guatiao source
serving libraries compiled to WebAssembly from any language, run sandboxed
by wasmtime, and the interface such a library speaks (guatiao ABI 1). It
ships inside the package and is self-contained. Development documentation
lives with the source at <https://github.com/Boriken-Dev/guatiao>.

No features. MSRV 1.89. Depends on `guatiao` (`load`), `guatiao-bridge`,
`guatiao-libffi`, `wasmtime` 36 (`cranelift`, `runtime`, `std`: no
component model, no WASI crate) and `getrandom`. Not published; a
workspace of its own. `crate-type = ["cdylib", "rlib"]`.

The root re-exports `WasmEngine`, `WasmLibrary`, `WireBytes` and the
module `abi`. The root is `deny(unsafe_code)`; `exports.rs`
(`guatiao::source!`) alone allows it.

Guests: Rust with `guest/` (`guatiao-wasm-guest`, its own `AGENTS.md`), Go
with `guest-go/` (package `guest`, its own `AGENTS.md`), any other language
by the ABI below.

## A library (`*.guatiao-wasm.toml` and its module)

```toml
module = "greeters"               # required: the library's id; the module is greeters.wasm
kinds = ["greeter", "counter"]    # required: every kind its providers serve
paths = ["."]                     # where greeters.wasm is, relative to this file (the first that has it)
version = "1.0.0"                 # optional when the module has a `guatiao.version` section
```

- **The version**: the manifest's, else the module's `guatiao.version`
  custom section (UTF-8), read from its bytes without compiling it; neither
  is a refusal asking for `version`, and both differing is one too.
- **Opening** compiles the module (Cranelift), checks its exports and
  imports before any of its code runs, instantiates it, calls
  `_initialize` when exported, checks `guatiao_guest_abi() == 1`, and reads
  its declaration; every step under the limits. A refresh that sees a new
  stamp (the manifest's bytes, the newest `.wasm` under `paths`) opens it
  afresh while the old build answers the instances taken from it.
- **Refused** (`GUATIAO_ERR_BAD_VALUE`, in words naming the module): not
  a core module (a component, not wasm), an export missing or of another
  signature, a 64-bit or shared memory, an import this source does not
  offer, another ABI version, a declaration that is not one or names more
  than 4096 providers, a provider serving a kind the manifest does not
  list, a required slot left out, a trap or a limit while opening; a kind
  with no description is `GUATIAO_ERR_NOT_FOUND`.
- **A provider** is served as `<module>_<id>` (dots made `_`). A slot it
  does not name is null in the table: a Rust caller runs the kind's
  default. A slot the host's description lacks is ignored.

## guatiao ABI 1 (`abi`)

A core module; every address and length an `i32` read as unsigned; every
value its wire encoding (`guatiao::value::wire`) in the module's memory.

| export | signature | |
| --- | --- | --- |
| `memory` | memory | 32-bit, not shared |
| `guatiao_guest_abi` | `() -> i32` | `1` |
| `guatiao_guest_alloc` | `(len: i32) -> i32` | bytes for a request; the guest's once written |
| `guatiao_guest_declare` | `() -> i32` | the declaration through `answer`; a status |
| `guatiao_guest_call` | `(object: i32, target: i64, request: i32, len: i32) -> i32` | a status |
| `guatiao_guest_release` | `(object: i64)` | optional: an object it answered was destroyed |
| `_initialize` | `()` | optional, called first |

| import (`guatiao`) | signature | |
| --- | --- | --- |
| `answer` | `(ptr: i32, len: i32)` | the call's answer (status 0: a wire value) or words (else); copied |
| `write_back` | `(index: i32, ptr: i32, len: i32) -> i32` | a wire value over argument `index`: a lent tree, or bytes for a buffer; `GUATIAO_ERR_WRONG_KIND` for another, `NOT_FOUND` past the last |
| `object_call` | `(handle: i64, request: i32, len: i32) -> i32` | calls a host object: request `[method, arg...]`; a status, result kept |
| `result` | `(ptr: i32, cap: i32) -> i32` | copies up to `cap` bytes of the last result (a wire value, empty for none, or words); answers its length |
| `drop` | `(handle: i64)` | lets go of an object handed over |

- **A call**: `object` 0 → `target` is the provider's position in the
  declaration; 1 → the guest's object id. Request: wire list `[kind,
  method, arg...]`. Arguments: scalars, text, bytes, trees as themselves;
  a buffer (`bytes_mut`) as bytes of its length (write it back); an object
  as a handle, a number: taken (the guest's: `drop` it) or lent (valid
  for the call, `GUATIAO_ERR_GONE` after).
- **Its answer**: status 0 and `answer(value)`, nothing for a slot that
  answers nothing, the guest's own id (a number) for an object kind's
  answer; or a status and its words. A status guatiao does not define is
  `GUATIAO_ERR_INTERNAL`, naming it.
- **Declaration**: `{"providers": [{"id", "name", "serves": {kind: [slot,
  ..]}}]}`, at most `abi::MOST_PROVIDERS`.
- **A host object** a guest calls: its slot's arguments as scalars, text,
  bytes, borrowed or taken trees, or a taken object (a handle the guest
  owns); a lent tree, a buffer or a lent object is `GUATIAO_ERR_UNSUPPORTED`.
  An object it answers becomes a handle the guest owns.

## WASI (`wasi_snapshot_preview1`), granting nothing

Linked for a module that imports it (Go's `wasip1` reactors do), and only
these 15: `sched_yield`, `proc_exit` (stops the call), `args_get`,
`args_sizes_get`, `environ_get`, `environ_sizes_get` (empty),
`clock_time_get` (realtime, and monotonic from the instance's start),
`fd_write` (fd 1 discarded, fd 2 kept, 4 KiB, to name a failure),
`random_get` (the OS's), `poll_oneoff` (every clock subscription answered
at once, nothing sleeps), `fd_close`, `fd_fdstat_get`,
`fd_fdstat_set_flags` (fds 0-2 only), `fd_prestat_get`,
`fd_prestat_dir_name` (no directory is preopened). Any other WASI import
refuses the library.

## The engine (`WasmEngine`)

`impl guatiao_bridge::Engine`: `ThreadModel::Locked`, `type Library =
Arc<WasmLibrary>`, manifests `*.guatiao-wasm.toml`, `id_key = "module"`,
code extension `wasm`, `Opened::Described`.

```rust
fn start(_host: Host, config: &Map) -> Result<BridgeSource<WasmEngine>, ProviderError> {
    Ok(BridgeSource::shared(config)?.with_mechanism(Arc::new(Libffi)))
}
guatiao::source!(serves = "wasm", make = start);
```

- **Each opened library is one instance in a store of its own**, behind
  its own lock, called on the caller's thread: calls into different
  libraries run at once, calls into one wait their turn. A call that
  re-enters a library on the thread already inside it (a host object it
  called calling it back) is `GUATIAO_ERR_BUSY`; an object destroyed then
  is released at the library's next call.
- **A trap or a limit discards the instance**: the call answers the error,
  and the next call makes the library afresh from its compiled module.
  What the instance held goes with it: its state, the objects it answered
  (`GUATIAO_ERR_GONE`) and the host objects it was handed (destroyed).

## Limits

From the configuration's `limits`, per call, opening a library included
(instantiation, `_initialize`, the ABI check and the declaration each get
the whole budget):

| limit | kept by |
| --- | --- |
| `operations` | wasmtime fuel (on only when set) |
| `time_ms` | epoch deadlines, a 1 ms ticker thread per source (on only when set) |
| `memory` | a limiter refusing linear memory past the limit, in bytes (tables to 2^20 entries with it) |
| `depth` | refused at start (`GUATIAO_ERR_BAD_VALUE`): a guest's stack overflowing traps on its own |

A stop is `GUATIAO_ERR_INTERNAL` ("stopped at its operations limit
(200000)", "stopped at its time_ms limit (50)"), or `GUATIAO_ERR_ALLOC`
for memory ("stopped at its memory limit (16777216)").

## Errors

| guest | status |
| --- | --- |
| a refusal | its status and words |
| a status guatiao does not define | `GUATIAO_ERR_INTERNAL`, naming it |
| a trap | `GUATIAO_ERR_INTERNAL`: "`<module>` trapped (`<trap>`): `<last words>`", the first line of fd 2 or a Rust guest's panic |
| `proc_exit` | `GUATIAO_ERR_INTERNAL`: "`<module>` exited with `<code>`" |
| bytes outside its memory | `GUATIAO_ERR_INTERNAL` |
| an answer that is not a value | `GUATIAO_ERR_BAD_VALUE` |
| a limit | as above |

## Values (`WireBytes`)

```rust
impl Convert for WireBytes { type Native<'a> = Vec<u8>; type Error = String; }
```

Every shared wire vector crosses unchanged: a guest receives exactly the
tree the host has.

## The C surface

```c
guatiao_status guatiao_source(const guatiao_host_info *host,
                              const guatiao_value *config,      /* a map, or null */
                              const guatiao_source_info **out,
                              guatiao_provider_error *err);
```

As `guatiao.h` declares any source's entry. The file declares
`source=wasm`, so a scan reports it as a source and never maps it as a
library. A configuration that is not a map is `GUATIAO_ERR_WRONG_KIND`; a
key that is not a list of text, a misspelled limit or `depth`,
`GUATIAO_ERR_BAD_VALUE`.

## Gotchas

- **A guest runs on the calling thread's stack**, up to 512 KiB for its
  own frames and 512 KiB for the host's below them; an entry with less
  than that left runs on a 4 MiB stack of its own, so any thread may call.
- **A wait can deadlock a library**: a host object a guest calls that
  waits for another thread calling the same library waits forever.
- **Opening compiles**: a Go module takes seconds (a 1.6 MB one, 2.6 s in
  a release build on one x86-64 core); a refresh compiles again only a
  library whose stamp changed. Re-making an instance after a stop does
  not compile.
- **Go's runtime starts in about 430 000 fuel**: an `operations` limit
  below that refuses every Go library at open.
- **Without a `memory` limit** a guest may grow to 4 GiB, and its answers
  are copied into the host.
- **`cargo test` builds the example guests** (the `wasm32-unknown-unknown`
  target, Go 1.24 or later); without them the tests say why and skip, and
  `GUATIAO_REQUIRE_ALL=1` makes that a failure. The C host's test needs
  `GUATIAO_LIBRARY` (a `guatiao` build with `load`).
