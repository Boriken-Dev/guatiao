# guatiao-derive — API header

Three derive macros for the `guatiao` value model, and — behind the
`provider` feature — the attribute and derive that make a provider kind a
trait. **Do not depend on this crate directly**; it is re-exported from
the crate it serves:

```toml
guatiao = { version = "=0.0.0-alpha.1", features = ["derive"] }     # ToValue, FromValue, Schema
guatiao = { version = "=0.0.0-alpha.1", features = ["provider"] }   # plus #[guatiao::kind], #[derive(Provider)]
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
| `default` | an absent key reads as the field type's `Default`; the field leaves the schema's `required` and the schema's `default` is that value |
| `default = <expr>` | the same, with an expression of the field's type |
| `flatten` | the field's own map is written beside the other keys and read from the whole map; its schema's fields join this one's. Stands alone (no `rename`, `skip`, `default`, `with`); not on an `Option` |
| `with = path` (or `"path"`) | a module with `to_value_in(&T, Alloc) -> Result<Value, ValueError>`, `from_value(&Value) -> Result<T, MapError>` and `kind(Alloc) -> KindBuilder` converts the field in place of `T`'s impls; not on an `Option` |
| `skip_serializing_if = path` (or `"path"`) | the key is not written when `path(&field)` is true; needs `default` (or an `Option` field) so an absent key reads back |

On a struct, **`#[map(transparent)]`** makes its value its one field's
(other fields must be `#[map(skip)]`), and **`#[map(deny_unknown_fields)]`**
makes the reader refuse a key no field declares (`MapError::BadValue`
under that key), as the schema already does; not beside a `flatten`.

On the struct or enum, **`#[map(rename_all = "...")]`** names every field
of a struct, or every variant of an enum (its fields are left alone), by
one case rule, as serde spells and applies them: `"lowercase"`,
`"snake_case"`, `"kebab-case"`, `"SCREAMING_SNAKE_CASE"`. A field or
variant's own `rename` wins. All three derives and `#[derive(Form)]` read
it, so the value, the schema's `enum` and the form's keys agree; any
other rule is refused by name. A raw identifier is named without its
`r#`, as serde names it: a field `r#type` is keyed `type` by all four.

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
| `sensitive` | never render, never log, never quote in an error | `x-sensitive` and `writeOnly` |
| `default = <expr>` | the declared default, as an annotation: the field stays required and `FromValue` does not use it. `#[map(default = <expr>)]` is the one that decodes | `default` |
| `read_only` | shown, never edited | `readOnly` |
| `deprecated` | kept for old documents, not offered for new ones | `deprecated` |
| `examples(<expr>, ..)` | sample values, each through `ToValue` | `examples` |
| `min_length = N`, `max_length = N` | text length, in code points | `minLength`, `maxLength` (b) |
| `pattern = "..."` | a regex the text matches, unanchored | `pattern` (b) |
| `format = "..."` | what the text holds (`email`, `uri`, ..) | `format` (b) |
| `multiple_of = 5` / `0.25` | the step a number moves by; refused at compile time if not > 0 | `multipleOf` (b) |
| `min_items = N`, `max_items = N` | how many elements a list holds | `minItems`, `maxItems` (b) |

The right-hand column is what lands in the document, because **a schema IS
a JSON Schema** -- and each key is named after what it writes.

**(b) chains onto the KIND**, before it becomes a field: `<T as
Schema>::kind(alloc).min_length(2)`. A keyword for another type is carried
and ignored, as JSON Schema says -- the derive cannot see the kind a type
will describe, so `min_length` on an integer compiles.

**(a) is `guatiao-intake`'s vocabulary**, not `guatiao`'s: it names
`x-section`, `x-order` and `x-advanced`, because how to group and order
controls is an opinion the value crate does not hold. The expansion
writes them through `guatiao::schema::Extras`, the general door, with the
key **spelled as a literal** -- so a crate deriving a schema never
depends on the form crate. That literal is a seam;
`crates/guatiao-intake/tests/form_derive.rs` is what notices if the two
sides drift. The field's own name is its key in `properties`, and
whether it is required — a field that is neither an `Option<T>` nor
`#[map(default)]` — is a name in the struct's `required` list. Neither is written inside the field.

