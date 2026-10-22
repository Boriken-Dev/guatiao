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
| `crates/guatiao-py/` | a source of libraries written in Python: a cdylib exporting `guatiao_source`, built with pyo3 for the stable ABI; a workspace of its own (no other build needs a Python to link), not published |
| `examples/hello_library/` | a real cdylib the test suite builds and loads, its envelope written by hand |
| `examples/greeter_kind/` | a kind as a trait: what a host and a library both compile against; `python/greeter_kind.py` and `node/greeter_kind.ts` describe its kinds for Python and Node; `go/` is the Go package of its kinds, their glue written by `guatiao-kind` (`go generate`); `julia/GreeterKind` declares them for Julia with `@kind`; `r/greeterkind` is the R package declaring them with `kind()`; `matlab/+greeterkind` declares them for MATLAB and Octave as classes inheriting `guatiao.Kind`; `fortran/` is the fpm package `greeter_kind`, the kinds as abstract types with their remotes and `bind(c)` glue |
| `examples/derived_greeter/` | a library written with no glue: `#[derive(Provider)]` and `guatiao::providers!` |
| `examples/wire_over_webtransport/` | values over WebTransport: `server/` (the shared channel and framing, `wire-server`, and `wire-probe`, a native client), `client/` (the page's wasm, its own workspace, built for `wasm32-unknown-unknown` only); commands in its `README.md` |
| `examples/greeter_host/` | a host program: scans a search path, offers what it found as the trait, builds a provider from a configuration it wrote with its own derives; `cargo run -p greeter_host` after a workspace build; its `tests/docs_snippets.rs` compiles the Rust snippets of `docs/index.md`, `docs/ownership.md`, `docs/migrating.md` and the root `README.md` and compares them with the text shown |
| `examples/python_host/` | a host program in Python: scans for greeters in any language, optionally adds `guatiao-py`, calls every slot; `bindings/python/guatiao/tests/test_examples.py` runs it |
| `examples/python_provider/` | two libraries written in Python, each a module and a `*.guatiao-py.toml` manifest; commands in its `README.md` |
| `examples/greeter_caller/` | a cdylib whose C functions call every slot of a greeter, or of a `wide` provider, through the kind's Rust proxy, for a host in another language to have its providers called from Rust |
| `examples/package.json` | the Node examples' one dependency, on `bindings/node`: `npm ci` here, after `npm run build` there (its dependencies live in `bindings/node_modules`) |
| `examples/node_host/` | a host program in TypeScript: scans for greeters in any language, registers Node libraries, optionally adds `guatiao-py`, calls every slot; `bindings/node/test/examples.test.ts` runs it |
| `examples/node_provider/` | `node_greeter/`, a library written in TypeScript: a package whose module is the library |
| `examples/go.mod` | the Go examples' module, using `bindings/go` through a `replace`; `examples/internal/` holds what its tests drive in C |
| `examples/go_host/` | a host program in Go: scans for greeters in any language, registers the Go library, optionally adds `guatiao-py`, calls every slot; its tests call the Rust, C, Python and Go greeters both ways |
| `examples/go_provider/` | `go_greeter/`, a library written in Go (the package is the library), and `cshared/`, the same as a `-buildmode=c-shared` file the Rust example host scans |
| `examples/julia_host/` | a host program in Julia: scans for greeters in any language, registers the Julia library, optionally adds `guatiao-py`, calls every slot; the binding's tests run it |
| `examples/julia_provider/` | `JlGreeter/`, a library written in Julia (the package is the library) |
| `examples/r_host/` | a host program in R: scans for greeters in any language, registers the R library, optionally adds `guatiao-py`, calls every slot; the binding's tests run it |
| `examples/r_provider/` | `rgreeter/`, a library written in R (the package is the library) |
| `examples/matlab_host/` | a host program in MATLAB or Octave: scans for greeters in any language, registers the M library, optionally adds `guatiao-py`, calls every slot; the binding's tests run it |
| `examples/matlab_provider/` | `mlgreeter/`, a library written in M (the folder is the library, named by its `DESCRIPTION`) |
| `examples/fortran_host/` | a host program in Fortran (an fpm package): scans for greeters in any language, registers the Fortran library, optionally adds `guatiao-py`, calls every slot; its test calls every greeter both ways |
| `examples/fortran_greeter/` | a library written in Fortran (an fpm package named and versioned as its source states, which its test checks); its `CMakeLists.txt` builds it as a library file a scan finds |
| `bindings/python/guatiao/` | ctypes bindings over the C ABI, `src/guatiao/` package with its own `AGENTS.md`; `guatiao.ffi` is what a package over another guatiao library builds on |
| `bindings/python/guatiao-serde/`, `bindings/python/guatiao-intake/` | the packages `guatiao-serde` and `guatiao-intake` (`import guatiao_serde`, `guatiao_intake`), each over its own library, with its own `AGENTS.md` |
| `bindings/dart/` | `dart:ffi` bindings over the C ABI, a plain Dart package (no Flutter) with its own `AGENTS.md`; `lib/src/bindings.g.dart` (from `guatiao.h`) and `lib/ffi_symbols.yaml` are ffigen output and never hand-edited; `lib/ffi.dart` is what a package over another guatiao library builds on |
| `bindings/dart-serde/`, `bindings/dart-intake/` | the pub.dev packages `guatiao_serde` and `guatiao_intake`, each over its own library and its own ffigen output (reusing the core's types through its symbol file); `pubspec_overrides.yaml` takes the core from the checkout and is not published |
| `bindings/node/` | Node.js and TypeScript bindings over the C ABI through koffi, an ESM package (npm `guatiao`) with its own `AGENTS.md`; `src/abi.ts` holds every struct layout the binding reads; `src/ffi.ts` (`guatiao/ffi`) is what a package over another guatiao library builds on |
| `bindings/node-serde/`, `bindings/node-intake/` | the npm packages `guatiao-serde` and `guatiao-intake`, each over its own library and the core's koffi; their tests run against the core's sources |
| `bindings/package.json` | the npm workspace of the three Node packages, with the one `package-lock.json`: `npm ci` there installs all three and links the core into the others |
| `bindings/go/` | Go bindings over the C ABI through cgo, the module `github.com/Boriken-Dev/guatiao/bindings/go` with its own `AGENTS.md`: a package per crate (`guatiao`, `serde`, `intake`), each over its own library, the other two building on what `guatiao` exports for them (`Load`, `Adopt`, `WithNodes`, `NodeOf`, `Borrow`, `StatusError`); `internal/native/` holds every line of C and copies of the three headers (a test compares them with the committed ones); `cmd/guatiao-kind` writes a kind package's glue |
| `bindings/julia/` | Julia bindings over the C ABI through `ccall`, the package `Guatiao` with its own `AGENTS.md`; `src/abi.jl` mirrors the header's structs (a test compares each layout with the header's text); `src/ffi.jl` (`Guatiao.FFI`) is what a package over another guatiao library builds on; `test/fixtures/` holds the C its tests build |
| `bindings/julia-serde/`, `bindings/julia-intake/` | the packages `GuatiaoSerde` and `GuatiaoIntake`, each over its own library, depending on `Guatiao` |
| `bindings/r/` | R bindings over the C ABI, the source package `guatiao`: `src/` holds a copy of `guatiao.h` (a test compares it with the committed one) and the C glue, compiled when the package installs, which opens the libraries at run time and calls kind slots through libffi; it registers the C table `guatiao_r_api` (`inst/include/guatiao_r_api.h`) the other packages build on; its API header is `inst/AGENTS.md`; `tests/fixtures/` holds the C its tests build |
| `bindings/r-serde/`, `bindings/r-intake/` | the source packages `guatiao.serde` and `guatiao.intake`: their own header copies and small C glue over `guatiao_r_api`, depending on `guatiao` and compiled against its installed headers |
| `bindings/fortran/` | Fortran bindings over the C ABI in pure Fortran (`iso_c_binding`), the fpm package `guatiao` with its own `AGENTS.md` and a `CMakeLists.txt`; `src/guatiao_c.f90`, `guatiao_serde_c.f90` and `guatiao_intake_c.f90` mirror the three headers (a test compares every interface's name and argument count with them); `guatiao_loader.F90` opens the library at run time; the repository's root `fpm.toml` is the same package for an fpm git dependency (a test checks the two agree) |
| `bindings/matlab/` | MATLAB and GNU Octave bindings over the C ABI: the `+guatiao` package in `inst/` (its API header is `inst/AGENTS.md`), and `src/`, one MEX gateway in C with copies of the three headers (a test compares them with the committed ones), built by `mkoctfile --mex` at `pkg install` or by `guatiao.build()`, which opens the library at run time and calls kind slots through libffi; `tests/` is a plain runner (`run_tests`) both interpreters run, `tests/fixtures/` holds the C its tests build |
| `docs/` | the documentation site (mkdocs): `index.md`, `ownership.md` (passing data between a host and a library without needless copies), `encodings.md` (every value encoding, stated for any language), `python.md`, `node.md`, `go.md`, `julia.md`, `r.md`, `matlab.md` and `fortran.md` (Python, Node, Go, Julia, R, MATLAB and Octave, and Fortran as a host and as a provider), `wasm.md`, `migrating.md` (every break since `0.0.0-alpha.0`, with what to write now); built strictly by `docs.yml` |
| `benchmarks/` | the benchmark runner (`cargo run -p guatiao-bench --release -- --save NAME`), committed results in `benchmarks/results/`, and `README.md` with the protocol; run on demand, numbers are local and provisional |
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

Two gates on a pinned nightly (`rustup toolchain install nightly-2026-09-21
--component miri,rust-src`, `cargo install cargo-public-api --version
0.52.0`). Miri runs the tests that load no library; the deep-tree test is
skipped for time only:

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
for c in guatiao guatiao-serde guatiao-intake; do
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
(cd bindings && npm ci)                       # the workspace: the three packages
(cd bindings/node && npm run build)
(cd examples && npm ci)                       # the example host's test
for p in node node-serde node-intake; do      # each suite also runs alone
  (cd bindings/$p && npm run typecheck && GUATIAO_LIBRARY=../../target/debug npm test)
done
(cd bindings/node-serde && npm run build) && (cd bindings/node-intake && npm run build)
(cd bindings/node && npm pack --dry-run)      # what ships; the same in the other two
```

Go bindings, after `cargo build --workspace --all-features`, with Go
1.26 or later and cgo (on Windows a MinGW-style compiler: llvm-mingw or
MinGW-w64 GCC). The examples are a module of their own; after a change to
a kind's header or interfaces, `go generate ./greeter_kind/go` there:

```bash
export GUATIAO_LIBRARY=$PWD/target/debug
(cd bindings/go && go vet ./... && go test ./...)
(cd examples && go vet ./... && go test ./...)   # GUATIAO_PY_BRIDGE adds the Python greeters
gofmt -l bindings/go examples                     # prints nothing
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
julia --project=bindings/julia -e 'using Pkg; Pkg.test()'
for p in serde intake; do   # each in a temporary environment holding the core from the checkout
  julia -e "using Pkg; Pkg.activate(temp=true); Pkg.develop([PackageSpec(path=\"bindings/julia\"),
    PackageSpec(path=\"bindings/julia-$p\")]); Pkg.test(\"Guatiao${p^}\")"
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
R CMD build bindings/r && R CMD check --no-manual guatiao_*.tar.gz
R CMD INSTALL guatiao_*.tar.gz     # the others compile against it
for p in serde intake; do
  R CMD build bindings/r-$p && R CMD check --no-manual guatiao.${p}_*.tar.gz
done
R CMD INSTALL bindings/r examples/greeter_kind/r/greeterkind examples/r_provider/rgreeter
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
cd bindings/matlab/tests && octave-cli --no-gui --eval "run_tests('Exit', true)"
matlab -batch "cd bindings/matlab/tests; assert(run_tests())"   # MATLAB
octave-cli --eval "addpath bindings/matlab/inst examples/matlab_host; host('target/debug')"   # the example host
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
(cd bindings/fortran && fpm test --compiler gfortran)
cmake -S bindings/fortran -B build/fortran && cmake --build build/fortran && ctest --test-dir build/fortran
(cd examples/fortran_greeter && fpm test --compiler gfortran)
(cd examples/fortran_host && fpm test --compiler gfortran)
cmake -S examples/fortran_greeter -B build/fortran-library && cmake --build build/fortran-library
GUATIAO_FORTRAN_LIBRARY=build/fortran-library cargo test -p greeter_host --test fortran_library
```

`cargo test -p greeter_host` rebuilds the workspace's cdylibs with only
what that package needs: build the workspace again before the Go, Julia,
R, MATLAB, Octave, Fortran, Python, Dart or Node suites.

The `guatiao-py` bridge, against the Python it will run with. Its tests
and the Python suite's bridge tests find it in its own `target/debug`
(or `GUATIAO_PY_BRIDGE`); on Windows the directory holding `python3.dll`
must be on `PATH` for a host that is not Python:

```bash
PYO3_PYTHON=python3 cargo build --manifest-path crates/guatiao-py/Cargo.toml
GUATIAO_LIBRARY=target/debug cargo test --manifest-path crates/guatiao-py/Cargo.toml
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
  module carries `#![forbid(unsafe_code)]`.
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
  three platforms and the floor SDK. Its `node` job installs the
  `bindings/` workspace and runs the three packages' type checks and tests
  on Node 22 and 24 on the three platforms, with the C greeter built and
  the guatiao-py bridge serving the Python greeters (a stated skip fails
  it), and checks what `npm pack` ships for each. Its `go` job runs `bindings/go` and
  `examples` (vet, gofmt, tests) on Go 1.26 and 1.27 on the three
  platforms, with the C greeter, the guatiao-py bridge and a stated skip
  failing the job, under `-race` and `GOEXPERIMENT=cgocheck2` on one leg,
  and builds the Go library as a file for the Rust example host's
  `go_library` test. Its `julia` job runs `bindings/julia`'s tests on
  Julia's LTS and current release on the three platforms, with the C
  greeter, the guatiao-py bridge and the Go library file, a stated skip
  failing the job, then `GuatiaoSerde`'s and `GuatiaoIntake`'s, each in a
  temporary environment holding the core from the checkout. Its `r` job
  runs `R CMD check` of `bindings/r` on R 4.2 and the current release on
  the three platforms, with every greeter the `julia` job has, a stated
  skip or a check WARNING failing the job, then installs it and checks
  `guatiao.serde` and `guatiao.intake` against it.
  Its `octave` job runs `bindings/matlab`'s suite on Ubuntu's Octave and
  MSYS2's on Windows, and its `matlab` job on MATLAB R2024a and the latest
  release (through `setup-matlab`) on the three platforms, each with every
  greeter the `julia` job has and a stated skip failing the job. Its
  `fortran` job runs `bindings/fortran` (fpm and CMake), the greeter
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
in `Cargo.toml`; `bindings/dart/pubspec.yaml` and those of `dart-serde` and
`dart-intake` (each with its `guatiao: ^x.y.z`); the same version the PEP 440
way in `bindings/python/guatiao/pyproject.toml` and its `__version__`
(`0.0.0-alpha.0` is `0.0.0a0`), and in `python-serde` and `python-intake`
(each with its `__version__` and its `guatiao==x.y.z`); the `=x.y.z` pins
between the crates; `crates/guatiao-py/Cargo.toml`'s own version and pins (a
workspace of its own, so the workspace version does not reach it);
`bindings/node/package.json`, `node-serde`'s and `node-intake`'s (each with
its `guatiao` dependency), `bindings/package-lock.json` and the
`../bindings/node` entry of `examples/package-lock.json` (`npm install` in
`bindings` and in `examples` rewrites them); `bindings/julia/Project.toml`
and those of `julia-serde` and `julia-intake`; `bindings/r/DESCRIPTION` and
those of `r-serde` and `r-intake` (each with `guatiao (>= x.y.z.N)`; R takes
no letters in a version: `0.0.0-alpha.N` is written `0.0.0.N`, so the first
release that is not a pre-release must be above `0.0.0`); `bindings/matlab/DESCRIPTION`
(the Octave package and the MATLAB toolbox both take the version as
written), `bindings/fortran/fpm.toml`, the root `fpm.toml` and
`bindings/fortran/CMakeLists.txt`'s project version (fpm takes three
numbers: `0.0.0-alpha.N` is written `0.0.0`), and the
`guatiao` entry of `examples/wire_over_webtransport/client/Cargo.lock` (a
`--locked` build refuses a stale one; `cargo build` in that directory
rewrites it). A `## [x.y.z] - <date>` section in
`CHANGELOG.md` (its body becomes the release notes) and in the
`CHANGELOG.md` of `bindings/dart`, `dart-serde` and `dart-intake` (pub
warns on a changelog that names no section for the version, which only a
rehearsal lets pass). Rehearse with `gh workflow run release.yml`,
then push the tag `vx.y.z` -- the tag is the release, and a registry never
takes a version back. The `version` job refuses a tag the manifests do not
name.

**What the tag does**: gates (tests, strict docs build, one version), then
packages everything (`cargo publish --workspace --dry-run`; `python -m
build` + `twine check` of the three Python packages; `dart pub publish
--dry-run` of the three Dart packages; `npm pack --dry-run` then `npm
pack` of the three Node packages; the Go module vetted and built; the
Fortran package built as a dependency and with CMake; the three Julia
packages resolved and loaded; the three R source packages built and
checked; the Octave package built, installed and used; and the MATLAB
toolbox packaged), then the GitHub release (carrying the R source
packages, the Octave package and the MATLAB toolbox, which no registry
takes)
(marked pre-release for a tag with a `-`), then publishes, and tags the
Go module `bindings/go/vx.y.z` at the same commit. Each publish job
skips a version its registry already has, so re-running a partly failed
release finishes it. The four crates go in dependency order: `guatiao-derive`,
`guatiao`, `guatiao-serde`, `guatiao-intake`, each naming the others with
`=x.y.z`; in every other registry the core package goes first and the serde
and intake packages after it.

**How each registry is reached**:

| registry | first release | afterwards |
| --- | --- | --- |
| crates.io | `CARGO_REGISTRY_TOKEN` secret: trusted publishing needs a crate that exists | trusted publishing per crate, then repository variable `CRATES_IO_TRUSTED_PUBLISHING=true` and delete the secret |
| PyPI | a *pending* trusted publisher for each of `guatiao`, `guatiao-serde` and `guatiao-intake`: repository `Boriken-Dev/guatiao`, workflow `release.yml`, environment `pypi` (`guatiao` has its publisher; the other two need theirs before their first release) | the same publishers |
| pub.dev | **by hand**, `dart pub publish` in `bindings/dart`, then in `bindings/dart-serde` and `bindings/dart-intake` once `guatiao` at that version is on pub.dev (their `pubspec_overrides.yaml` is not published): automated publishing needs a package that exists | automated publishing for each package from `Boriken-Dev/guatiao`, tag pattern `v{{version}}`, environment `pub-dev` |
| npm | `NPM_TOKEN` secret (an automation token), environment `npm`: trusted publishing needs a package that exists, so the token publishes `guatiao`, `guatiao-serde` and `guatiao-intake` the first time; a pre-release is published under the `next` dist-tag, with provenance | a trusted publisher on each of the three for `Boriken-Dev/guatiao`, workflow `release.yml`, environment `npm`, then repository variable `NPM_TRUSTED_PUBLISHING=true` and delete the secret |
| Go module proxy | the tag `bindings/go/vx.y.z`, pushed by `release.yml` with the workflow's own token; the job then asks proxy.golang.org for the version, which lists it | the same; nothing to configure |
| R-universe | nothing in `release.yml`: the owner's universe (`Boriken-Dev/boriken-dev.r-universe.dev`) lists each package in its `packages.json`, `{"package": "guatiao", "url": "https://github.com/Boriken-Dev/guatiao", "subdir": "bindings/r"}`, and the same for `guatiao.serde` (`"subdir": "bindings/r-serde"`) and `guatiao.intake` (`"subdir": "bindings/r-intake"`); R-universe builds binaries for every OS from the repository, the core first as their `Depends` and `LinkingTo` name it; CRAN only if the owner asks | the same |
| fpm registry | nothing in `release.yml`: an fpm consumer depends on the repository by its tag (the root `fpm.toml`; one package, its serde and intake modules in it); a listing in the fpm registry only on the owner's word | the same |
| Julia General | the JuliaRegistrator app installed on `Boriken-Dev/guatiao`, and a `JULIA_REGISTRATOR_TOKEN` secret (a token of an account Registrator trusts): `release.yml` comments `@JuliaRegistrator register subdir=bindings/julia`, then `subdir=bindings/julia-serde` and `subdir=bindings/julia-intake`, on the release commit. General takes no version with pre-release data and wants a first version of `0.0.1`, `0.1.0` or `1.0.0`, so a pre-release tag registers nothing; before the first registration of `GuatiaoSerde` and `GuatiaoIntake`, their `Project.toml` gain a `[compat]` entry for `Guatiao` (General requires one) | the same; a version General lists is skipped |

**Who owns what.** Neither registry lets a new package start under an
organisation, so each is published by a person and handed over:

- **crates.io** has no organisations; a crate is owned by people and GitHub
  teams. The four crates belong to the `boriken-dev` team of the
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
