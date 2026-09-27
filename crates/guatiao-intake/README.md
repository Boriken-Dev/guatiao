# guatiao-intake

[![Crate](https://img.shields.io/crates/v/guatiao-intake.svg)](https://crates.io/crates/guatiao-intake)
[![docs.rs](https://img.shields.io/docsrs/guatiao-intake)](https://docs.rs/guatiao-intake)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

How a [guatiao](https://crates.io/crates/guatiao) schema is **shown**: what
its sections are called and the order they come in, which control draws a
field, and when a field is visible — as a value that sits beside the schema
and never repeats it.

> **Status: alpha.** Released in lockstep with `guatiao`.

## Features

- **Three layers, one per question** — the value says what is being passed,
  the schema what it is and what a valid one looks like, and **the form how
  to show one to a person**. A consumer with no screen never compiles this,
  which is why it is a crate.
- **Only what no field can say about itself** — a field's `title`,
  `description`, section (`x-section`), position (`x-order`) and whether it is
  advanced or a secret stay in the schema. A form adds what a section is
  **called**, which **control** draws a field, a **unit** beside a number, and
  **when** a field is shown.
- **A path grammar** — `agent[1].name[home].host` names one place inside a
  value, and a flat `key -> text` projection stores one.
- **The judgement is exported** — `check`, `layout` and `is_visible` in C, so
  two renderers never work the rules out differently.

## Installation

```toml
[dependencies]
guatiao-intake = { version = "0.0.0-alpha.0", features = ["derive"] }
```

| Flag | Adds | Needed for |
| --- | --- | --- |
| `derive` | `guatiao-derive` | `#[derive(Form)]`, beside `guatiao`'s `#[derive(Schema)]` |
| `c-header` | `cbindgen` | regenerating the committed C header |

## Quick start

```rust
use guatiao::schema::read::SchemaRef;
use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao_intake::{Form, FormBuilder, FormRef, Hints, Section, check, is_visible, layout};
# fn main() -> Result<(), Box<dyn std::error::Error>> {

let schema = SchemaBuilder::new()
    .field(
        FieldBuilder::new("host", KindBuilder::string())
            .section("net"),     // the FIELD's hint, not the kind's
    )
    .field(FieldBuilder::new("verify", KindBuilder::bool()))
    .field(FieldBuilder::new("ca", KindBuilder::string()))
    .finish()?;

let form = Form::new()
    .section(Section::new("net").label("Network"))
    .field("ca", Hints::new().placeholder("/etc/ssl/ca.pem").visible_when("verify", true))
    .finish()?;

let (s, f) = (SchemaRef::new(&schema).unwrap(), FormRef::new(&form).unwrap());
check(s, f)?;                       // every path names a field; every condition can be met
let groups = layout(s, f);          // sections in order, fields in order

let mut entered_so_far = guatiao::Map::new();
entered_so_far.set("verify", false)?;
let shown = is_visible(s, f, "ca", &entered_so_far.into())?;
# assert_eq!(groups.len(), 2, "the default section, then Network");
# assert!(!shown, "`ca` waits on `verify`, which holds false");
# Ok(())
# }
```

With the `derive` feature the same form is declared on the type,
beside its schema:

```rust,ignore
#[derive(guatiao::Schema, guatiao_intake::Form)]
#[form(section(id = "net", label = "Network"))]
struct Connection {
    #[schema(section = "net")]
    host: String,
    verify: bool,
    #[form(placeholder = "/etc/ssl/ca.pem", visible_when(field = "verify", equals = true))]
    ca: Option<String>,
}
let form = Connection::form(guatiao::Alloc::rust())?;   // `guatiao_intake::Screen`
```

### From C

A form is a value, so a C, Python or Dart consumer reads one with
`guatiao.h`. The judgement is exported, because two renderers that each
worked the rules out would disagree about the same form:

```c
guatiao_intake_check(schema, form, alloc, &error);      // does it fit?
guatiao_intake_layout(schema, form, alloc, &groups);    // keys, grouped and ordered
guatiao_intake_is_visible(schema, form, key, values, &shown);
```

## API overview

| Item | Purpose |
| --- | --- |
| `Form`, `Section`, `Hints` | building a form document |
| `FormRef`, `SectionRef`, `HintsRef`, `Condition` | reading one |
| `check`, `layout`, `is_visible` | the judgement |
| `for_schema`, `for_field`, `form_for` | the form a schema implies, when nobody wrote one |
| `FormBuilder`, `FormFieldBuilder`, `FormField` | a field's section, order and advanced flag, on `guatiao`'s builders |
| `path`, `flat` | naming a place inside a value; flat `key -> text` storage |
| `prelude` | both crates' halves of a declaration in one import |

## Development

Part of the [guatiao](https://github.com/Boriken-Dev/guatiao) workspace;
its README has the commands.

## License

**Mozilla Public License 2.0**, like the rest of the workspace — see
[LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