**`crate = <path>`** on the type, for a consumer that reaches `guatiao`
through a re-export and names no such dependency itself:

| on | written as |
| --- | --- |
| `ToValue`, `FromValue`, `Schema` | `#[map(crate = ::acme_kinds::guatiao)]` on the struct or enum |
| `#[derive(Provider)]` | `#[provider(Greeter, crate = ::acme_kinds::guatiao)]` |
| `#[kind]` | `#[acme_kinds::guatiao::kind(crate = ::acme_kinds::guatiao)]` |
| `#[derive(Form)]` | `#[form(crate = ::acme_kinds::guatiao_intake)]`: the path is `guatiao_intake`'s, and `guatiao`'s own items are reached through it |

Every path the expansion would root at `::guatiao` is rooted at the
path instead, so a crate that defines kinds can `pub use guatiao;` and
be the one thing its providers depend on -- one version to agree on
rather than two that must match. `providers!` needs nothing: it is
`macro_rules!` and already resolves through the path it was called by
(`acme_kinds::guatiao::providers!(Tuner);`). `#[derive(Form)]` roots
every path at `::guatiao_intake` (reaching `guatiao` through its hidden
`__private` re-export), so its `crate =` names that crate's re-export and
a consumer of the form derive needs no `guatiao` dependency of its own.

**Enums**, in three shapes. Refused otherwise, with a sentence saying why.

| declaration | value | `Schema::kind` |
| --- | --- | --- |
| unit variants only | the variant's name, as a string | `type: "string"` + `enum` + `x-enum-labels` |
| unit variants and one `#[map(other)]` | the name, or the text the catch-all holds | `type: "string"` + `examples` + `x-enum-labels`: any text is accepted, the known names are offered |
| `#[map(tag = "k")]` on the enum | a map: name under `k` first, then the variant's fields | `type: "object"` + `x-variant-tag` + `oneOf` |
| a tagged variant `Name(T)` | the name under the tag, then `T`'s map beside it (serde's internally tagged newtype) | the arm declares `T`'s fields. A `T` that writes no map is refused when written (`ValueError::WrongKind`) and its schema fails to build, as serde's does at run time |
| `#[map(untagged)]` on the enum | what the variant writes: a unit variant its name, named fields a map, `Name(T)` `T`'s value | `anyOf` of the arms: the unit variants one string `enum` + `x-enum-labels`, the record an object, `T`'s kind |

| on | attribute | effect |
| --- | --- | --- |
| enum | `#[map(tag = "...")]` | the key the variant's name is stored under; required when any variant has fields |
| enum | `#[map(rename_all = "...")]` | every variant's stored name by one case rule (`kebab-case`: `HighEfficiency` → `high-efficiency`); `rename` on a variant wins |
| enum | `#[map(untagged)]` | no name is stored: the variants are told apart by the kind of value each writes (below) |
| variant | `#[map(rename = "...")]` | the name stored instead of the variant's own |
| variant | `#[map(other)]` | **unit enums only**, on one variant of the shape `Other(T)` with `T: From<&str> + AsRef<str>` (`String`): a name no other variant is stored as is kept there rather than refused, and written back as that text |
| variant | `#[schema(title = "...")]` | an arm's title — **tagged enums only**: an arm is a subschema |
| variant | `#[schema(description = "...")]` | an arm's description — **tagged enums only** |
| variant | `#[schema(label = "...")]` | a choice's label — **unit enums only**: it lands in `x-enum-labels`, whose word is `label` |
| a variant's field | anything a struct field takes | means the same thing |

**A doc comment fills the most descriptive slot the thing has.** A field
or an arm has a description as well as a title, so its doc comment is the
description; a choice has only a label, so its doc comment is the label.

A catch-all is for a vocabulary shared across a boundary, where the two
sides are built at different times and a name one side does not know
must not fail the whole value. Text equal to a known name always reads
as that variant, so `Other("h264")` writes `h264` and reads back as the
variant stored as `h264`.

