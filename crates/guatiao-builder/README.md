# guatiao-builder

guatiao's side of [coqui-builder](https://github.com/Boriken-Dev/coqui-builder): a guatiao
**schema, value and source made from the description of a struct built field by field**, so
the struct can be shown in a form, held as a guatiao value or read from one with no
second description.

> **Status: alpha, not published.** coqui-builder is a git dependency until it is released,
> and a crate with one cannot be published.

## Features

- **A schema from a description** — one to one: the types guatiao already describes keep
  their schema, a duration is a number of seconds or its text, a field with a written
  `None` is `anyOf` the text and the field's schema.
- **A value as a source and as an inspected thing** — `ValueSource` fills a `Partial`,
  `ValueView` lets a report, a profile or `differences` judge a value as they judge a built
  one.
- **A described type as a value** — `to_value`, `from_value` and `from_value_over`, by the
  text of each leaf.
- **A form's flat store** — `form_source` reads it through the schema.
- **`Described<T>` and `with_module!`** — a described type as a guatiao value, a schema and
  a field of a struct guatiao derives.

## Installation

```toml
[dependencies]
guatiao-builder = { git = "https://github.com/Boriken-Dev/guatiao" }
coqui-builder = { git = "https://github.com/Boriken-Dev/coqui-builder", features = ["derive"] }
```

## Quick start

```rust
use coqui_builder::Describe;
use guatiao_builder::{schema, to_value};

#[derive(Describe)]
struct Listener {
    host: String,
    #[map(default = 8080)]
    port: u16,
}

// A guatiao schema from the description, for a form or an editor.
let document = schema(&Listener::description()).unwrap();

// The value a built `Listener` is, and the `Listener` a value holds.
let value = to_value(&Listener { host: "h".into(), port: 1 }).unwrap();
let back: Listener = guatiao_builder::from_value(&value).unwrap();
# let _ = (document, back);
```

## API overview

| Item | Purpose |
| --- | --- |
| `schema`, `schema_in`, `kind_in` | the schema, or the kind of a field, of a description |
| `ValueSource`, `partial` | a value as what a `Partial` is filled from |
| `ValueView` | a value read through its description as an `Inspect` |
| `to_value`, `to_value_in`, `from_value`, `from_value_over`, `FromError` | a described type to a value and back |
| `form_source` | a form's flat `key -> text` store as a value |
| `Described`, `with_module!` | a described type as a guatiao value, schema and field |
| `text` | the text of a leaf, and back |

## License

MPL 2.0.
