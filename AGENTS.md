# guatiao — repository orientation

Contributor notes for a checkout. The shipped API header for each crate is
the `AGENTS.md` beside that crate's `Cargo.toml` (it lands with the crate's
public surface); this file is about the repository.

## Layout

| path | what |
| --- | --- |
| `crates/guatiao/` | the crate: value model (every operation on its container, `Value` thin), C type vocabulary, schema (a description of the struct, and nothing about how it is named or stored), library envelope, loader |
| `crates/guatiao/include/guatiao.h` | the C header, rendered by `build.rs` and committed |
| `guatiao.dll` / `libguatiao.so` | the C ABI artifact, from the same crate — `cargo build` |
| `crates/guatiao-derive/` | `#[derive(ToValue, FromValue, Schema)]`, `#[guatiao::kind]`/`#[derive(Provider)]`, `#[derive(Form)]`; reached through `guatiao`'s `derive`/`provider` features and `guatiao-intake`'s `derive`, never named directly |
| `crates/guatiao-serde/` | serde for values: JSON, TOML and YAML as features, and a C surface with its own `include/guatiao_serde.h` |
| `crates/guatiao-intake/` | taking a value in from a person: the path grammar (`agent[1].name`), flat `key -> text` storage, checking a store of text, and how a schema is shown (sections, widget hints, conditional visibility, `#[derive(Form)]`); C surface in `include/guatiao_intake.h` |
| `crates/guatiao-bridge/` | what every bridge to a language runtime shares: the `Engine` trait and its thread, manifests and stamps, `BridgeSource` (an engine as a `Source`), kinds served through tables built from their descriptions by the mechanism a source is given (`with_mechanism`), limits, error mapping, `Convert` and the shared vectors (`vectors/wire.json`, a copy of the core's); `#![forbid(unsafe_code)]`, published; `tests/toy_engine.rs` serves a toy language through every piece |
| `crates/guatiao-libffi/` | `Libffi`, libffi as the `Mechanism` core's run-time kind tables are built on (`library/kind/runtime.rs`): a call interface per slot signature, a closure per served slot, `ffi_call` per call; builds libffi from C, not on wasm32; published; `tests/described_kinds.rs` (with the C caller `tests/c_consumer/described_caller.c`) and `tests/closures.rs` run the tables through it natively, while core's own tests run them under Miri through a mechanism with no foreign code |
| `crates/guatiao-source-remote/` | providers called across a process, a machine or a network: `RemoteSource` (a peer's providers as a `Source`; the cdylib is the source file any host adds), `Server` over a registry, the `guatiao-remote-server` binary, one framed protocol (`protocol`, no I/O) over `transport` (a child's stdio; a Unix socket or named pipe with `local`; shared memory with `shm`; trusted TCP with `tcp`; WebSocket, what a browser opens, with `ws`; RDMA, experimental and Linux only, with `rdma`); `docs/remote.md` states it; `tests/vectors/remote.json` and `tests/c/` (a codec, a C client and server, a C host) check it from C, and the Dart client from Dart; `rdma-tests/` is a workspace of its own holding the tests that need an RDMA device (`cargo test --manifest-path crates/guatiao-source-remote/rdma-tests/Cargo.toml`; Soft-RoCE will do, and without a device every test fails); published |
| `crates/guatiao-source-py/` | a source of libraries written in Python: a cdylib exporting `guatiao_source`, built with pyo3 for the stable ABI; a workspace of its own (no other build needs a Python to link), not published |
| `crates/guatiao-source-lua/` | a source of libraries written in Lua (a guatiao-bridge engine, Lua built in from C: `lua54` default, `lua55`, `luau`), and Lua's side of guatiao (values, handles on kinds, a registry) the loadable module builds on; cdylib + rlib, a workspace of its own (exclusive Lua features), not published |
| `crates/guatiao-source-js/` | a source of libraries written in JavaScript, run by QuickJS (rquickjs) built into the file: a cdylib exporting `guatiao_source`, libraries written as the Node binding's providers; a workspace of its own (no other build needs a C compiler for QuickJS), not published; its tests are in `crates/guatiao-source-js/tests`, the Python host's in `bindings/python/guatiao/tests/test_js_source.py` |
| `examples/js_provider/` | `quickjs_greeter/`, a library in JavaScript that runs under the Node binding and the guatiao-source-js source, and `quickjs_trees/`, one taking trees as plain data; `README.md` has the commands |
| `crates/guatiao-source-wasm/` | a source of libraries compiled to WebAssembly: a cdylib exporting `guatiao_source` over a wasmtime engine (`ThreadModel::Locked`, a store per library), guatiao ABI 1 in `src/abi.rs`, a WASI preview-1 shim granting stdio, a clock and randomness; `guest/` is `guatiao-wasm-guest` (Rust guests, `wasm32-unknown-unknown`) and `guest-go/` the Go guest package (`wasip1` reactors); each its own workspace or module, not published; `tests/guests.rs` runs hand-written modules, `tests/source.rs` both example guests from a Rust and a C host |
| `examples/wasm_greeter_rust/`, `examples/wasm_greeter_go/` | greeters in Rust and in Go compiled to WebAssembly, each beside its `*.guatiao.toml`; built by `crates/guatiao-source-wasm`'s tests |
| `crates/guatiao-source-rhai/` | a source of libraries written in Rhai: a cdylib exporting `guatiao_source`, Rhai built in (libffi for the kind tables); a library is a script ending in a map of providers, with a `*.guatiao.toml` manifest; a workspace of its own, not published; its tests (a Rust host, the crate's own file, a C host) are in `crates/guatiao-source-rhai/tests`, the Python host's in `bindings/python/guatiao/tests/test_rhai_source.py` |
| `examples/rhai_provider/` | `rhai_greeter/`, greeters written in Rhai: a script and its `*.guatiao.toml`, serving every slot of the example kinds |
| `examples/source_library/` | a source written in Rust and exported with `guatiao::source!`: a Rust host and a C host add it in `crates/guatiao/tests/source_export.rs` |
| `examples/hello_library/` | a real cdylib the test suite builds and loads, its envelope written by hand |
| `examples/greeter_kind/` | a kind as a trait: what a host and a library both compile against; `include/greeter_kind.kinds.json` is the same kinds as data, written by its `description_is_current` test; `python/greeter_kind.py` is what `python -m guatiao.kinds` writes from the description (the Python suite checks it is current); `node/` is what `guatiao-kind` writes from the description (`greeter_kind.js` and `.d.ts`; the Node suite checks it is current); `dart/` is the Dart package `greeter_kind`, its `lib/greeter_kind.dart` what `dart run guatiao:kind` writes from the description (the Dart suite checks it is current); `go/` is the Go package of its kinds, their glue written by `guatiao-kind` from the description (`go generate`; its test checks it is current); `julia/GreeterKind` declares them for Julia with `@kind`; `r/greeterkind` is the R package declaring them with `kind()`; `matlab/+greeterkind` declares them for MATLAB and Octave as classes inheriting `guatiao.Kind`; `fortran/` is the fpm package `greeter_kind`, the kinds as abstract types with their remotes and `bind(c)` glue |
| `examples/tally_kind/` | a kind that exists only in TypeScript: `tally_kind.ts` declares it as interfaces (`u32`, `Lent<Watcher>`, `extends ObjectKind`), `guatiao-kind` reads them into `include/tally_kind.kinds.json` and writes `node/` from it, and `dart/` is the Dart package `dart run guatiao:kind` writes from the same document; the Node suite checks the document and the Node files, the Dart suite the Dart ones, and each serves the kind from a provider of its own |
| `examples/derived_greeter/` | a library written with no glue: `#[derive(Provider)]` and `guatiao::providers!` |
| `examples/wire_over_webtransport/` | values over WebTransport: `server/` (the shared channel and framing, `wire-server`, and `wire-probe`, a native client), `client/` (the page's wasm, its own workspace, built for `wasm32-unknown-unknown` only); commands in its `README.md` |
| `examples/app_library/` | an application's own C library (`myapp`): depends on the three crates and exports their C surfaces beside `myapp_version`; every binding's suite also runs against it alone, and it is the Dart web suites' default module |
| `examples/app_host/` | a Rust program linking the `guatiao` crate and `myapp` (`raw-dylib` on Windows); its test runs it |
| `examples/dart_host/` | a host program in Dart: calls greeters written in Rust, C, Python and Dart through the greeter kind's classes and has the Rust caller (`examples/greeter_caller`) call the Dart one; `dart build cli` makes it an AOT executable |
| `examples/dart_provider/` | `dart_greeter/`, a library written in Dart: provider classes, its identity written from its `pubspec.yaml` by `dart run guatiao:kind identity` |
| `examples/flutter_app/` | a Flutter app over the three Dart packages from the checkout: their build hooks build the libraries with cargo for each target; it prints what each surface answered, what a greeter written in Dart answered through its table, and where its library was loaded from (`GUATIAO_EXAMPLE_EXIT=1` ends a desktop run there); `pubspec.yaml` shows the user define that builds `examples/app_library` instead |
| `examples/remote_pool/` | a guatiao-source-remote host fanning one call out to three peers at once (servers on loopback ports in the same process), a registry per peer since identical devices serve the same library; `cargo run -p remote_pool`, and its test runs it |
| `examples/greeter_host/` | a host program: scans a search path, offers what it found as the trait, builds a provider from a configuration it wrote with its own derives; `cargo run -p greeter_host` after a workspace build; its `tests/docs_snippets.rs` compiles the Rust snippets of `docs/index.md`, `docs/ownership.md`, `docs/migrating.md`, `docs/configuration.md`, `docs/bridges.md` and the root `README.md` and compares them with the text shown; `tests/configuration_files.rs` reads a configuration from TOML text through guatiao |
| `examples/python_host/` | a host program in Python: scans for greeters in any language, optionally adds `guatiao-source-py`, calls every slot; `bindings/python/guatiao/tests/test_examples.py` runs it |
| `examples/python_provider/` | two libraries written in Python, each a module and a `*.guatiao.toml` manifest; commands in its `README.md` |
| `examples/lua_provider/` | `lua_greeter`, a library written in Lua (a module and its `*.guatiao.toml`) serving every slot of the example kinds |
| `examples/lua_host/` | `host.lua`, a host in Lua: loads libraries and sources, calls every slot of every greeter; guatiao-source-lua's tests run it embedded, the module's in a stock interpreter |
| `examples/greeter_caller/` | a cdylib whose C functions call every slot of a greeter, or of a `wide` provider, through the kind's Rust proxy, for a host in another language to have its providers called from Rust |
| `examples/package.json` | the Node examples' one dependency, on `bindings/node/guatiao`: `npm ci` here, after `npm run build` there (its dependencies live in `bindings/node/node_modules`) |
| `examples/node_host/` | a host program in TypeScript: scans for greeters in any language, registers Node libraries, optionally adds `guatiao-source-py`, calls every slot; `bindings/node/guatiao/test/examples.test.ts` runs it |
| `examples/node_provider/` | `node_greeter/`, a library written in TypeScript: a package whose module is the library |
| `examples/node_js/` | a library and a host in plain JavaScript, run with no build step, and kinds defined in JavaScript as classes (`echo_kind.js`), which `describe.js` writes as `echo_kind.kinds.json` for the other languages' generators; `bindings/node/guatiao/test/examples.test.ts` runs it and checks the description is current, the Dart suite generates from it |
| `examples/go.mod` | the Go examples' module, using `bindings/go/guatiao` through a `replace`; `examples/internal/` holds what its tests drive in C |
| `examples/go_host/` | a host program in Go: scans for greeters in any language, registers the Go library, optionally adds `guatiao-source-py`, calls every slot; its tests call the Rust, C, Python and Go greeters both ways |
| `examples/go_provider/` | `go_greeter/`, a library written in Go (the package is the library), and `cshared/`, the same as a `-buildmode=c-shared` file the Rust example host scans |
| `examples/julia_host/` | a host program in Julia: scans for greeters in any language, registers the Julia library, optionally adds `guatiao-source-py`, calls every slot; the binding's tests run it |
| `examples/julia_provider/` | `JlGreeter/`, a library written in Julia (the package is the library) |
| `examples/r_host/` | a host program in R: scans for greeters in any language, registers the R library, optionally adds `guatiao-source-py`, calls every slot; the binding's tests run it |
| `examples/r_provider/` | `rgreeter/`, a library written in R (the package is the library) |
| `examples/matlab_host/` | a host program in MATLAB or Octave: scans for greeters in any language, registers the M library, optionally adds `guatiao-source-py`, calls every slot; the binding's tests run it |
| `examples/matlab_provider/` | `mlgreeter/`, a library written in M (the folder is the library, named by its `DESCRIPTION`) |
| `examples/fortran_host/` | a host program in Fortran (an fpm package): scans for greeters in any language, registers the Fortran library, optionally adds `guatiao-source-py`, calls every slot; its test calls every greeter both ways |
| `examples/greeter_kind/cpp/` | the example kinds declared in C++ over `greeter_kind.h` (`GUATIAO_KIND`), what a C++ host and a C++ library include |
| `examples/cpp_greeter/` | a library written in C++ (CMake): classes serving the example kinds, a library file a scan finds (`GUATIAO_LIBRARY`) |
| `examples/cpp_host/` | a host program in C++ (CMake) and its test calling the greeters written in Rust, C, Python, Go and C++ through sources and `refresh()`; it builds the C greeter and the C++ one |
| `examples/cpp_consumer/` | a project outside the repository consuming the three C++ packages through `find_package` |
| `examples/fortran_greeter/` | a library written in Fortran (an fpm package named and versioned as its source states, which its test checks); its `CMakeLists.txt` builds it as a library file a scan finds |
| `bindings/python/guatiao/` | ctypes bindings over the C ABI, `src/guatiao/` package with its own `AGENTS.md`; `guatiao.ffi` is what a package over another guatiao library builds on |
| `bindings/python/guatiao-serde/`, `bindings/python/guatiao-intake/` | the packages `guatiao-serde` and `guatiao-intake` (`import guatiao_serde`, `guatiao_intake`), each over its own library, with its own `AGENTS.md` |
| `bindings/dart/guatiao/` | Dart bindings over the C ABI, a plain Dart package (no Flutter) with its own `AGENTS.md`: `dart:ffi` natively, the WebAssembly module on the web (`lib/src/web/`, chosen by a conditional import on `dart.library.js_interop`, `lib/wasm.dart` its counterpart of `ffi.dart`); `lib/src/bindings.g.dart` (from `guatiao.h`) and `lib/ffi_symbols.yaml` are ffigen output and never hand-edited; `lib/ffi.dart` is what a package over another guatiao library builds on; `lib/remote.dart` (`lib/src/remote/`) a client of the remote protocol over WebSocket in plain Dart, a page's way to a server's providers, its suites starting `guatiao-remote-server` (`GUATIAO_REMOTE_SERVER`, else `target/debug`, with `derived_greeter` beside it); `lib/kinds.dart` kinds and providers as classes, `bin/kind.dart` (`dart run guatiao:kind`) writing a kind description's Dart and a library identity, `lib/kind_glue.dart` what that code calls; `hook/build.dart` builds the core crate with cargo for the target a Dart or Flutter build names, through `lib/hook.dart`, which the other two packages' hooks use too; its own `pubspec.yaml` tells the hook to build nothing for its own suites |
| `bindings/dart/guatiao_serde/`, `bindings/dart/guatiao_intake/` | the pub.dev packages `guatiao_serde` and `guatiao_intake`, each over its own library and its own ffigen output (reusing the core's types through its symbol file); `pubspec_overrides.yaml` takes the core from the checkout and is not published |
| `bindings/node/guatiao/` | Node.js and TypeScript bindings over the C ABI through koffi, an ESM package (npm `guatiao`) with its own `AGENTS.md`; `src/abi.ts` holds every struct layout the binding reads; `src/ffi.ts` (`guatiao/ffi`) is what a package over another guatiao library builds on; `src/markers.ts` the types a kind written as an interface is spelled with and `src/describe.ts` (`guatiao/describe`) reads such interfaces through the TypeScript 7 compiler API into a description |
| `bindings/node/guatiao-serde/`, `bindings/node/guatiao-intake/` | the npm packages `guatiao-serde` and `guatiao-intake`, each over its own library and the core's koffi; their tests run against the core's sources |
| `bindings/node/package.json` | the npm workspace of the three Node packages, with the one `package-lock.json`: `npm ci` there installs all three and links the core into the others |
| `bindings/go/guatiao/` | Go bindings over the C ABI through cgo, the module `github.com/Boriken-Dev/guatiao/bindings/go/guatiao` (package `guatiao`) with its own `AGENTS.md`: what the other two modules build on (`Surface`, `Load`, `Adopt`, `WithNodes`, `NodeOf`, `Borrow`, `StatusError`); `internal/native/` holds its C and a copy of `guatiao.h` (a test compares it with the committed one); `cmd/guatiao-kind` writes a kind package's glue |
| `bindings/go/guatiao-serde/`, `bindings/go/guatiao-intake/` | the modules `.../bindings/go/guatiao-serde` (package `serde`) and `.../bindings/go/guatiao-intake` (package `intake`), each with its own `AGENTS.md`, over its own library, requiring the core module at the same version; each carries copies of the headers it calls, compared with the committed ones by a test |
| `bindings/go/go.work` | builds the three Go modules together from a checkout |
| `bindings/julia/Guatiao/` | Julia bindings over the C ABI through `ccall`, the package `Guatiao` with its own `AGENTS.md`; `src/abi.jl` mirrors the header's structs (a test compares each layout with the header's text); `src/ffi.jl` (`Guatiao.FFI`) is what a package over another guatiao library builds on; `test/fixtures/` holds the C its tests build |
| `bindings/julia/GuatiaoSerde/`, `bindings/julia/GuatiaoIntake/` | the packages `GuatiaoSerde` and `GuatiaoIntake`, each over its own library, depending on `Guatiao` |
| `bindings/r/guatiao/` | R bindings over the C ABI, the source package `guatiao`: `src/` holds a copy of `guatiao.h` (a test compares it with the committed one) and the C glue, compiled when the package installs, which opens the libraries at run time and calls kind slots through libffi; it registers the C table `guatiao_r_api` (`inst/include/guatiao_r_api.h`) the other packages build on; its API header is `inst/AGENTS.md`; `tests/fixtures/` holds the C its tests build |
| `bindings/r/guatiao.serde/`, `bindings/r/guatiao.intake/` | the source packages `guatiao.serde` and `guatiao.intake`: their own header copies and small C glue over `guatiao_r_api`, depending on `guatiao` and compiled against its installed headers |
| `bindings/fortran/guatiao/` | Fortran bindings over the C ABI in pure Fortran (`iso_c_binding`), the fpm package `guatiao` with its own `AGENTS.md` and a `CMakeLists.txt`; `src/guatiao_c.f90`, `guatiao_serde_c.f90` and `guatiao_intake_c.f90` mirror the three headers (a test compares every interface's name and argument count with them); `guatiao_loader.F90` opens the library at run time; the repository's root `fpm.toml` is the same package for an fpm git dependency (a test checks the two agree) |
| `bindings/cpp/guatiao/` | C++ bindings over the C ABI, header-only (`include/guatiao/`), linking the library: the CMake package `guatiao` (`guatiao::guatiao`, `guatiaoConfig.cmake`, `cmake/GuatiaoNative.cmake` finding or cargo-building the library) with its own `AGENTS.md`; `cmake/GuatiaoTests.cmake` builds every suite with exceptions on and off; `tests/compile_fail/` holds files that must not compile; `tests/support/` the harness the satellites' suites share |
| `bindings/cpp/guatiao-serde/`, `bindings/cpp/guatiao-intake/` | the CMake packages `guatiao-serde` (`guatiao::serde`) and `guatiao-intake` (`guatiao::intake`), each over its own library, with its own `AGENTS.md` |
| `bindings/matlab/guatiao/` | MATLAB and GNU Octave bindings over the C ABI: the `+guatiao` package in `inst/` (its API header is `inst/AGENTS.md`), and `src/`, one MEX gateway in C with copies of the three headers (a test compares them with the committed ones), built by `mkoctfile --mex` at `pkg install` or by `guatiao.build()`, which opens the library at run time and calls kind slots through libffi; `tests/` is a plain runner (`run_tests`) both interpreters run, `tests/fixtures/` holds the C its tests build |
| `bindings/lua/guatiao/` | the module a stock Lua requires (`require "guatiao"`, `luaopen_guatiao`): a cdylib over `guatiao-source-lua` built against the interpreter that loads it (`lua54` default, `lua55`, `luajit`), the guatiao library linked in; `guatiao-dev-1.rockspec` (`luarocks make`); its own `AGENTS.md`; a workspace of its own |
| `docs/` | the documentation site (mkdocs): `index.md`, `ownership.md` (passing data between a host and a library without needless copies), `encodings.md` (every value encoding, a kind's description among them, stated for any language), `kinds.md` (how each language declares a kind and a provider), `python.md`, `node.md`, `go.md`, `julia.md`, `r.md`, `matlab.md`, `fortran.md`, `cpp.md` and `lua.md` (Python, Node, Go, Julia, R, MATLAB and Octave, Fortran, C++ and Lua as a host and as a provider), `language-bridges.md` (a bridge to another language on guatiao-bridge), `rhai.md` (scripts written in Rhai served to any host), `bridges.md` (a format of its own on the visitor traits, from Rust, C and the bindings), `js.md` (JavaScript without Node), `wasm.md`, `migrating.md` (every break since `0.0.0-alpha.0`, with what to write now); built strictly by `docs.yml` |
| `benchmarks/` | the benchmark runner (`cargo run -p guatiao-bench --release -- --save NAME`), `python/documents.py` and `dart/` (a document to Python and Dart objects three ways), `remote/` (`guatiao-remote-bench`: `serve` and `run` over each transport, one JSON line of five metrics), committed results in `benchmarks/results/`, and `README.md` with the protocol; run on demand, numbers are local and provisional |
| `.github/workflows/test.yaml` | the on-demand test workflow |

## Commands

```bash
cargo test --workspace --all-features
cargo test -p guatiao-serde        # its default features: the tests `exact-numbers` compiles out
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo package --list -p guatiao --allow-dirty   # must list AGENTS.md, README.md and no dot-prefixed path
```

The crate builds for a browser; the module carries the C surface:

```bash
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown -p guatiao -p guatiao-serde -p guatiao-intake \
  --features guatiao/derive,guatiao/provider,guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
cargo build -p guatiao --target wasm32-unknown-unknown --release
```

The Dart suites in Chrome, under each web compiler, against a module
(`GUATIAO_WASM`, default `target/wasm32-unknown-unknown/release/myapp.wasm`,
built by `cargo build -p myapp --target wasm32-unknown-unknown --release`;
`CHROME_EXECUTABLE` names the browser where it is not found). The remote
suites start the native `guatiao-remote-server` and `derived_greeter`
(`cargo build -p guatiao-source-remote -p derived_greeter --features
guatiao-source-remote/ws`, or the workspace build), on the VM as in a
browser:

```bash
cd bindings/dart/guatiao && dart test -p chrome --compiler dart2js   # or dart2wasm
```

The Dart suites on 32-bit ARM run the libraries built for
`armv7-unknown-linux-gnueabihf` and Dart's `linux-arm` SDK under
qemu-user; `test.yaml`'s `dart-armv7` job is the recipe. A build hook runs
with a filtered environment, so `QEMU_LD_PREFIX` does not reach it: the
ARM loader goes where an ARM program looks for it.

The Flutter example, after rustup's targets for the platform (Android:
`aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`):

```bash
cd examples/flutter_app
flutter build windows          # or linux, macos, apk, ios --simulator
GUATIAO_EXAMPLE_EXIT=1 build/windows/x64/runner/Release/flutter_app.exe
```

Two gates on a pinned nightly (`rustup toolchain install nightly-2026-09-21
--component miri,rust-src`, `cargo install cargo-public-api --version
0.52.0`). Miri runs the tests that load no library; the deep-tree test is
skipped for time only, and the vector test reads its file (isolation off):

```bash
cargo +nightly-2026-09-21 miri test -p guatiao --all-features --lib -- value:: schema::
cargo +nightly-2026-09-21 miri test -p guatiao --all-features --test container_roundtrip \
  --test std_traits --test public_surface --test exports_boundary \
  --test schema_as_value --test kind_glue_probes --test schema_constraints \
  --test schema_annotations --test status_codes --test ownership \
  -- --skip a_tree_of_any_depth
cargo +nightly-2026-09-21 miri test -p guatiao-intake --all-features \
  --test paths --test flat_projection
cargo +nightly-2026-09-21 miri test -p guatiao --all-features --test wire_roundtrip \
  --test wire_channel -- --skip ten_thousand_random --skip a_tree_deeper_than \
  --skip a_thousand_keys
MIRIFLAGS="-Zmiri-disable-isolation" \
  cargo +nightly-2026-09-21 miri test -p guatiao --all-features --test wire_vectors
```

The loader, the kind runtime and sources, through descriptors the host
links, under Stacked Borrows and again with `-Zmiri-tree-borrows` appended. Registering
resolves a path (isolation off) and a registry leaks its host block by
design (leak check off); `scan::` holds no `unsafe` and runs natively only:

```bash
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-ignore-leaks" \
  cargo +nightly-2026-09-21 miri test -p guatiao --all-features --lib \
  -- library::raw:: library::kind:: library::registry:: library::source::
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-ignore-leaks" \
  cargo +nightly-2026-09-21 miri test -p guatiao --all-features \
  --test host_lookups --test provider_offers
```

Each crate commits its public surface. A change to it regenerates the
snapshot in the same commit; CI fails on any difference:

```bash
for c in guatiao guatiao-serde guatiao-intake guatiao-bridge guatiao-libffi guatiao-source-remote; do
  cargo public-api -p $c --all-features -ss > crates/$c/public-api.txt
done
```

Regenerating the header. `build.rs` renders it on every build with
`c-header` on, into `OUT_DIR`; this writes the committed copy as well.
cbindgen is an **optional** build-dependency enabled by that feature, so a
default build of the crate still pulls in nothing — which is why the
render is gated rather than unconditional.

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-serde --features c-header,json,toml,yaml
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-intake --features c-header
GUATIAO_WRITE_HEADER=1 cargo build -p greeter_kind --features c-header   # the kind tables, via macro expansion
GUATIAO_WRITE_HEADER=1 cargo test -p greeter_kind --test description_is_current   # the same kinds as data, include/greeter_kind.kinds.json
```

Lua, after `cargo build --workspace --all-features` (the Rust greeter
and the library the C hosts link): the source and the module are
workspaces of their own, built before their tests, which add them as
files. `GUATIAO_PY_BRIDGE` (a guatiao-source-py build) adds the Python
greeters, `GUATIAO_LUA` names the stock interpreter (else `lua`), and
`GUATIAO_LUA_SOURCE` the source for the module's host test:

```bash
cargo build --manifest-path crates/guatiao-source-lua/Cargo.toml
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-source-lua/Cargo.toml
cargo test --manifest-path crates/guatiao-source-lua/Cargo.toml --no-default-features --features luau,source
cargo build --manifest-path bindings/lua/guatiao/Cargo.toml
GUATIAO_LIBRARY=target/debug GUATIAO_LUA_SOURCE=crates/guatiao-source-lua/target/debug/libguatiao_source_lua.so \
  cargo test --manifest-path bindings/lua/guatiao/Cargo.toml
```

Python bindings, after `cargo build --workspace --all-features`, with the
three packages installed (`pip install -e "bindings/python/guatiao[dev]" -e
bindings/python/guatiao-serde -e bindings/python/guatiao-intake`); each suite also passes
alone:

```bash
GUATIAO_LIBRARY=target/debug dotagents py -- -m pytest bindings/python/guatiao/tests \
  bindings/python/guatiao-serde/tests bindings/python/guatiao-intake/tests -rs
MYPYPATH=bindings/python/guatiao/src:bindings/python/guatiao-serde/src:bindings/python/guatiao-intake/src:examples/greeter_kind/python \
  dotagents py -- -m mypy --strict bindings/python/guatiao/src/guatiao \
  bindings/python/guatiao-serde/src/guatiao_serde bindings/python/guatiao-intake/src/guatiao_intake \
  examples/greeter_kind/python examples/python_host \
  examples/python_provider/py_greeter examples/python_provider/py_tally
```

Node bindings, after `cargo build --workspace --all-features`, with
Node 22 or later (the examples run their TypeScript as it is, which
needs 22.18):

```bash
(cd bindings/node && npm ci)                  # the workspace: the three packages
(cd bindings/node/guatiao && npm run build)
(cd examples && npm ci)                       # the example host's test
for p in guatiao guatiao-serde guatiao-intake; do   # each suite also runs alone
  (cd bindings/node/$p && npm run typecheck && GUATIAO_LIBRARY=../../../target/debug npm test)
done
(cd bindings/node/guatiao-serde && npm run build) && (cd bindings/node/guatiao-intake && npm run build)
(cd bindings/node/guatiao && npm pack --dry-run)      # what ships; the same in the other two
```

After the greeter kinds' description changes, the Node module, the Dart
file and the Python module written from it (each suite fails until they
are):

```bash
(cd examples/greeter_kind/node && node ../../../bindings/node/guatiao/dist/kind_cli.js \
  ../include/greeter_kind.kinds.json --out greeter_kind)
(cd examples/greeter_kind/dart && dart run guatiao:kind ../include/greeter_kind.kinds.json -o lib/greeter_kind.dart)
(cd examples/dart_provider/dart_greeter && dart run guatiao:kind identity)   # after its pubspec's version moves
(cd examples/greeter_kind/python && PYTHONPATH=../../../bindings/python/guatiao/src \
  python -m guatiao.kinds ../include/greeter_kind.kinds.json -o greeter_kind.py)
```

After `examples/tally_kind/tally_kind.ts` changes, its document, the Node
module and the Dart file (the document is read by the compiler, so the
workspace's `typescript` must be installed); after
`examples/node_js/echo_kind.js` changes, its document:

```bash
(cd examples/tally_kind/node && node ../../../bindings/node/guatiao/dist/kind_cli.js ../tally_kind.ts \
  --out tally_kind --document ../include/tally_kind.kinds.json --project ../tsconfig.json)
(cd examples/tally_kind/dart && dart run guatiao:kind ../include/tally_kind.kinds.json -o lib/tally_kind.dart)
node examples/node_js/describe.js            # after npm ci in examples
```

Go bindings, after `cargo build --workspace --all-features`, with Go
1.26 or later and cgo (on Windows a MinGW-style compiler: llvm-mingw or
MinGW-w64 GCC). The examples are a module of their own; after a change to
a kind's description or interfaces, `go generate ./greeter_kind/go` there:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
(cd bindings/go && go vet ./guatiao/... ./guatiao-serde/... ./guatiao-intake/... \
  && go test ./guatiao/... ./guatiao-serde/... ./guatiao-intake/...)   # through go.work
(cd examples && go vet ./... && go test ./...)   # GUATIAO_PY_BRIDGE adds the Python greeters
gofmt -l bindings/go examples                    # prints nothing
# the Go library as a file, scanned by the Rust example host
(cd examples && go build -buildmode=c-shared -o ../go-library/libgo_greeter.so ./go_provider/cshared)
GUATIAO_GO_LIBRARY=go-library cargo test -p greeter_host --all-features --test go_library
```

Julia bindings, after `cargo build --workspace --all-features`, with
Julia 1.10 or later built for the same architecture as the libraries. The
tests find the example packages themselves; the C greeter is built with
`CC` (else `clang`, `cc` or `gcc`), the Python greeters come from the
bridge `GUATIAO_PY_BRIDGE` names, the Go greeter from the directory
`GUATIAO_GO_LIBRARY` names, and `GUATIAO_REQUIRE_ALL=1` makes a missing
one a failure:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
julia --project=bindings/julia/Guatiao -e 'using Pkg; Pkg.test()'
for p in Serde Intake; do   # each in a temporary environment holding the core from the checkout
  julia -e "using Pkg; Pkg.activate(temp=true); Pkg.develop([PackageSpec(path=\"bindings/julia/Guatiao\"),
    PackageSpec(path=\"bindings/julia/Guatiao$p\")]); Pkg.test(\"Guatiao$p\")"
done
julia examples/julia_host/host.jl target/debug   # the example host
```

R bindings, after `cargo build --workspace --all-features`, with R 4.2
or later built for the same architecture as the libraries, its compiler
setup (Rtools on Windows) and libffi. `R CMD check` runs the tests; they
need `GUATIAO_LIBRARY` and a checkout in `GUATIAO_REPO` (for the shared
vectors and the example packages, which they install themselves), build
the C greeter and their fixtures with `R CMD SHLIB`, and take the Python
and Go greeters and `GUATIAO_REQUIRE_ALL` as the Julia suite does. A
failing check WARNING counts as a failure:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug GUATIAO_REPO=$PWD
R CMD build bindings/r/guatiao && R CMD check --no-manual guatiao_*.tar.gz
R CMD INSTALL guatiao_*.tar.gz     # the others compile against it
for p in serde intake; do
  R CMD build bindings/r/guatiao.$p && R CMD check --no-manual guatiao.${p}_*.tar.gz
done
R CMD INSTALL bindings/r/guatiao examples/greeter_kind/r/greeterkind examples/r_provider/rgreeter
Rscript examples/r_host/host.R target/debug   # the example host
```

MATLAB and Octave bindings, after `cargo build --workspace --all-features`,
with MATLAB R2024a or Octave 8.4 or later built for the same architecture
as the libraries (on Windows ARM64 both are x64: build the libraries for
`x86_64-pc-windows-msvc`), a C compiler `mex` or `mkoctfile` supports, and
libffi. `run_tests` builds the gateway when it is not on the path, and the
C greeter and the `gtfixture` MEX file it needs; it takes the Python and Go
greeters and `GUATIAO_REQUIRE_ALL` as the Julia suite does:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
cd bindings/matlab/guatiao/tests && octave-cli --no-gui --eval "run_tests('Exit', true)"
matlab -batch "cd bindings/matlab/guatiao/tests; assert(run_tests())"   # MATLAB
octave-cli --eval "addpath bindings/matlab/guatiao/inst examples/matlab_host; host('target/debug')"   # the example host
```

Fortran bindings, after `cargo build --workspace --all-features`, with
gfortran, LLVM flang or Intel ifx, fpm 0.13 and CMake. The host's test
builds the C greeter (`CC`, else the first of clang, cc, gcc) and takes
the Python and Go greeters and `GUATIAO_REQUIRE_ALL` as the Julia suite
does; `GUATIAO_FORTRAN_LIBRARY` names the Fortran library file's build for
the Rust and Python hosts' tests. With flang on Windows, fpm needs
`--archiver llvm-ar` and CMake a Visual Studio environment:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
(cd bindings/fortran/guatiao && fpm test --compiler gfortran)
cmake -S bindings/fortran/guatiao -B build/fortran && cmake --build build/fortran && ctest --test-dir build/fortran
(cd examples/fortran_greeter && fpm test --compiler gfortran)
(cd examples/fortran_host && fpm test --compiler gfortran)
cmake -S examples/fortran_greeter -B build/fortran-library && cmake --build build/fortran-library
GUATIAO_FORTRAN_LIBRARY=build/fortran-library cargo test -p greeter_host --test fortran_library
```

C++ bindings, after `cargo build --workspace --all-features`, with CMake,
Ninja and MSVC, clang or g++ (C++17; `-DCMAKE_CXX_STANDARD=20` too). Each
package's suite builds twice, with exceptions on and off; the host's test
takes the Python and Go greeters and `GUATIAO_REQUIRE_ALL` as the Fortran
host's does; `GUATIAO_CPP_LIBRARY` names the C++ library file's build for
the Rust and Python hosts' tests:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
for p in guatiao guatiao-serde guatiao-intake; do
  cmake -S bindings/cpp/$p -B bindings/cpp/$p/build -G Ninja && cmake --build bindings/cpp/$p/build \
    && ctest --test-dir bindings/cpp/$p/build --output-on-failure
done
cmake -S examples/cpp_host -B examples/cpp_host/build -G Ninja && cmake --build examples/cpp_host/build \
  && ctest --test-dir examples/cpp_host/build -V
GUATIAO_CPP_LIBRARY=examples/cpp_host/build/greeters/cpp cargo test -p greeter_host --test cpp_library
```

`cargo test -p greeter_host` rebuilds the workspace's cdylibs with only
what that package needs: build the workspace again before the Go, Julia,
R, MATLAB, Octave, Fortran, C++, Python, Dart or Node suites.

The `guatiao-source-py` bridge, against the Python it will run with. Its tests
and the Python suite's bridge tests find it in its own `target/debug`
(or `GUATIAO_PY_BRIDGE`); on Windows the directory holding `python3.dll`
must be on `PATH` for a host that is not Python:

```bash
PYO3_PYTHON=python3 cargo build --manifest-path crates/guatiao-source-py/Cargo.toml
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-source-py/Cargo.toml
```

The WebAssembly source, its Rust guest crate and its Go guest package:

```bash
rustup target add wasm32-unknown-unknown        # the Rust guest; Go 1.24 or later builds the Go one
cargo build -p guatiao --features load          # what the C host's test links
GUATIAO_LIBRARY=$PWD/target/debug cargo test --manifest-path crates/guatiao-source-wasm/Cargo.toml
(cd crates/guatiao-source-wasm/guest && cargo test)
(cd crates/guatiao-source-wasm/guest-go && go vet ./... && go test ./...)
```

Each crate with a C surface commits its header and has a test comparing
the committed copy with a fresh render, so an edit nobody regenerated fails
the suite rather than shipping.

## Rules

- **guatiao is glue between languages, and every change serves that.** A
  value encoding (what a tree looks like for a type: a string with a
  grammar, a two-element list, an object with text keys) is defined at the
  value and schema level, written in `docs/encodings.md` and the C header's
  documentation, and readable and writable from C, Python and Dart: what a
  reader must parse has a C export or a binding mapping, tested against the
  same vectors as the Rust code. A Rust impl (`ToValue`, `FromValue`,
  `Schema` for a Rust type) is one implementation of an encoding and never
  introduces a shape only Rust can read. A convenience only Rust users can
  see, such as interop with a third-party Rust crate, waits until a
  consumer asks for it.
- **Nothing a host holds needs a library's allocator or its image.** A
  tree a call returns is built in, or moved into, the caller's allocator;
  every handle holds a lease on its library, so `unload` refuses while one
  is alive; `clone()` copies into this binary's own allocator. The rules
  for callers and library authors are in `docs/ownership.md`.
- **What a trusted producer wrote is checked where it enters, not walled
  off with `unsafe`.** A library the host chose to load, code the macros
  generate and the crate's own builders are trusted: descriptors are
  checked on entry (alignment, sizes, strides, lengths, addresses inside
  the image) and then read by safe functions. `unsafe` marks a raw pointer
  from anywhere (`from_raw`) and an operation that can invalidate what
  safe code holds (`unload_unchecked`).
- **A fix arrives with the test that would have caught it.** A test that
  counts allocations warms every fixture before the measured call:
  `std::sync::Mutex` allocates on its first lock on macOS, and nowhere
  else CI runs.
- **A default build pulls in nothing.** Every dependency sits behind a
  default-off feature; a required dependency needs an argument nobody has
  made yet.
- **No process-global state in `guatiao`.** No interning table, no static
  registry, no allocator singleton. This is what lets a host and a library
  both link the crate.
- **`unsafe` is confined to named places.** In `guatiao`: the value
  model's raw layer, the loader (`library/raw/`), the kind runtime's
  files (`library/kind/`), and `exports/` — listed in
  `tests/forbid_unsafe_per_module.rs`. In `guatiao-intake`: `exports.rs` and `exports_flat.rs`
  alone, checked by its `tests/unsafe_stays_in_exports.rs`. Every other
  module carries `#![forbid(unsafe_code)]`. `guatiao-bridge` forbids it
  crate-wide; `guatiao-libffi` is the libffi glue and nothing else.
- **Lines are LF** (`.gitattributes`), on every platform, including
  generated files.
- **This crate stands alone.** It names no application that embeds it.

## Licence and contributions

**MPL 2.0** (`LICENSE`), per-file copyleft. Every Rust source file carries
the Exhibit A notice; the repository-wide statement is in `README.md`. A
consumer linking this crate takes on nothing; a change to one of *this*
crate's own files must be published under the same licence.

Contributions need a one-time CLA (`CLA.md`), enforced by
`.github/workflows/cla.yaml`. It grants the maintainer copyright and patent
rights over contributions, which is what makes a future relicence possible
without tracking down past contributors.

## CI and release

Four workflows:

- `test.yaml` runs on `workflow_dispatch` or a `ci-*` tag; push a uniquely
  named `ci-*` tag, watch the run, then delete the tag. Every binding's
  serde and intake packages run beside its core, each suite alone. Its
  `python` job builds the workspace in each leg, then runs the tests of
  `bindings/python/guatiao`, `bindings/python/guatiao-serde` and `bindings/python/guatiao-intake`:
  every Python from 3.9 to 3.14 on Linux, 3.9 and 3.14 on Windows and
  macOS. Its `dart` job analyzes and tests the three Dart packages on the
  three platforms and the floor SDK; `dart-web` runs them in Chrome under
  dart2js and dart2wasm against the application module and intake's, and
  `dart-armv7` on 32-bit ARM under qemu-user against the separate
  libraries and the application's. Its `flutter` job builds
  `examples/flutter_app` on each platform for its desktop, twice (the
  second must run no hook) and runs it; the APK on Linux, checked for
  every library on every ABI; the iOS simulator app on macOS. Its `node` job installs the
  `bindings/node` workspace and runs the three packages' type checks and tests
  on Node 22 and 24 on the three platforms, with the C greeter built and
  the guatiao-source-py bridge serving the Python greeters (a stated skip fails
  it), and checks what `npm pack` ships for each. Its `go` job runs the three Go modules
  through `bindings/go/go.work`, serde and intake again each alone (their
  own `go.mod`, the core replaced by the checkout's), and `examples` (vet,
  gofmt, tests) on Go 1.26 and 1.27 on the three
  platforms, with the C greeter, the guatiao-source-py bridge and a stated skip
  failing the job, under `-race` and `GOEXPERIMENT=cgocheck2` on one leg,
  and builds the Go library as a file for the Rust example host's
  `go_library` test. Its `julia` job runs `bindings/julia/Guatiao`'s tests on
  Julia's LTS and current release on the three platforms, with the C
  greeter, the guatiao-source-py bridge and the Go library file, a stated skip
  failing the job, then `GuatiaoSerde`'s and `GuatiaoIntake`'s, each in a
  temporary environment holding the core from the checkout. Its `r` job
  runs `R CMD check` of `bindings/r/guatiao` on R 4.2 and the current release on
  the three platforms, with every greeter the `julia` job has, a stated
  skip or a check WARNING failing the job, then installs it and checks
  `guatiao.serde` and `guatiao.intake` against it.
  Its `octave` job runs `bindings/matlab/guatiao`'s suite on Ubuntu's Octave and
  MSYS2's on Windows, and its `matlab` job on MATLAB R2024a and the latest
  release (through `setup-matlab`) on the three platforms, each with every
  greeter the `julia` job has and a stated skip failing the job. Its
  `fortran` job runs `bindings/fortran/guatiao` (fpm and CMake), the greeter
  package and the host's test with every greeter, and has the Rust and
  Python example hosts scan the Fortran library file: gfortran 13 and 15,
  Intel ifx and LLVM flang on Ubuntu, gfortran 15 on macOS, gfortran 15
  and ifx on Windows, a stated skip failing the job.
  `release.yml` calls it as its test gate.
- `docs.yml` owns every Pages deploy: a push to `main` touching a docs
  source, or a dispatch at any ref. `release.yml` calls it with
  `deploy: false` as a strict build gate, and dispatches it at a final tag.
- `release.yml` releases a `v*` tag, and rehearses one on `workflow_dispatch`
  (every gate, build and dry run; nothing released, nothing published).
- `cla.yaml` gates pull requests.

**Versions.** While every release is a pre-release (`0.0.0-alpha.N`), a
change that breaks the documented API ships as the next alpha, with its
migration under `[Unreleased]` in `CHANGELOG.md`; work lands there and the
version moves only when a release is cut.

**Cutting a release.** One version everywhere: `[workspace.package] version`
in `Cargo.toml`; the `pubspec.yaml` of `bindings/dart/guatiao`,
`guatiao_serde` and `guatiao_intake` (the last two with `guatiao: ^x.y.z`);
the same version the PEP 440 way in the `pyproject.toml` of
`bindings/python/guatiao`, `guatiao-serde` and `guatiao-intake` and each one's
`__version__` (`0.0.0-alpha.0` is `0.0.0a0`; the last two pin
`guatiao==x.y.z`); the `=x.y.z` pins between the crates;
`crates/guatiao-source-py/Cargo.toml`'s own version and pins (a workspace of its
own, so the workspace version does not reach it); the `package.json` of
`bindings/node/guatiao`, `guatiao-serde` and `guatiao-intake` (the last two
with their `guatiao` dependency), `bindings/node/package-lock.json` and the
`../bindings/node/guatiao` entry of `examples/package-lock.json` (`npm
install` in `bindings/node` and in `examples` rewrites them); the
`Project.toml` of `bindings/julia/Guatiao`, `GuatiaoSerde` and
`GuatiaoIntake` (the last two with `[compat] Guatiao = "=x.y.z"` once the
version is not a pre-release, which Pkg cannot bound, and with none before); the `DESCRIPTION` of `bindings/r/guatiao`, `guatiao.serde`
and `guatiao.intake` (the last two with `guatiao (>= x.y.z.N)`; R takes
no letters in a version: `0.0.0-alpha.N` is written `0.0.0.N`, so the first
release that is not a pre-release must be above `0.0.0`); `bindings/matlab/guatiao/DESCRIPTION`
(the Octave package and the MATLAB toolbox both take the version as
written), `bindings/fortran/guatiao/fpm.toml`, the root `fpm.toml` and
`bindings/fortran/guatiao/CMakeLists.txt`'s project version (fpm takes three
numbers: `0.0.0-alpha.N` is written `0.0.0`), `GUATIAO_CPP_VERSION` in
`bindings/cpp/guatiao/include/guatiao/value.hpp` (as written) and the
project version of each `bindings/cpp/*/CMakeLists.txt` (three numbers, as
fpm's), the `require` of the core
module in `bindings/go/guatiao-serde/go.mod` and `guatiao-intake/go.mod`
(`vx.y.z`; a Go module's own version is its tag), and the
`guatiao` entry of `examples/wire_over_webtransport/client/Cargo.lock` (a
`--locked` build refuses a stale one; `cargo build` in that directory
rewrites it). A `## [x.y.z] - <date>` section in
`CHANGELOG.md` (its body becomes the release notes) and in the
`CHANGELOG.md` of `bindings/dart/guatiao`, `guatiao_serde` and `guatiao_intake` (pub
warns on a changelog that names no section for the version, which only a
rehearsal lets pass). Rehearse with `gh workflow run release.yml`,
then push the tag `vx.y.z` -- the tag is the release, and a registry never
takes a version back. The `version` job refuses a tag the manifests do not
name.

**What the tag does**: gates (tests, strict docs build, one version), then
packages everything (`cargo publish --workspace --dry-run`; `python -m
build` + `twine check` of the three Python packages; `dart pub publish
--dry-run` of the three Dart packages; `npm pack --dry-run` then `npm
pack` of the three Node packages; the three Go modules vetted, built
and each built alone by a consumer outside the checkout; the
Fortran package built as a dependency and with CMake; the three Julia
packages resolved and loaded; the three R source packages built and
checked; the Octave package built, installed and used; and the MATLAB
toolbox packaged), then the GitHub release (carrying the R source
packages, the Octave package and the MATLAB toolbox, which no registry
takes)
(marked pre-release for a tag with a `-`), then publishes, and tags the
Go modules `bindings/go/guatiao/vx.y.z`, `bindings/go/guatiao-serde/vx.y.z`
and `bindings/go/guatiao-intake/vx.y.z` at the same commit. Each publish job
skips a version its registry already has, so re-running a partly failed
release finishes it. The seven crates go in dependency order: `guatiao-derive`,
`guatiao`, `guatiao-serde`, `guatiao-intake`, `guatiao-libffi`, `guatiao-bridge`,
`guatiao-source-remote`, each naming
the others with `=x.y.z`; in every other registry the core package goes first and the serde
and intake packages after it.

**How each registry is reached**:

| registry | first release | afterwards |
| --- | --- | --- |
| crates.io | `CARGO_REGISTRY_TOKEN` secret: trusted publishing needs a crate that exists (so a new crate, `guatiao-bridge`, `guatiao-libffi` or `guatiao-source-remote` the first time, is published with the token) | trusted publishing per crate, then repository variable `CRATES_IO_TRUSTED_PUBLISHING=true` and delete the secret |
| PyPI | a *pending* trusted publisher for each of `guatiao`, `guatiao-serde` and `guatiao-intake`: repository `Boriken-Dev/guatiao`, workflow `release.yml`, environment `pypi` (`guatiao` has its publisher; the other two need theirs before their first release) | the same publishers |
| pub.dev | **by hand**, `dart pub publish` in `bindings/dart/guatiao`, then in `bindings/dart/guatiao_serde` and `bindings/dart/guatiao_intake` once `guatiao` at that version is on pub.dev (their `pubspec_overrides.yaml` is not published): automated publishing needs a package that exists | automated publishing for each package from `Boriken-Dev/guatiao`, tag pattern `v{{version}}`, environment `pub-dev` |
| npm | `NPM_TOKEN` secret (an automation token), environment `npm`: trusted publishing needs a package that exists, so the token publishes `guatiao`, `guatiao-serde` and `guatiao-intake` the first time; a pre-release is published under the `next` dist-tag, with provenance | a trusted publisher on each of the three for `Boriken-Dev/guatiao`, workflow `release.yml`, environment `npm`, then repository variable `NPM_TRUSTED_PUBLISHING=true` and delete the secret |
| Go module proxy | a tag per module, `bindings/go/guatiao/vx.y.z`, `bindings/go/guatiao-serde/vx.y.z` and `bindings/go/guatiao-intake/vx.y.z`, pushed by `release.yml` with the workflow's own token, the core first; the job then asks proxy.golang.org for each, which lists it | the same; nothing to configure |
| R-universe | nothing in `release.yml`: the owner's universe (`Boriken-Dev/boriken-dev.r-universe.dev`) lists each package in its `packages.json`, `{"package": "guatiao", "url": "https://github.com/Boriken-Dev/guatiao", "subdir": "bindings/r/guatiao"}`, and the same for `guatiao.serde` (`"subdir": "bindings/r/guatiao.serde"`) and `guatiao.intake` (`"subdir": "bindings/r/guatiao.intake"`); R-universe builds binaries for every OS from the repository, the core first as their `Depends` and `LinkingTo` name it; CRAN only if the owner asks | the same |
| fpm registry | nothing in `release.yml`: an fpm consumer depends on the repository by its tag (the root `fpm.toml`; one package, its serde and intake modules in it); a listing in the fpm registry only on the owner's word | the same |
| Julia General | the JuliaRegistrator app installed on `Boriken-Dev/guatiao`, and a `JULIA_REGISTRATOR_TOKEN` secret (a token of an account Registrator trusts): `release.yml` comments `@JuliaRegistrator register subdir=bindings/julia/Guatiao`, then `subdir=bindings/julia/GuatiaoSerde` and `subdir=bindings/julia/GuatiaoIntake`, on the release commit. General takes no version with pre-release data and wants a first version of `0.0.1`, `0.1.0` or `1.0.0`, so a pre-release tag registers nothing; before the first registration of `GuatiaoSerde` and `GuatiaoIntake`, their `Project.toml` gain a `[compat]` entry for `Guatiao` (General requires one) | the same; a version General lists is skipped |

**Who owns what.** Neither registry lets a new package start under an
organisation, so each is published by a person and handed over:

- **crates.io** has no organisations; a crate is owned by people and GitHub
  teams. The seven crates belong to the `boriken-dev` team of the
  `Boriken-Dev` organisation: `cargo owner --add github:Boriken-Dev:boriken-dev
  <crate>` on each, once it exists. A team owner may publish but not change
  owners, so one person stays an owner as well.
- **pub.dev** calls an organisation a verified publisher. `guatiao`,
  `guatiao_serde` and `guatiao_intake` belong to `boriken.dev`: after each
  one's first `dart pub publish`, its Admin tab transfers it there, and
  automated publishing is configured on the same tab.
- **npm**: the three packages are published by the token's account first;
  adding the `Boriken-Dev` organisation (or its team) as a maintainer of
  each is the owner's.