**`#[map(untagged)]`**: no two variants may read one kind. The unit
variants together read a string (a name none has is `BadValue`, "one of
.."), the one variant with named fields a map, and `Name(T)` the kinds
`<T as FromValue>::KINDS` states. A kind no variant reads is `BadValue`
naming the kinds ("a string or a map"). Fixed kinds are checked as the
enum is read (two variants with named fields are refused there); a
`Name(T)` is checked by the `FromValue` expansion in a `const` that names
both variants when it fails, an item `const` for a concrete `T` (so
`cargo check` reports it) and an inline one for a type parameter (so a
build of the instance that overlaps reports it). `ToValue` and `Schema`
alone do not check it. A unit variant takes `rename` and `label`, as in
a unit enum; another takes `title` and `description`, which land on its
arm (`KindBuilder::title`/`description`). Refused: `untagged` beside
`tag`, `other`, a tuple variant of two or more fields (hold a tuple:
`Name((A, B))`), `rename` on a variant that holds a value, two unit
variants stored under one name.

**Every derived `FromValue` states `KINDS`**: a struct with named fields,
a tagged enum and a record a map; a unit enum a string; a tuple struct a
list; a unit struct null; a newtype or transparent struct its field's;
an untagged enum the union of its arms'.

Refused, each with its own message: `other` on a tagged enum, on a
variant that is not `Name(T)`, beside `rename` or `label`, or on two
variants; an enum with no variants; a
data-carrying enum with no `tag`; a tuple variant of two or more fields;
an attribute on a newtype variant's field; two variants stored
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
`"session_backend"`). Emits the trait, with `<method>_in(.., alloc: Alloc)`
added as a provided method beside every method answering a `Value`, `Map`
or `List` (its default answers `<method>(..)`). The shim calls
`<method>_in` with the allocator the caller carried in: override it with
the `_in` forms (`Map::new_in`, `set_in`, `Text::new_in`, `clone_in`,
`to_value_in`) and implement `<method>` as `self.<method>_in(..,
Alloc::rust())`, and the answer crosses with no copy; leave it and the
proxy copies the answer once. Then:

| item | what |
| --- | --- |
| `<Trait>Vtable` | `repr(C)`: `header: KindHeader`, then one `Option<unsafe extern "C" fn>` slot per method, declaration order |
| `<Trait>Vtable::of::<T>()` | `const fn`, the table for an implementation; `floor()`; `<method>_end()` per slot; `FLOOR_HASH` as a **literal** the attribute computed, so a C header generator renders it as `#define <table>_FLOOR_HASH n` |
| `impl Kind for dyn Trait` | `NAME`, `Vtable`, `FLOOR`, `FLOOR_HASH` (FNV-1a over `provider;` or `object;` and the required signatures), `REQUIRED`, `OBJECT`, `as_dyn`/`as_dyn_mut` |
| `impl Trait for Remote<dyn Trait>` | the proxy: reads each slot under the table's size, marshals, calls, converts back |
| `impl From<Remote<dyn Trait>> for Box<dyn Trait>` | `Box` only: `Arc` is not fundamental (`Kind::shared`) |
| `impl Trait for Instance<dyn Trait>`, `impl From<Instance<dyn Trait>> for Box<dyn Trait>` | provider kinds only: an instance built from a configuration is the trait, so `Arc::new(instance)` coerces to `Arc<dyn Trait>` and `destroy` runs when the last owner goes |

Accepted method shapes — receiver `&self`, the trait names `Send + Sync`:

