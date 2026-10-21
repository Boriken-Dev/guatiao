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
| `examples/hello_library/` | a real cdylib the test suite builds and loads, its envelope written by hand |
| `examples/greeter_kind/` | a kind as a trait: what a host and a library both compile against |
| `examples/derived_greeter/` | a library written with no glue: `#[derive(Provider)]` and `guatiao::providers!` |
| `examples/wire_over_webtransport/` | values over WebTransport: `server/` (the shared channel and framing, `wire-server`, and `wire-probe`, a native client), `client/` (the page's wasm, its own workspace, built for `wasm32-unknown-unknown` only); commands in its `README.md` |
| `examples/greeter_host/` | a host program: scans a search path, offers what it found as the trait, builds a provider from a configuration it wrote with its own derives; `cargo run -p greeter_host` after a workspace build; its `tests/docs_snippets.rs` compiles the Rust snippets of `docs/index.md`, `docs/ownership.md` and the root `README.md` and compares them with the text shown |
| `bindings/python/` | ctypes bindings over the C ABI, `src/guatiao/` package with its own `AGENTS.md` |
| `bindings/dart/` | `dart:ffi` bindings over the C ABI, a plain Dart package (no Flutter) with its own `AGENTS.md`; `lib/src/bindings.g.dart` is ffigen output and never hand-edited |
| `docs/` | the documentation site (mkdocs): `index.md`, `ownership.md` (passing data between a host and a library without needless copies), `encodings.md` (every value encoding, stated for any language), `wasm.md`; built strictly by `docs.yml` |
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

The loader and the kind runtime, through descriptors the host links, under
Stacked Borrows and again with `-Zmiri-tree-borrows` appended. Registering
resolves a path (isolation off) and a registry leaks its host block by
design (leak check off); `scan::` holds no `unsafe` and runs natively only:

```bash
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-ignore-leaks" \
  cargo +nightly-2026-09-21 miri test -p guatiao --all-features --lib \
  -- library::raw:: library::kind:: library::registry::
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

Python bindings, after `cargo build --workspace --all-features`:

```bash
GUATIAO_LIBRARY=target/debug dotagents py -- -m pytest bindings/python/tests -rs
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
  named `ci-*` tag, watch the run, then delete the tag. Its `python` job
  builds the workspace in each leg, then runs `bindings/python`'s tests: every
  Python from 3.9 to 3.14 on Linux, 3.9 and 3.14 on Windows and macOS.
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
in `Cargo.toml`, `bindings/dart/pubspec.yaml`, and the same version the
PEP 440 way in `bindings/python/pyproject.toml` and its `__version__`
(`0.0.0-alpha.0` is `0.0.0a0`), the `=x.y.z` pins between the crates, and the
`guatiao` entry of `examples/wire_over_webtransport/client/Cargo.lock` (a
`--locked` build refuses a stale one; `cargo build` in that directory
rewrites it). A `## [x.y.z] - <date>` section in
`CHANGELOG.md` (its body becomes the release notes) and in
`bindings/dart/CHANGELOG.md`. Rehearse with `gh workflow run release.yml`,
then push the tag `vx.y.z` -- the tag is the release, and a registry never
takes a version back. The `version` job refuses a tag the manifests do not
name.

**What the tag does**: gates (tests, strict docs build, one version), then
packages all three (`cargo publish --workspace --dry-run`, `python -m build`
+ `twine check`, `dart pub publish --dry-run`), then the GitHub release
(marked pre-release for a tag with a `-`), then publishes. Each publish job
skips a version its registry already has, so re-running a partly failed
release finishes it. The four crates go in dependency order: `guatiao-derive`,
`guatiao`, `guatiao-serde`, `guatiao-intake`, each naming the others with
`=x.y.z`.

**How each registry is reached**:

| registry | first release | afterwards |
| --- | --- | --- |
| crates.io | `CARGO_REGISTRY_TOKEN` secret: trusted publishing needs a crate that exists | trusted publishing per crate, then repository variable `CRATES_IO_TRUSTED_PUBLISHING=true` and delete the secret |
| PyPI | a *pending* trusted publisher: repository `Boriken-Dev/guatiao`, workflow `release.yml`, environment `pypi` | the same publisher |
| pub.dev | **by hand**, `dart pub publish` in `bindings/dart`: automated publishing needs a package that exists | automated publishing from `Boriken-Dev/guatiao`, tag pattern `v{{version}}`, environment `pub-dev` |

**Who owns what.** Neither registry lets a new package start under an
organisation, so each is published by a person and handed over:

- **crates.io** has no organisations; a crate is owned by people and GitHub
  teams. The four crates belong to the `boriken-dev` team of the
  `Boriken-Dev` organisation: `cargo owner --add github:Boriken-Dev:boriken-dev
  <crate>` on each, once it exists. A team owner may publish but not change
  owners, so one person stays an owner as well.
- **pub.dev** calls an organisation a verified publisher. `guatiao` belongs
  to `boriken.dev`: after the first `dart pub publish`, the package's Admin
  tab transfers it there, and automated publishing is configured on the same
  tab.
