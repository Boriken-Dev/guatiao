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

| key | effect |
| --- | --- |
| `label = "..."` | display name |
| `help = "..."` | help text; a `///` doc comment sets this when absent |
| `section = "..."` | which section the option belongs to |
| `order = <int>` | sort position |
| `advanced` | hide behind an "advanced" toggle |
| `sensitive` | never render, never log, never quote in an error |
| `default = <expr>` | the declared default |

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

- **Named-field structs only.** A tuple struct, unit struct, enum, union
  or generic type is refused at compile time, with a message naming the
  derive you actually wrote rather than "one of the derives on this
  struct".
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