| position | crosses as |
| --- | --- |
| integers, floats, `bool` | by value |
| `&str` / `&[u8]` | `Str` / `Bytes` views |
| `&Value`, `&Map` | non-null pointers |
| `Option<&Value>` | a pointer that may be null |
| `Value`, `Map`, `List` by value | **a move**: `*mut Value`/`*mut Map`/`*mut List` (no `const`) to the caller's node, which the proxy then forgets and the callee owns on every path, taken first in the shim; nothing is copied. The tree is in the caller's allocators; `into_local()` re-homes one a callee keeps past them. A map's keys are checked as for `&Map` |
| `&mut Value`, `&mut Map`, `&mut List` | **lent for the call**: `*mut Value`/`*mut Map`/`*mut List` to the caller's node; the callee may change and grow it, never free it or keep it. The shim refuses null and a map's keys that are not UTF-8 (`kind::{value,map,list}_mut_arg`). The proxy reads the allocator the tree records before the call (`kind::lent_home`) and, after it, answered or refused, moves into it whatever the callee built elsewhere (`kind::lent_settle`, which is `make_local_in`, and removes a key a foreign slot wrote that is not UTF-8). The slot's field carries a doc line, "Lent for the call, not handed over", which a C header renders. Provider and object kinds alike; `Option<&mut T>` is refused |
| any other argument type, by value | `*const Value` via `ToValue` (the shim reads it with `FromValue`) |
| `Object<dyn K>` (`K` an object kind) | an `ObjectRaw` by value; **ownership crosses with the call**, the callee destroys it — taken first in the shim, before anything can fail |
| `&mut dyn K` / `&dyn K` (`K` an object kind) | **lent for the call**: `*const ObjectRaw` (`const guatiao_object *`) on the caller's stack, `ctx` the address of the caller's `*mut dyn K`, the table `K`'s lent table; the callee gets `BorrowedMut`/`Borrowed` (no `Drop`) and never calls `destroy`; nothing is allocated. `&dyn K` needs a `&self` method on `K`; lent shared, a `&mut self` slot answers `GUATIAO_ERR_UNSUPPORTED`. A provider kind, or `&dyn K` of a kind with no `&self` method, is a compile error with a sentence (a `const` assertion on the hidden `Lend::LENT_MUT`/`LENT`) |
| return `()`, scalar, `String` | an out-pointer (`String` as `Text`) |
| return `Value`, `Map`, `List` | an out-pointer the proxy fills first with a **carrier**: an empty container recording the allocator the answer is wanted in (ABSENT, its map arm recording it, for a `Value`). The shim reads it (`kind::return_alloc`) and calls `<method>_in` with it; a tree arriving in any other allocator is copied there by the proxy |
| any other return type | `*mut Value` via `ToValue` into the carrier's allocator, read back with `FromValue` |
| return `Object<dyn K>` | `out: *mut ObjectRaw`; the caller owns what it receives |
| `Result<X, ProviderError>` | `X` as above plus `err: *mut ProviderError`; **required** for any method that converts, passes an `Object` or lends one |

A method **with a default body** is an appended slot: an older table
lacks it and the proxy runs the default. A required method after a
defaulted one is refused. A method that cannot fail propagates a provider
failure as a panic. Refused by name, on the author's span: a generic or
`unsafe` trait, a `where` clause, missing `Send + Sync`, an empty trait,
associated consts/types, `async`, generic or variadic methods, `&mut
self`/`self`/no receiver, a `&mut` argument other than a lent tree
(`&mut Value`/`&mut Map`/`&mut List`), a lent object or `&mut [u8]` on
an object kind, `Option<&mut T>`, a
lent object with another bound beside the trait (`&mut (dyn K + Send)`),
a borrowed argument of any other type, a `Text` or `Buffer` by value as an argument, `impl Trait` anywhere,
a function argument, a borrowed or `impl Trait` return, a `Result` whose
error is not `ProviderError`, a method named `header` (or `destroy` or
`into_object` on an object kind), a method named `<m>_in` beside a method
`<m>` answering a tree, an argument whose name starts with
`__`, and an attribute key other than `name`, `object` and `crate`.

**Types are read by path, not by last segment.** guatiao's `Value`,
`Map`, `List`, `Text`, `ProviderError`, `Object`, `Status` are recognised
bare, under `guatiao::`/`::guatiao::` or under the `crate =` root; the
standard `String`, `Vec`, `Option`, `Result`, `Box` bare or under
`std::`/`core::`/`alloc::`. A path ending in `Value`, `Map`, `List`,
`String`, `Object` or `ProviderError` under any other root is
refused where it could only mean guatiao's or the standard type —
borrowed (`&foreign::Value`, `Option<&foreign::Value>`), returned
(`-> Result<model::String, _>`), as the error, or as
`other::Object<dyn K>` — with a sentence naming the accepted spellings,
rather than misread and failing inside the expansion; a type of your own
returned under such a name needs another in the signature (`use .. as
..`). **As an argument by value, a type of yours crosses through
`ToValue`/`FromValue` whatever its last name** (`proto::String`,
`json::Object`, `proto::Text`).

**Hygiene.** A shim's own parameters are `__ctx`, `__out` and `__err`,
so an argument may be named `ctx`, `out` or `err`; in the slot's type,
which a C header renders, such an argument is written `ctx_`, `out_`,
`err_`, so no prototype names a parameter twice. Every message is pinned by text in
`kind::tests::kind_rejects_by_name`; the expansion of a two-method trait
is pinned by `src/snapshots/greeter.expected.rs`.

