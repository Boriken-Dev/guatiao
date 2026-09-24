# guatiao-derive — API header

Three derive macros for the `guatiao` value model, and — behind the
`provider` feature — the attribute and derive that make a provider kind a
trait. **Do not depend on this crate directly**; it is re-exported from
the crate it serves:

```toml
guatiao = { version = "0.1", features = ["derive"] }     # ToValue, FromValue, Schema
guatiao = { version = "0.1", features = ["provider"] }   # plus #[guatiao::kind], #[derive(Provider)]
```

A macro and a trait live in different namespaces, so `ToValue` names both
and a consumer writes one import — serde's arrangement, for the same
reason.

## Exports

All three register `map` and `schema`, so one declaration compiles under
any of them; what each derive **acts on** differs.

| derive | attributes it acts on | generates |
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

**`#[schema(...)]`** — what `Schema` adds. Registered by all three, so
a field carrying one compiles under `ToValue` and `FromValue` alone; those
two parse it and ignore it.

| key | effect | written as |
| --- | --- | --- |
| `title = "..."` | display name | `title` |
| `description = "..."` | the prose under it; a `///` doc comment sets this when absent | `description` |
| `section = "..."` | which section the field belongs to | `x-section` (a) |
| `order = <int>` | sort position | `x-order` (a) |
| `advanced` | hide behind an "advanced" toggle | `x-advanced` (a) |
| `sensitive` | never render, never log, never quote in an error | `x-sensitive` |
| `default = <expr>` | the declared default | `default` |

The right-hand column is what lands in the document, because **a schema IS
a JSON Schema** -- and each key is named after what it writes.

**(a) is `guatiao-intake`'s vocabulary**, not `guatiao`'s: it names
`x-section`, `x-order` and `x-advanced`, because how to group and order
controls is an opinion the value crate does not hold. The expansion
writes them through `guatiao::schema::Extras`, the general door, with the
key **spelled as a literal** -- so a crate deriving a schema never
depends on the form crate. That literal is a seam;
`crates/guatiao-intake/tests/form_derive.rs` is what notices if the two
sides drift. The field's own name is its key in `properties`, and
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
| variant | `#[schema(title = "...")]` | an arm's title — **tagged enums only**: an arm is a subschema |
| variant | `#[schema(description = "...")]` | an arm's description — **tagged enums only** |
| variant | `#[schema(label = "...")]` | a choice's label — **unit enums only**: it lands in `x-enum-labels`, whose word is `label` |
| a variant's field | anything a struct field takes | means the same thing |

**A doc comment fills the most descriptive slot the thing has.** A field
or an arm has a description as well as a title, so its doc comment is the
description; a choice has only a label, so its doc comment is the label.

Refused, each with its own message: an enum with no variants; a
data-carrying enum with no `tag`; a tuple variant; two variants stored
under one name; a field stored under the tag; a misspelled container-level
`#[map(...)]` key (ignoring it would change the wire shape); `tag` on a
**struct**, which has no variants to tell apart; `#[map(skip)]` on a
variant.

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

## `#[guatiao::kind]` and `#[derive(Provider)]` (feature `provider`)

`provider = ["syn/full"]`: parsing a whole `trait` item needs the full
parser, which a consumer deriving only `Schema` should not pay for.

**`#[kind]` on a trait** (`#[kind(name = "...")]` names the kind; the
default is the trait's ident in snake case, `SessionBackend` →
`"session_backend"`). Emits the trait verbatim, then:

| item | what |
| --- | --- |
| `<Trait>Vtable` | `repr(C)`: `header: KindHeader`, then one `Option<unsafe extern "C" fn>` slot per method, declaration order |
| `<Trait>Vtable::of::<T>()` | `const fn`, the table for an implementation; `floor()`; `<method>_end()` per slot; `FLOOR_HASH` as a **literal** the attribute computed, so a C header generator renders it as `#define <table>_FLOOR_HASH n` |
| `impl Kind for dyn Trait` | `NAME`, `Vtable`, `FLOOR`, `FLOOR_HASH` (FNV-1a over `provider;` or `object;` and the required signatures), `REQUIRED`, `OBJECT`, `as_dyn`/`as_dyn_mut` |
| `impl Trait for Remote<dyn Trait>` | the proxy: reads each slot under the table's size, marshals, calls, converts back |
| `impl From<Remote<dyn Trait>> for Box<dyn Trait>` | `Box` only: `Arc` is not fundamental (`Kind::shared`) |

Accepted method shapes — receiver `&self`, the trait names `Send + Sync`:

| position | crosses as |
| --- | --- |
| integers, floats, `bool` | by value |
| `&str` / `&[u8]` | `Str` / `Bytes` views |
| `&Value`, `&Map` | non-null pointers |
| `Option<&Value>` | a pointer that may be null |
| any other argument type, by value | `*const Value` via `ToValue` (the shim reads it with `FromValue`) |
| `Object<dyn K>` (`K` an object kind) | an `ObjectRaw` by value; **ownership crosses with the call**, the callee destroys it — taken first in the shim, before anything can fail |
| return `()`, scalar, `Value`, `Map`, `List`, `String` | an out-pointer (`String` as `Text`) |
| any other return type | `*mut Value` via `ToValue`, read back with `FromValue` |
| return `Object<dyn K>` | `out: *mut ObjectRaw`; the caller owns what it receives |
| `Result<X, ProviderError>` | `X` as above plus `err: *mut ProviderError`; **required** for any method that converts or passes an `Object` |

A method **with a default body** is an appended slot: an older table
lacks it and the proxy runs the default. A required method after a
defaulted one is refused. A method that cannot fail propagates a provider
failure as a panic. Refused by name, on the author's span: a generic or
`unsafe` trait, a `where` clause, missing `Send + Sync`, an empty trait,
associated consts/types, `async`, generic or variadic methods, `&mut
self`/`self`/no receiver, a `&mut` argument, a borrowed argument of any
other type, a value type by value as an argument, `impl Trait` anywhere,
a function argument, a borrowed or `impl Trait` return, a `Result` whose
error is not `ProviderError`, and an attribute key other than `name`
and `object`. Every message is pinned by text in
`kind::tests::kind_rejects_by_name`; the expansion of a two-method trait
is pinned by `src/snapshots/greeter.expected.rs`.

**`#[kind(object)]` on a trait** — an **object kind**: a handle one
caller owns (a session, a scan, a stream), never offered by a registry.
The trait names `Send` (`Sync` optional); methods may take `&mut self`;
a `&mut [u8]` argument crosses as a `BytesMut` out-buffer (the one
`&mut` argument allowed, and only here). What differs in the expansion:

| item | what |
| --- | --- |
| `<Trait>Vtable` | `header`, then **`destroy: Option<unsafe extern "C" fn(ctx)>`**, then the slots; `destroy_end()`; `destroy` is the first entry of `REQUIRED` |
| `Kind::OBJECT` | `true`; the hash is over `object;` + the required signatures, so an object table never passes for a provider table with the same methods |
| the trait | gains `fn into_object(self) -> Object<dyn Trait> where Self: Sized + Send + 'static`: boxes the value beside a table `of::<Self>()` (an `ObjectCell`) and hands back the handle |
| the shims | address the value through that cell (`object_mut`), `&mut` for every method; `__guatiao_destroy::<T>` frees the cell |

A Rust implementation becomes a handle with `value.into_object()`; the
receiving side drives it through `Object<dyn Trait>` (`Deref`/`DerefMut`
to the trait) and drops it, which calls `destroy` exactly once. A C
implementation fills the same table and hands `{table, size, ctx}` as a
`guatiao_object`.

**`#[derive(Provider)]`** with `#[provider(...)]`:

| key | effect | default |
| --- | --- | --- |
| a bare path, or `kinds(A, B)` | the kinds this type serves; a `T: Kind` bound is checked per kind | required, at least one |
| `id = "..."` | the provider id | `{CARGO_PKG_NAME}_{type}` in snake case, `-` as `_` |
| `name = "..."` | display name | the type's ident |
| `version = "..."` | the provider's own version | empty: the library's |
| `config = C` | the provider is **built from `C`**: schema through `C: Schema`, decoded through `C: FromValue`, built through `Self: TryFrom<C, Error: Into<ProviderError>>` (`C = Self` is the identity); emits the `create`/`destroy` slots, so a host gets an `Instance` per configuration whose address is the `ctx` | none: the type is its one instance |
| `new = path` / `new_with_host = path` | `fn() -> Self` / `fn(Host) -> Self` building the default instance (the descriptor's `ctx`); with `config` and neither, there is no default instance | `Default`, except with `config` |
| `available = path` | `fn(&Self) -> Result<(), &'static str>`, asked on every call of the default instance; refused when there is none | always available |

Emits, inside a `const _` block: one `static` table per kind
(`<dyn K as Kind>::Vtable::of::<T>()`, reached through the trait so only
the trait need be in scope), a `OnceLock<T>` holding the instance, and
`impl ProviderDecl for T` building a `ProviderParts`. Refused: a generic
type, no kinds, an unknown key, a key given twice, both `new` and
`new_with_host`. `guatiao::providers!(A, B)` (in the core crate) then
writes the whole library.

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
- **Every path the expansion emits is rooted** at `::guatiao::`,
  `::core::` or `::std::` — the last for `::std::vec::Vec::new()`, the
  list a schema's fields are collected into. A test asserts this token by
  token, over all three derives: generated code that named a third crate
  would fail in every consumer that expanded it while passing here.
- Errors are `compile_error!` at the offending span, never a panic.

## Gotcha

The expansion names `::guatiao::` paths, so the `guatiao` crate must keep
its root re-exports (`Value`, `Map`, `Alloc`, `ToValue`, `FromValue`,
`MapError`, `ValueError`) wherever the modules underneath move. Rearranging
`guatiao`'s internals is free; renaming a root re-export breaks every
already-expanded macro.
