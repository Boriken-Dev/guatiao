# guatiao — repository orientation

Contributor notes for a checkout. The shipped API header for each crate is
the `AGENTS.md` beside that crate's `Cargo.toml` (it lands with the crate's
public surface); this file is about the repository.

## Layout

| path | what |
| --- | --- |
| `crates/guatiao/` | the crate: value model, C type vocabulary, schema, library envelope, loader |
| `crates/guatiao/include/guatiao.h` | the C header, rendered by `build.rs` and committed |
| `guatiao.dll` / `libguatiao.so` | the C ABI artifact, from the same crate — `cargo build` |
| `crates/guatiao-derive/` | `#[derive(ToValue, FromValue, Schema)]`, `#[guatiao::kind]`/`#[derive(Provider)]`, `#[derive(Form)]`; reached through `guatiao`'s `derive`/`provider` features and `guatiao-form`'s `derive`, never named directly |
| `crates/guatiao-serde/` | serde for values: JSON, TOML and YAML as features, and a C surface with its own `include/guatiao_serde.h` |
| `crates/guatiao-form/` | how a schema is shown: sections, widget hints, conditional visibility, as a value beside the schema, or declared with `#[derive(Form)]`; C surface in `include/guatiao_form.h` |
| `examples/hello_library/` | a real cdylib the test suite builds and loads, its envelope written by hand |
| `examples/greeter_kind/` | a kind as a trait: what a host and a library both compile against |
| `examples/derived_greeter/` | a library written with no glue: `#[derive(Provider)]` and `guatiao::providers!` |
| `examples/greeter_host/` | a host program: scans a search path, offers what it found as the trait, builds a provider from a configuration it wrote with its own derives; `cargo run -p greeter_host` after a workspace build |
| `bindings/python/` | ctypes bindings over the C ABI, `src/guatiao/` package with its own `AGENTS.md` |
| `.github/workflows/test.yaml` | the on-demand test workflow |

## Commands

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo package --list -p guatiao --allow-dirty   # must list AGENTS.md, README.md and no dot-prefixed path
```

Regenerating the header. `build.rs` renders it on every build with
`c-header` on, into `OUT_DIR`; this writes the committed copy as well.
cbindgen is an **optional** build-dependency enabled by that feature, so a
default build of the crate still pulls in nothing — which is why the
render is gated rather than unconditional.

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-serde --features c-header,json,toml,yaml
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao-form --features c-header
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

- **A default build pulls in nothing.** Every dependency sits behind a
  default-off feature; a required dependency needs an argument nobody has
  made yet.
- **No process-global state in `guatiao`.** No interning table, no static
  registry, no allocator singleton. This is what lets a host and a library
  both link the crate.
- **`unsafe` is confined to named places.** In `guatiao`: the value
  model's raw layer, `library/raw.rs`, and `exports/` — listed in
  `tests/forbid_unsafe_per_module.rs`. In `guatiao-form`: `exports.rs`
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

Two workflows. `test.yaml` runs on `workflow_dispatch` or a `ci-*` tag; push
a uniquely named `ci-*` tag, watch the run, then delete the tag. Its `python`
job builds the workspace once, then runs `bindings/python`'s tests on 3.9
and 3.14. `cla.yaml` gates pull requests. There is no release workflow and
`publish = false` everywhere: the licence is settled, but the crate is not
offered on a registry yet.