**What the hash covers.** `FLOOR_HASH` is FNV-1a over `provider;` or
`object;` and each required method's `name(arg,..)->ret`, every type in a
canonical spelling with the spaces removed. The canonical spelling makes
these equal: a `std::`, `core::` or `alloc::` path of `Vec`, `String`,
`Option`, `Result` or `Box` and its bare name; a `guatiao::` or
`::guatiao::` path (or one under the `crate =` root) of `Value`, `Map`,
`List`, `Text`, `ProviderError`, `Object` or `Status` and its bare name;
a leading `::`; a reference's lifetime; parentheses. **Every other type
is hashed as written**: the macro sees tokens, not types, so
`Vec<Channel>` and `Vec<RenamedChannel>` hash differently though they
cross identically, and so do `Channel` and `model::Channel`. Renaming or
re-pathing a model type in a kind crate is therefore a new declaration of
the kind, and every built provider is refused with `HashMismatch` until
rebuilt; pin each kind's `FLOOR_HASH` in a test of the kind crate so
that is a decision, not a surprise.

**`#[kind(object)]` on a trait** — an **object kind**: a handle one
caller owns (a session, a scan, a stream), never offered by a registry.
The trait names `Send` (`Sync` optional); methods may take `&mut self`;
a `&mut [u8]` argument crosses as a `BytesMut` out-buffer (the one
`&mut` argument allowed, and only here). What differs in the expansion:

| item | what |
| --- | --- |
| `<Trait>Vtable` | `header`, then **`destroy: Option<unsafe extern "C" fn(ctx)>`**, then the slots; `destroy_end()`; `destroy` is the first entry of `REQUIRED` |
| `Kind::OBJECT` | `true`; the hash is over `object;` + the required signatures, so an object table never passes for a provider table with the same methods |
| the trait | gains `fn into_object(self) -> Object<dyn Trait> where Self: Sized + Send + 'static`: boxes the value beside a table `of::<Self>()` (`ObjectCell::new`) and hands back the handle |
| `TableFor` | a hidden `unsafe impl<T: Trait> guatiao::library::kind::TableFor<T> for <Trait>Vtable` (`Kind = dyn Trait`, `table() = of::<T>()`): what `ObjectCell::new` and the safe `Object::from_cell` require, so a cell's table is the one built for its value's type and kind |
| the shims | address the value through that cell: `&mut` for a `&mut self` method (`object_mut`), `&` for a `&self` one (`object_ref`), so an object of a `Sync` kind is shared between threads; `__guatiao_destroy::<T>` frees the cell |
| `lent_mut()`, `lent()` | the lent tables: slots `__guatiao_lentmut__<m>`/`__guatiao_lent__<m>` reach the lender's `*mut dyn Trait`/`*const dyn Trait` behind `ctx`, exclusively for a `&mut self` method and shared for a `&self` one; `destroy` is `__guatiao_lent_destroy` (does nothing); `lent()` exists only when a method takes `&self`, and its `&mut self` slots refuse. The hidden `unsafe impl guatiao::library::kind::Lend` names them (every kind gets one; a provider kind's names none) |

A Rust implementation becomes a handle with `value.into_object()`; the
receiving side drives it through `Object<dyn Trait>` (`Deref`/`DerefMut`
to the trait) and drops it, which calls `destroy` exactly once. A C
implementation fills the same table and hands `{table, size, ctx}` as a
`guatiao_object`.

**`#[derive(Provider)]`** with `#[provider(...)]`:

