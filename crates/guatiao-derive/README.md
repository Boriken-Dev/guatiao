# guatiao-derive

[![Crate](https://img.shields.io/crates/v/guatiao-derive.svg)](https://crates.io/crates/guatiao-derive)
[![docs.rs](https://img.shields.io/docsrs/guatiao-derive)](https://docs.rs/guatiao-derive)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

The proc-macro half of [`guatiao`](https://crates.io/crates/guatiao)'s
`derive` feature (and, behind `form`, of
[`guatiao-intake`](https://crates.io/crates/guatiao-intake)'s:
`#[derive(Form)]`): **`#[derive(ToValue)]`, `#[derive(FromValue)]` and
`#[derive(Schema)]`, for a struct with named fields, an enum of unit
variants, or an enum that names its tag.**

> **Status: alpha.** Released in lockstep with `guatiao`; every version
> names the other exactly.

## Features

- **One declaration, three derives** — the value's shape, its conversion
  both ways and its JSON Schema all read the same fields, so the schema
  cannot describe a value the type refuses.
- **Attributes named after what they write** — `#[schema(title, description,
  default, sensitive, read_only, examples(..))]` on a field, `#[schema(min_length,
  pattern, multiple_of, min_items, ..)]` on its kind, `#[map(rename, skip,
  default)]` for the key and what an absent one reads as.
- **Refusals at compile time, with a sentence** — a tuple struct, an untagged
  enum with data, two fields on one key, a step that divides nothing.

## Installation

Not named directly. Enable it through the crate it serves:

```toml
[dependencies]
guatiao = { version = "0.0.0-alpha.0", features = ["derive"] }
```

The traits the generated code implements live in `guatiao`; this crate
holds only the macros, because a proc-macro crate can export nothing else.

## Development

Part of the [guatiao](https://github.com/Boriken-Dev/guatiao) workspace;
its README has the commands.

## License

**Mozilla Public License 2.0**, the same as the crate it serves — see
[LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
Per-file copyleft: the code this macro *expands into* is yours, and carries
no obligation from this crate.
