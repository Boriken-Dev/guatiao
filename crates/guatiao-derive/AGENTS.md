# guatiao-derive — API header

Three derive macros for the `guatiao` value model. **Do not depend on this
crate directly**; it is re-exported from the crate it serves:

```toml
guatiao = { version = "0.1", features = ["derive"] }
```

A macro and a trait live in different namespaces, so `ToValue` names both
and a consumer writes one import — serde's arrangement, for the same
reason.

## Exports

| derive | attributes it reads | generates |
| --- | --- | --- |
| `ToValue` | `map` | `impl ToValue`: the type as a map |
| `FromValue` | `map` | `impl FromValue`: the type from a map |
| `Schema` | `map`, `schema` | `impl Schema`: a schema value for the type |

## Attributes

**`#[map(...)]`** — how a field crosses. Read by all three, so a type
cannot describe a value it refuses.

| key | effect |
| --- | --- |
| `rename = "..."` | the key to use instead of the field name |
| `skip` | the field does not cross; `FromValue` fills it from `Default` |

**`#[schema(...)]`** — what `Schema` adds. Ignored by the other two.

| key | effect | written as |
| --- | --- | --- |
| `label = "..."` | display name | `title` |
| `help = "..."` | help text; a `///` doc comment sets this when absent | `description` |
| `section = "..."` | which section the field belongs to | `x-section` |
| `order = <int>` | sort position | `x-order` |
| `advanced` | hide behind an "advanced" toggle | `x-advanced` |
| `sensitive` | never render, never log, never quote in an error | `x-sensitive` |
| `default = <expr>` | the declared default | `default` |

The right-hand column is what lands in the document, because **a schema IS
a JSON Schema**. The field's own name is its key in `properties`, and
whether it is required — a field that is not an `Option<T>` — is a name in
the struct's `required` list. Neither is written inside the field.

**Enums**, in two shapes. Refused otherwise, with a sentence saying why.

| declaration | value | `Schema::kind` |
| --- | --- | --- |
| unit variants only | the variant's name, as a string | `type: "string"` + `enum` + `x-enum-labels` |
| `#[map(tag = "k")]` on the enum | a map: name under `k` first, then the variant's fields | `type: "object"` + `x-variant-tag` + `oneOf` |

| on | attribute | effect |
| --- | --- | --- |
| enum | `#[map(tag = "...")]` | the key the variant's name is stored under; required when any variant has fields |
| variant | `#[map(rename = "...")]` | the name stored instead of the variant's own |
| variant | `#[schema(label = "...")]` | a choice's or an arm's label |
| variant | `#[schema(help = "...")]` | an arm's help — **tagged enums only**; a choice has no help slot |
| a variant's field | anything a struct field takes | means the same thing |

**A doc comment fills the most descriptive slot the thing has.** A field
or an arm has a label and help, so its doc comment is help; a choice has
only a label, so its doc comment is the label.

Refused, each with its own message: an enum with no variants; a
data-carrying enum with no `tag`; a tuple variant; two variants stored
under one name; a field stored under the tag; a misspelled enum-level
`#[map(...)]` key (ignoring it would change the wire shape); `#[map(skip)]`
on a variant.

A value the enum writes is one its own schema accepts, for both shapes —
`validate_value` over `to_value` is what the test suite checks.

```rust
#[derive(ToValue, FromValue, Schema, PartialEq, Debug)]
struct Connection {
    /// Where to connect.
    host: String,
    port: u16,
    #[map(rename = "view-only")]
    view_only: bool,
    #[schema(sensitive)]
    password: Option<String>,
}
```

## Contract

- **Named-field structs, unit enums and tagged enums.** A tuple struct,
  unit struct, untagged data-carrying enum, union or generic type is
  refused at compile time, with a message naming the derive you actually
  wrote rather than "one of the derives on this struct".
- **Reading is lenient, validating is strict.** A generated reader ignores
  keys it does not declare, for a struct and for a tagged variant's arm
  alike; `schema::validate` is what refuses them.
- **`Option<T>` is omitted when `None`** rather than written as a stored
  null. Both spellings read back as `None`, so the two agree.
- **A key may contain a NUL.** Keys are pointer and length, so the derive
  does not invent a restriction the value model does not have.
- **Every path the expansion emits is rooted** at `::guatiao::` or
  `::core::`. A test asserts this token by token: generated code that
  named a third crate would fail in every consumer that expanded it while
  passing here.
- Errors are `compile_error!` at the offending span, never a panic.

## Gotcha

The expansion names `::guatiao::` paths, so the `guatiao` crate must keep
its root re-exports (`Value`, `Map`, `Alloc`, `ToValue`, `FromValue`,
`MapError`, `ValueError`) wherever the modules underneath move. Rearranging
`guatiao`'s internals is free; renaming a root re-export breaks every
already-expanded macro.
