# guatiao-derive

The proc-macro half of `guatiao`'s `derive` feature: `#[derive(ToValue)]`,
`#[derive(FromValue)]` and `#[derive(Schema)]`, for a struct with named
fields, an enum of unit variants, or an enum that names its tag.

This crate is not meant to be named by a consumer. Enable it through the
crate it serves:

```toml
[dependencies]
guatiao = { version = "0.1", features = ["derive"] }
```

The traits the generated code implements live in `guatiao`; this crate
holds only the macros, because a proc-macro crate can export nothing else.

## Licence

**Mozilla Public License 2.0**, the same as the crate it serves. Per-file
copyleft: the code this macro *expands into* is yours, and carries no
obligation from this crate.
