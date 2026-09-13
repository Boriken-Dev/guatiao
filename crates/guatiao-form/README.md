# guatiao-form

How a [guatiao](../guatiao) schema is **shown**: what its sections are
called and the order they come in, which control draws a field, and when a
field is visible — as a value that sits beside the schema and never
repeats it.

```rust
use guatiao::schema::read::SchemaRef;
use guatiao::schema::{FieldBuilder, FormBuilder, KindBuilder, SchemaBuilder};
use guatiao_form::{Form, FormRef, Hints, Section, check, is_visible, layout};

let schema = SchemaBuilder::new()
    .field(FieldBuilder::new("host", KindBuilder::string()).section("net"))
    .field(FieldBuilder::new("verify", KindBuilder::bool()))
    .field(FieldBuilder::new("ca", KindBuilder::string()))
    .finish()?;

let form = Form::new()
    .section(Section::new("net").label("Network"))
    .field("ca", Hints::new().placeholder("/etc/ssl/ca.pem").visible_when("verify", true))
    .finish()?;

let (s, f) = (SchemaRef::new(&schema).unwrap(), FormRef::new(&form).unwrap());
check(s, f)?;                          // every path names a field; every condition can be met
for group in layout(s, f) { /* sections in order, fields in order */ }
let shown = is_visible(s, f, "ca", &entered_so_far);
```

## Three layers

| layer | answers | crate |
| --- | --- | --- |
| value | what is being passed | `guatiao` |
| schema | what it is, and what a valid one looks like | `guatiao` |
| **form** | how to show one to a person | **`guatiao-form`** |

A consumer with no screen never compiles this, which is why it is a crate.

## What stays in the schema

Everything a field can say about itself: its `title` and `description`,
which section it belongs to (`x-section`), its position (`x-order`), and
whether it is advanced or a secret. A form adds only what no single field
can say: what a section is **called**, which **control** draws it, and
**when** it is shown.

## From C

A form is a value, so a C, Python or Dart consumer reads one with
`guatiao.h`. The judgement is exported, because two renderers that each
worked the rules out would disagree about the same form:

```c
guatiao_form_check(schema, form, alloc, &error);      // does it fit?
guatiao_form_layout(schema, form, alloc, &groups);    // keys, grouped and ordered
guatiao_form_is_visible(schema, form, key, values, &shown);
```

## Licence

MPL-2.0, like the rest of the workspace.
