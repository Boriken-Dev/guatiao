# Contributing to guatiao

## Licence

This project is distributed under the **Mozilla Public License 2.0**
(`LICENSE`). Its copyleft is per file, which means:

- **Using guatiao imposes nothing on your code.** Include the header, link
  the crate statically or dynamically, and ship a closed-source binary. The
  only obligation is that guatiao's own source stays available to the people
  you ship to, which it is, here.
- **Changing guatiao's own files does carry an obligation.** A modified file
  of this project must be published under the MPL 2.0. Your files beside it
  are yours, under whatever licence you choose.
- A combined work may be distributed under terms of your choosing (MPL 2.0
  section 3.3), and MPL 2.0 remains compatible with the GPL, so a GPL
  consumer can still use this.

## Contributor licence agreement

A first pull request needs a one-time signature. The bot will comment with
the exact sentence to reply with; read [`CLA.md`](CLA.md) first.

It exists so the project can be relicensed later without tracking down every
past contributor. You keep the copyright in everything you write.

## Before you open a pull request

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all --check
```

All three must be clean. See [`AGENTS.md`](AGENTS.md) for the repository's
layout and the rules that the test suite enforces rather than describes:

- a default build pulls in nothing, so every dependency sits behind a
  default-off feature;
- the crate holds no process-global mutable state;
- `unsafe` lives under `crates/guatiao/src/ffi/` and nowhere else;
- files are LF, on every platform.

## The C header is generated

`crates/guatiao/include/guatiao.h` is rendered by `build.rs` from the Rust
source and committed. Do not hand-edit it. Change the Rust, then:

```bash
GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header
```

A test compares the committed header against a fresh render, so a change to
the Rust that is not regenerated fails the build rather than shipping a
header that describes a different ABI.