| key | effect | default |
| --- | --- | --- |
| a bare path, or `kinds(A, B)` | the kinds this type serves; a `T: Kind` bound is checked per kind | required, at least one |
| `id = "..."` | the provider id | `{CARGO_PKG_NAME}_{type}` in snake case, `-` as `_`; the package is the crate holding the derive, not the one calling `providers!` |
| `name = "..."` | display name | the type's ident |
| `version = "..."` | the provider's own version | empty: the library's |
| `config = C` | the provider is **built from `C`**: schema through `C: Schema`, decoded through `C: FromValue`, built through `Self: TryFrom<C, Error: Into<ProviderError>>` (`C = Self` is the identity); emits the `create`/`destroy` slots, so a host gets an `Instance` per configuration whose address is the `ctx` | none: the type is its one instance |
| `new = path` / `new_with_host = path` | `fn() -> Self` / `fn(Host) -> Self` building the default instance (the descriptor's `ctx`); with `config` and neither, there is no default instance | `Default`, except with `config` |
| `available = path` | `fn(&Self) -> Result<(), &'static str>`, asked on every call of the default instance; refused when there is none | always available |
| `form = Ty` | `Ty` derives `Form` (from `guatiao-intake`): its screen is built into the descriptor's `meta` under `form` (`guatiao_intake::PROVIDER_FORM_KEY`), so a host shows the configuration with its sections and hints; reached through the hidden `Ty::__guatiao_form` the Form derive writes, so the provider names no `guatiao_intake` path | no `meta` |
| `crate = path` | where the expansion finds `guatiao`, for a provider that reaches it through a re-export | `::guatiao` |

Emits, inside a `const _` block: one `static` table per kind
(`<dyn K as Kind>::Vtable::of::<T>()`, reached through the trait so only
the trait need be in scope), a `OnceLock<T>` holding the instance, and
`impl ProviderDecl for T` building a `ProviderParts`. Refused: a generic
type, no kinds, a kind named twice (by its spelling, a leading `::`
aside), an unknown key, a key given twice, both `new` and
`new_with_host`. `guatiao::providers!(A, B)` (in the core crate) then
writes the whole library.

## Contract

- **Every struct shape, unit enums and tagged enums**, as serde writes
  them: named fields are a map; a newtype struct (or a
  `#[map(transparent)]` one) is its field's value and describes its
  field's kind; a tuple struct is a list by position, described with
  `prefixItems`; a unit struct is null. A field of a tuple or newtype
  struct takes no `#[map]` or `#[schema]`: a position has no key.
- **Generic types**: each type parameter is bound by the trait derived
  (`T: ToValue`, `T: FromValue`, `T: Schema`), as serde bounds them;
  lifetimes and const parameters pass through. A borrowed field
  (`&'a str`) writes and describes itself; `FromValue` refuses it at the
  field (E0277), since nothing a reader returns can borrow from the value.
- **Not supported**, each refused with a sentence: a data-carrying enum
  with neither `tag` nor `untagged` (the key telling variants apart is a
  wire-format decision the author makes), or `untagged` with two
  variants of one kind; a tagged variant of two or more
  unnamed fields (no keys beside the tag); `alias` (a schema refuses
  every key it does not declare, so an alias would read a value its own
  schema refuses); a union. `#[derive(Form)]` places named fields only,
  so a tuple, newtype or unit struct has no form of its own.
- **Reading is lenient, validating is strict.** A generated reader ignores
  keys it does not declare, for a struct and for a tagged variant's arm
  alike; `schema::validate` is what refuses them.
- **`Option<T>` is omitted when `None`** rather than written as a stored
  null. Both spellings read back as `None`, so the two agree.
- **A defaulted field is always written**, and read as its default only
  when the key is absent: a stored null or a value of another kind is
  refused under the key, as for any field. Refused at compile time:
  `default` with `skip`, `default` on an `Option<T>` (an absent key is
  already `None`, and `None` is written as an absent key), and
  `#[map(default)]` beside `#[schema(default = ..)]`.
- **A key may contain a NUL.** Keys are pointer and length, so the derive
  does not invent a restriction the value model does not have.
- **Every path the expansion emits is rooted** at `::guatiao::` (or at
  the path `crate = ..` names instead), `::core::`, `::guatiao::__private::alloc::` for what `alloc` holds (the
  list a schema's fields are collected into), or `::std::` for what needs
  the operating system (`OnceLock`). A test asserts this token by
  token, over all three derives: generated code that named a third crate
  would fail in every consumer that expanded it while passing here.
- Errors are `compile_error!` at the offending span, never a panic.

## Gotcha

The expansion names `::guatiao::` paths, so the `guatiao` crate must keep
its root re-exports (`Value`, `Map`, `Alloc`, `ToValue`, `FromValue`,
`MapError`, `ValueError`) wherever the modules underneath move. Rearranging
`guatiao`'s internals is free; renaming a root re-export breaks every
already-expanded macro.
