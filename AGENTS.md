# guatiao — repository orientation

Contributor notes for a checkout. The shipped API header for each crate is
the `AGENTS.md` beside that crate's `Cargo.toml` (it lands with the crate's
public surface); this file is about the repository.

## Layout

| path | what |
| --- | --- |
| `crates/guatiao/` | the crate: value model, C type vocabulary, schema, library envelope, loader |
| `crates/guatiao/include/guatiao.h` | the C header, rendered by `build.rs` and committed |
| `crates/guatiao-derive/` | `#[derive(ToValue, FromValue, Schema)]`; reached through `guatiao`'s `derive` feature, never named directly |
| `.github/workflows/test.yaml` | the on-demand test workflow |

## Commands

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all --check
cargo package --list -p guatiao --allow-dirty   # must list AGENTS.md, README.md and no dot-prefixed path
```

Regenerating the header. `build.rs` renders it on every build with
`c-exports` on, into `OUT_DIR`; this writes the committed copy as well.
cbindgen is an **optional** build-dependency enabled by that feature, so a
default build of the crate still pulls in nothing — which is why the
render is gated rather than unconditional.

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-exports
```

## Rules

- **A default build pulls in nothing.** Every dependency sits behind a
  default-off feature; a required dependency needs an argument nobody has
  made yet.
- **No process-global state in `guatiao`.** No interning table, no static
  registry, no allocator singleton. This is what lets a host and a library
  both link the crate.
- **`unsafe` lives under `crates/guatiao/src/ffi/` and nowhere else.** Every other
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
a uniquely named `ci-*` tag, watch the run, then delete the tag. `cla.yaml`
gates pull requests. There is no release workflow and `publish = false`
everywhere: the licence is settled, but the crate is not offered on a
registry yet.
