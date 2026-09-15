# guatiao — API header

The public surface of the `guatiao` crate, so a consumer does not have to
read the source. Current with the crate it ships beside.

**What it is.** One value model for crossing a language boundary: plain
`repr(C)` structs and no opaque handles. Three layers, each usable
without the ones above it:

| layer | answers | module |
| --- | --- | --- |
| **Value** | what is being passed | `value` |
| **Schema** | what that value is, and what a valid one looks like | `schema` |
| **Library** | who is offering it, and how a host loads them | `library` |

A C, C++ or Dart consumer reads a whole tree through
`include/guatiao.h` with no call into any library.

## Features

| flag | adds | default |
| --- | --- | --- |
| `derive` | `#[derive(ToValue, FromValue, Schema)]` | off |
| `provider` | `#[guatiao::kind]`, `#[derive(Provider)]`, `guatiao::providers!` (implies `derive`; turns on `syn/full` in the derive crate) | off |
| `c-header` | regenerating the committed `include/guatiao.h` | off |

MSRV 1.89. No required dependencies; `derive` pulls `guatiao-derive`,
and the loader pulls `libloading`.

---

# Building a value

**Building in Rust names no allocator.** The short constructors use the
crate's own allocator and abort on allocation failure, exactly as
`String::from` does. The `_in` forms name one and stay fallible, because a
foreign allocator refusing is recoverable.

```rust
use guatiao::{Map, ReadValue};

let mut fields = Map::new();
fields.set("compression", 6)?;

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;
map.set("tls", true)?;
map.set("fields", fields)?;
```

`set` and `push` take `impl Into<Value>`, which is implemented for
`&str`, `String`, `&String`, `bool`, every integer width (`i8`..`i128`,
`u8`..`u128`, `isize`, `usize`), `Map`, `List`, `Text`, `Buffer`, and
`Option<T: Into<Value>>` (`None` stores a null). Floats convert through
`TryFrom<f64>`/`TryFrom<f32>`, because `NaN` and the infinities have no
representation here.

## Containers

`Map::new()` hands back a **`Map`**, not a value; `map.into()` makes it a
`Value` at the one point something wants one. All four containers free
what they own on drop.

```rust
Map::new() -> Map                      List::new() -> List
Map::new_in(alloc: Alloc) -> Map       List::new_in(alloc: Alloc) -> List
map.alloc() -> Result<Alloc, ValueError>
map.len() / map.is_empty() / map.entries() -> &[Entry]
map.set(key: &str, value: impl Into<Value>) -> Result<(), ValueError>
map.get(key) -> Option<&Value>         map.get_mut(key) -> Option<&mut Value>
map.contains_key(key) -> bool
map.remove(key) -> Option<Value>       // handed back, frees on drop
map.clear()

list.items() -> &[Value]               list.push(impl Into<Value>)
list.get(i) / list.get_mut(i)          list.remove(i) / list.pop()
list.clear()

Text::new(&str) -> Text                Buffer::new(&[u8]) -> Buffer
Text::new_in(alloc, &str) -> Result<Text, ValueError>
Buffer::new_in(alloc, &[u8]) -> Result<Buffer, ValueError>
text.as_str() -> Option<&str>          buffer.as_slice() -> &[u8]

Entry::key() -> &[u8]                  // bytes: a key may contain a NUL
Entry::key_str() -> Option<&str>       Entry::value() -> &Value
Entry::value_mut() -> &mut Value       // the key is not offered this way
Text::default() / Buffer::default()    // empty, allocating nothing
```

Every name above is at the **crate root**: `guatiao::Text`,
`guatiao::Entry`, `guatiao::Tag`, `guatiao::Str`, beside `Alloc`,
`List`, `Map`, `MAX_DEPTH`, `ReadValue`, `Status`, `Value` and
`ValueError`.

**`guatiao::Bytes` is the conversion marker**, not the borrowed view: it
is the field type that says "cross as the bytes kind". The borrowed view
of the same name stays at `guatiao::value::types::Bytes`, with
`Bytes::borrowed(&'static [u8])` and `Bytes::empty()` mirroring `Str`.

**Setting an existing key replaces it in place**, keeping its position.
Map order is insertion order and is part of the contract.

## Values

Infallible, on Rust's heap:

```rust
Value::null()      Value::absent()     Value::bool(b: bool)
Value::string(&str)                    Value::bytes(&[u8])
Value::int(i64)                        Value::map()    Value::list()
```

Fallible because the **argument** may be wrong, not the memory:

```rust
Value::number(text: &str) -> Result<Value, ValueError>   // JSON grammar
Value::float(v: f64)      -> Result<Value, ValueError>   // NaN/inf refused
```

Naming an allocator, all fallible: `Value::string_in`, `bytes_in`,
`number_in`, `int_in`, `float_in`; `Value::map_in(alloc)` and
`list_in(alloc)` are infallible (an empty container owns nothing).

`absent` is not `null`: absent is the answer to a lookup that found
nothing, null is a stored value.

**Nothing here stores absent in a list or a map, and nothing refuses one
that arrives.** It is a convention this crate keeps, not an invariant it
enforces: a foreign producer can put an absent node anywhere a value
goes, and every reader treats it as the ordinary kind it is.

## Ownership

**A `Value` owns its tree and frees it on drop.** Handing it to something
else is a move, which is when Rust stops dropping it.

```rust
let mut out = Value::absent();
let status = unsafe { guatiao_merge(..., &mut out, ...) };
// `out` now owns the result and frees when it goes out of scope.
```

**`value.alloc()` refuses two different things.** A scalar — null, bool,
absent — has no container to have recorded an allocator and answers
`WrongKind`. A container whose `alloc` field is null — a literal another
language wrote as a brace initialiser — answers `Alloc(AllocError::Null)`.
Growing either means naming one to adopt: `set_in`, `push_in`.

**`copy_from` is not atomic.** A failure at entry *k* leaves entries
`0..k` applied. Nothing leaks, and `src` may be this node or one inside
it: the source is copied whole before the target is touched.

**`Value`, `Map`, `List`, `Text` and `Buffer` are `Clone`, `PartialEq`,
`Send` and `Sync`.** `clone()` is a deep copy through the allocator the
source recorded (the crate's own for a scalar or a literal), and panics
where the short constructors do; `clone_in(alloc)` is the fallible form
that names one. Equality is structural, the same as `equal`. `Send` and
`Sync` rest on the allocator contract below: an `Allocator` may be
called from any thread.

**Crossing FFI**: a value handed to a foreign caller must be forgotten
(`std::mem::forget`, `ManuallyDrop`, or a move into `ptr::write`) or Drop
will free what the far side now owns. The far side frees with
`guatiao_value_free`, which runs the same walk.

The free walk is **iterative**, because a tree may have arrived from a
caller this crate cannot see and recursion on adversarial depth is an
uncatchable stack overflow on Windows. Every recursive read is bounded by
`MAX_DEPTH` (128).

---

# Reading a value

Get the value, then convert it. There are no per-kind getters on a map
and no per-kind readers on a value.

```rust
let host: &str = map.get("host").ok_or_missing()?.try_into()?;
let port: u16  = map.get("port").ok_or_missing()?.try_into()?;
let n: i64 = map.get("fields").get("compression").ok_or_missing()?.try_into()?;
```

`ReadValue` is implemented for `&Value` **and** for `Option<&Value>`, so a
path chains and a missing key does not need unwrapping at each step.
`ok_or_missing()` is `Option::ok_or` with the one error it could be.

`TryFrom<&Value>` exists for every scalar plus `&str`, `&[u8]`,
`&[Value]` and `&[Entry]`. Integer reads **never truncate**: a fractional
or exponent spelling is refused rather than rounded.

Inherent readers on `Value`:

```rust
v.tag() -> Result<Tag, ValueError>     // `into_raw_parts().0` is the raw u32
v.as_bool() / as_str() / as_bytes() / as_number_str()
v.as_map() / as_map_mut() / as_list() / as_list_mut()
v.entries() -> Option<&[Entry]>        v.items() -> Option<&[Value]>
v.get(key) / get_mut(key) / contains_key(key)
v.set(key, impl Into<Value>) / push(..) / push_into(key, ..)
v.remove(key) / discard(key) / remove_at(i) / discard_at(i) / clear()
v.into_map() -> Result<Map, Value>     v.into_list() -> Result<List, Value>   // by value; Err hands it back
```

Defaulting getters, mirroring the header's `static inline` helpers:
`bool_or`, `int_or`, `float_or`, `str_or`, `bytes_or`. Iterators:
`entries(v)`, `keys(v)`, `items(v)`. Structural comparison: `equal(a, b)`.
`Dump(&v)` is a bounded `Debug` tree for diagnostics.

**Numbers are the exact text that declared them.** `1.10` reads back as
`1.10`, a 200-digit integer survives, and `u64::MAX` crosses as itself.
Nothing coerces between kinds.

## Errors

```rust
ValueError = Alloc(AllocError) | NotANumber | NotUtf8 | WrongKind
           | UnknownTag(u32) | OutOfRange | TooDeep
MapError   = MissingKey | WrongType | BadValue     // carries the path
```

`MapError` builds its path on the way out: `.under(prefix)` and
`.at(index)`.

---

# Derives (feature `derive`)

```rust
#[derive(ToValue, FromValue, Schema, PartialEq, Debug)]
struct Connection {
    /// Where to connect.          // doc comment becomes the help text
    host: String,
    port: u16,
    #[map(rename = "view-only")]
    view_only: bool,
    #[schema(sensitive)]
    password: Option<String>,
}
```

`#[map(rename = "...")]`, `#[map(skip)]`; `#[schema(label, help, section,
order, advanced, sensitive, default)]`. Unions, tuple structs and generics
are refused with a message naming the derive you wrote.

Enums, in two shapes:

```rust
#[derive(ToValue, FromValue, Schema)]
enum Level { Off, #[map(rename = "warn")] Warning, On }   // "Off" | "warn" | "On"

#[derive(ToValue, FromValue, Schema)]
#[map(tag = "auth")]
enum Auth { Ambient, UserPass { username: String } }       // {"auth": "UserPass", "username": ...}
```

A unit enum is a string and describes itself as a choice; a tagged enum is
a map and describes itself as a variant. An enum whose variants carry
fields and names **no** tag is refused: the key that tells variants apart
is a wire-format decision, and it is yours. A doc comment on a choice is
its label; on an arm it is help, as on a field.

Generated readers go through `convert::{expect_map, expect_str, expect_key,
find_key}`; they are public so a hand-written impl reports errors the same
way.

An `Option<T>` field is omitted when `None` rather than written as null,
and both spellings read back as `None`.

Generated code names only `::guatiao::` paths, checked by a test.

---

# Schema

**A schema is an ordinary value, and that value IS a JSON Schema
(2020-12)**, so a consumer in any language reads one by walking a map --
and the keys it walks are ones its ecosystem probably already has a
library for. `schema::vocab` is the contract.

There is no serialisation here and no emitter: the value already IS the
document, so `guatiao-serde` writes it out in JSON, TOML or YAML knowing
nothing about schemas.

```text
{ "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "title": ..., "description": ...,
  "properties": { "<name>": <schema>, ... },
  "required": ["<name>", ...] }
```

Keys we added are `x-` prefixed, which is the space the specification
reserves for exactly that: `x-section`, `x-order`, `x-advanced`,
`x-sensitive`, `x-enum-labels`, `x-variant-tag`. Everything else is
JSON Schema's.

**Two things are not JSON Schema's, on purpose.** `type: "bytes"` extends
the type set, so a document using it is readable by anything and fails a
strict meta-schema check -- the data is unaffected. And `x-variant-tag`
names a variant's discriminant, because JSON Schema has no discriminator
keyword and inferring one stops working the moment two properties are
`const`.

**An undeclared key is refused, and the document says so.** Every object
the builders seal — a struct's document and each arm's subschema — carries
`additionalProperties: false`, so a general JSON Schema validator refuses
exactly what `validate_map` refuses. Refusing is deliberate: silently
dropping a misspelled field is how somebody ends up convinced a setting
does nothing.

`Schema::schema()` carries **every key of the finished kind** onto the
root document, not only `properties`/`required`: a tagged enum's
`schema()` is its `x-variant-tag` and its `oneOf`. `SchemaBuilder::finish`
refuses a field key containing `.` (`WrongKind`), which is
`flat::check_keys` run where nobody has to remember it.

Build:

```rust
use guatiao::schema::{FieldBuilder, FormBuilder, KindBuilder, SchemaBuilder};

SchemaBuilder::new()
    .label("Connection")                      // FormBuilder
    .field(FieldBuilder::new("port", KindBuilder::int_range(1, 65535))
        .label("Port").help("...")            // FormBuilder
        .required())                          // inherent: substance
    .finish() -> Result<Value, ValueError>
```

**A name lives in one place, and so does requiredness.** A field's name
is its key in `properties` and appears nowhere inside the field; whether
it is required is a name in the owner's `required` list and appears
nowhere inside the field either. That is why a builder **collects** its
fields and writes both at `finish` rather than appending as it goes, and
why a `FieldRef` carries its name and its requiredness alongside the
schema it views -- a bare pointer to a field's subschema cannot say what
it is called.

The same reason the C flat exports take `(schema, key)` rather than a
field pointer.

**A schema does not know about forms.** There is no way here to DECLARE
a section: a section exists only to group controls on a screen, so naming
one is a form's business. `FormBuilder::section` says which section a
field belongs to — a hint carried alongside the field — and what that
section is CALLED belongs to whatever draws it. A producer that must
write one uses `option`, the door every annotation goes through.

**A schema and a form are different questions.** Substance lives on the
builders; presentation lives on two traits in `schema::form`:

| trait | gives | implemented for |
| --- | --- | --- |
| `FormBuilder` | `label`→`title`, `help`→`description`, `section`→`x-section` | `SchemaBuilder`, `FieldBuilder`, `ArmBuilder` |
| `FormFieldBuilder` | `order`, `advanced`, `sensitive` (all `x-`) | `FieldBuilder` |

The Rust names stay `label`/`help`; the wire names are JSON Schema's.

Two, because a schema has no position among siblings and an arm is not a
secret — a single trait would hand out methods that mean nothing on two of
its three implementers. What stays inherent on `FieldBuilder` is the
substance: `required`, `default`, `extra`.

Import the trait to use its methods. **`#[derive(Schema)]` needs no
import**: it names them by rooted path, because a trait reached by method
syntax would have to be in scope at the expansion site.

Presentation is optional and substance is not: every presentation key may
be missing and the schema is still correct and still usable. Never make a
validation or type decision depend on one.

`FieldBuilder` rather than `OptionBuilder`, because the same builder
produces an entry in a schema's `properties` **and** in a map kind's.

**Building names no allocator**, the same rule the value API has. Every
constructor has an `_in` twin that takes one — `SchemaBuilder::new_in`,
`KindBuilder::int_range_in` — and that is what a schema built into a
host's arena uses. **Use them throughout when you use them at all**: a
sub-builder left on the plain form allocates through the crate's
allocator, and the tree then holds some of both. Sound, because every
container carries the allocator that made it, but not what somebody
building into an arena meant.

Kinds, with what each writes:

| builder | document |
| --- | --- |
| `bool` | `type: "boolean"` |
| `string` | `type: "string"` |
| `int`, `int_range`, `int_bounds` | `type: "integer"` + `minimum`/`maximum` |
| `float`, `float_bounds` | `type: "number"` + `minimum`/`maximum` |
| `bytes` | `type: "bytes"` (ours) |
| `list(items)` | `type: "array"` + `items` |
| `map(fields)` | `type: "object"` + `properties`/`required` |
| `enumeration(choices)` | `type: "string"` + `enum` + `x-enum-labels` (a map, keyed by value) |
| `union(arms)` | `anyOf` — **any** arm accepting is enough, and no `type` |
| `variant(tag, arms)` | `type: "object"` + `x-variant-tag` + `oneOf`, each arm pinning the tag with `const` and requiring it |

`anyOf` for a union and `oneOf` for a variant is not cosmetic: a union
asks only whether the value is acceptable, so `oneOf` would reject a value
two arms both accept.

Read (borrowed views over the value, no copying):

```rust
SchemaRef::new(&value) -> Option<SchemaRef>
  .dialect() .label() .help() .fields() .find(key) .extra(key) .extras()
  .as_value()
FieldRef::new(key, &schema) -> Option<FieldRef>   // answers is_required() false
FieldRef: .key() .kind() .label() .help() .section() .default() .order()
           .is_advanced() .is_sensitive() .is_required() .extra(key) .extras()
Kind: .choices() .alternatives() .arms() .items() .fields() .name()
```

Validate:

```rust
validate_map(schema, &values)   -> Result<(), ValidationError>
validate_value(field, &value)  -> Result<(), ValidationError>
validate_text(field, text)     // for a string-typed front end
validate_texts(schema, &BTreeMap<String, String>)
```

`ValidationError` is `UnknownOption { .. }` or `BadValue { .. }`. **An
error never quotes the value it refused** — a field may be sensitive —
it says what would have been accepted.

**Validation is depth-bounded** by `MAX_DEPTH`, like every other walk
here: it runs over two trees a caller supplied, and either nested past
the bound is a `BadValue` naming the key rather than a stack overflow.

Flat projection, for a front end that only has `key -> text`:
`flatten`, `unflatten`, `keys`, `resolve`, `resolve_in`, `is_sensitive`,
`check_keys`, separator `.`. `resolve` answers for the schema alone;
`resolve_in(schema, key, selected)` answers for a store, checking a
payload key against the arm the store selects (or the field's default),
and `validate_texts` goes through it -- so `auth=sso&auth.username=x` is
refused in the text form the way it is in the nested one.

---

# Layering

```rust
MergeMode::{Simple, Deep, Substitute}
  .merge(earlier, later, alloc) -> Result<Value, MergeError>
  .merge_with(earlier, later, alloc, fields, overrides)
  .merge_layers([("system", &a), ("user", &b)], alloc)
      -> Result<(Value, Provenance), MergeError>
```

- **Simple** — scalars and maps replace, lists overwrite positionally.
- **Substitute** — recursive maps, wholesale list replacement.
- **Deep** — recursive maps, lists union.

`MergeOptions::new().with_mergelists(true)` merges map elements of a list
by position; **off by default**. `MergeOverrides` is a `path -> mode`
lookup for per-key control, and `schema::merge` reads the same thing from
a schema's own `x-merge` annotation (`merge_overrides`, `merge_options`,
`merge_with_schema`). **Nested objects are walked**: a declaration on a
field of a nested object governs its dotted path (`tls.ciphers`), which
is the path the merge matches. A variant's arms are not walked — two arms
may declare different modes for one path.

**Provenance is keyed by leaf path**, not by top-level key: after a
recursive merge `tls.ca` and `tls.verify` may come from different layers,
so `source_of("tls")` answers `Source::Mixed` rather than a confident lie.

Merging **builds a new tree** and borrows its inputs. A value whose tag
this build does not know stops the merge rather than being copied, because
copying it would make two owners of whatever it points at.

---

# Library envelope

A library exports **exactly one symbol**. Everything else it offers is
reached through the descriptor it returns.

```rust
fn describe(host: &HostInfo) -> Option<&'static LibraryInfo> { ... }
guatiao::guatiao_library!(describe);   // emits `guatiao_library_entry`
```

`LibraryInfo { struct_size, abi_version, id, version, providers, meta }`;
`ProviderInfo { struct_size, vtable_size, kinds, id, display_name, config,
vtable, ctx, meta, version, available, tables, create, destroy }`;
`HostInfo { struct_size,
abi_version, host_id, host_version, alloc, meta, services }`;
`HostServices { struct_size, ctx, get, list, alloc }`. `ABI_VERSION` is 1.

`Providers { ptr, len, stride }` and `Kinds { ptr, len }` — the provider
array states its stride and the kind array does not, because a `Str`
declares no `struct_size` and so has no way to grow. Build them with
`Providers::new(&SLICE)` / `Kinds::new(&SLICE)` rather than by hand.

`meta` is a `MaybeNull<Map>` on all three — an open-ended map for
whatever the envelope did not think of, and the escape hatch that keeps a
vendor's own key out of the fields everyone shares. A reader that does not
know a key skips it, the rule the value model already has for a tag it
does not know. Null and an empty map mean the same thing.

**Non-null means a well-formed map**, and nothing checks it: the same
class of promise as `vtable`, whose shape only the `kind` knows. It is a
pointer rather than an inline `Map` because these descriptors are `Copy`
and every reader projects their fields with a bitwise read — an inline
owned container would make a second owner of one buffer on every read.

`MaybeNull<T>` is `#[repr(transparent)]` over `*const T`, so C sees a
plain pointer (`guatiao_map_ptr` in the header); `unsafe fn get()` is the
read, and `is_null()` needs no contract.

Host side:

```rust
let mut reg = Registry::new("my-host", "1.0");
reg.load_file(&path)?;                     // Result<Loading, LoadError>
reg.register_local("engine", crate::library)?;   // a library the host LINKS: the same absorb, keys,
                                                 // dedup and `Loaded` record, from `<engine>` instead
                                                 // of a file; `library` is what `local_providers!`
                                                 // writes (or a hand-written `describe`)
reg.providers("greeter")                   // by kind, BEST FIRST
reg.available("greeter")                   // the same, that can run here
reg.best("greeter")                        // the head of that
reg.set_priority(id, 10) / reg.priority(id)
reg.provider("acme_net_pve")               // by key: Option<&Provider>
reg.providers_of("acme_net_pve")           // every loaded version of one id (>1 only under %id@%version)
reg.all() / reg.all_ranked()               // everything, load order / best first
provider.kinds() / provider.supports(kind) // what it serves
provider.config_schema()                   // Option<&'static Value>
provider.vtable() -> (*const c_void, usize)
provider.view().vtable_as::<T>()           // unsafe; checks vtable_size >= size_of::<T>()
provider.meta()                            // Option<&'static Map>
loaded.meta                                // Option<&'static Map>
```

### A library is a collection; a provider is a thing in it

- **A library loads once.** By canonical path first (which costs no
  `dlopen`), then by its rendered library key. A repeat is
  `Loading::Skipped(Skipped::AlreadyLoaded { from })` **naming where it
  came from** — not an error. A host's search path and the directory
  beside its executable are routinely the same place, so this is the
  common path, and refusing it silently disables whatever the first load
  had not reached.
- **A provider id is globally meaningful**, conventionally
  `{library id}_{name}`. Two libraries may offer one provider — a
  re-export, a vendored copy, a second build — and they agree on its id,
  which is what makes it detectable. **The rendered key decides**: when
  the key a provider renders is already filed and the holder has the same
  id, the newcomer is `Skipped::ProviderAlreadyLoaded { id, from }` and
  **the rest of that library goes on loading**; so `%id` keeps one build
  of a provider and `%id@%version` keeps every build. Same key, different
  id is `LoadError::Duplicate`.
- **A file can come to nothing five ways, each reported as itself:**
  `Skipped::NoEntrySymbol` (not a library), `Skipped::Filtered { by }`
  (a scan rule kept it out by what it declares, before it was mapped),
  `Skipped::DeclinedThisHost` (its entry point answered null),
  `Skipped::UnsupportedAbi { declared }` (it speaks another envelope
  version; the host's `abi_version` is checked by the library, the
  library's by the loader), and `LoadError::Malformed` (a descriptor this
  build cannot read: below the floor, non-UTF-8 text, a stride below the
  floor, an element overlapping its neighbour).
- **A library's entry point must not call back into the registry loading
  it.** `load_file` holds the registry exclusively for the whole call; a
  provider that needs a peer looks it up later, from a vtable call.
- **A provider serves many kinds.** `kinds` is a list; `providers(kind)`
  filters on `supports(kind)`. One implementation that both discovers
  hosts and opens sessions to them is one provider answering to both, not
  two registrations a host has to know are the same. An empty list is a
  provider reached by name rather than by capability.
- **A provider carries its own version**, empty meaning its library's —
  which is the common case, because a provider shipped in its own library
  moves with it.
- `LoadError` is then only `Open`, `Malformed`, and `Duplicate` for two
  **different** providers landing on one key, which only a host's own
  template can produce.

### A library can reach its host

The entry point receives a `Host` (`Copy + Send + Sync + 'static`), a
pointer to a block the registry leaks on first use and never frees. Keep
it in a `OnceLock` of your own.

```rust
fn describe(host: Host) -> Option<&'static LibraryInfo> { .. }
host.id() / host.version() / host.abi_version()
host.alloc() -> Option<Alloc>                // the host's, or None
host.meta() -> Option<&'static Map>
host.get(key) -> Result<Option<&'static ProviderInfo>, Status>
host.list(kind) -> Result<Vec<&'static ProviderInfo>, Status>  // "" lists all
provider_info.view() -> Option<ProviderView>  // then .vtable_as::<T>(), .id, .ctx
host.snapshot() -> HostInfo                   // a copy, absent fields nulled
```

- **Answers are pointers to other libraries' own descriptors**, which
  live for the process. Nothing borrowed from the registry escapes.
- `list` returns **every** provider claiming the kind, unavailable ones
  included, in the host's order. **There is no best-pick**: ask each
  (`view().available()`) and choose.
- `Err(GUATIAO_ERR_NULL)`: the host offers no services (older host, or a
  host that passed none). `Err(GUATIAO_ERR_GONE)`: the registry was
  dropped; the block is still readable, the lookups are not.
- A lookup from `describe` sees the libraries registered before this one.
  A provider that needs a peer looks it up from a vtable call, on every
  call, never at `describe`.
- The C side: `guatiao_registry_host(reg)` hands a host the same block,
  to pass to a `guatiao_library_entry` it drives itself; a library reads
  `host->services->list(ctx, kind, out, cap, &total)` (call with `cap = 0`
  to size), `->get(ctx, key, &out)` and `->alloc(ctx)`, guarded by
  `host->struct_size >= offsetof(services) + sizeof` and
  `services->struct_size`.

### A kind is a trait (feature `provider`)

Declare a kind once, as a trait; the host and every library compile
against it. The table, the shims and the proxy are generated, and every
`unsafe` step in them is a call into `guatiao::library::kind`.

```rust
#[guatiao::kind]                       // name defaults to "greeter"; #[kind(name = "..")]
pub trait Greeter: Send + Sync {
    fn greet(&self, name: &str) -> Result<Map, ProviderError>;   // required
    fn shout(&self, name: &str) -> String { .. }                  // default body = appended slot
}

#[derive(Default, Provider)]
#[provider(Greeter, Counter)]          // long form below
struct Hello;
impl Greeter for Hello { .. }
guatiao::providers!(Hello);              // id and version from Cargo; the whole library
guatiao::local_providers!(Hello);        // the same, for a library the HOST LINKS: writes `library`,
                                         // no entry symbol, no declaration -- see register_local

// Consuming, from a host or from a library through its Host:
for offer in registry.offers::<dyn Greeter>() { offer.id(); offer.available(); offer.greet("x")?; }
registry.offer::<dyn Greeter>("acme_hello")    // Option<Result<Offer, KindMismatch>>
registry.mismatches::<dyn Greeter>()           // (&Provider, KindMismatch): tables that failed
provider.as_kind::<dyn Greeter>()              // Result<Remote<dyn Greeter>, KindMismatch>
host.offers::<dyn Greeter>() / host.offer(key) / host.mismatches()
unsafe { Remote::<dyn Greeter>::from_raw(table, size, ctx) }   // a table from anywhere else

// Building a configured provider, from the host's own type:
#[derive(ToValue, Schema)] struct Settings { prefix: String }
let config = Settings { prefix: "hey".into() }.to_value(Alloc::rust())?;
validate_map(SchemaRef::new(offer.config_schema()?)?, &config)?;   // the provider's schema
let instance = offer.instantiate(&config)?;   // derefs to dyn Greeter; drop runs `destroy`
```

The whole host side, as a program that scans a search path and does the
above against a library on disk, is `examples/greeter_host` in the
repository (`cargo run -p greeter_host` after a workspace build).

- **Object kinds: a handle one caller owns.** `#[guatiao::kind(object)]`
  on a trait naming `Send` declares what a provider hands BACK (a
  session, a scan, a stream) or a host hands IN (a callback): never
  offered by a registry, driven through `&mut self`, destroyed by
  whoever holds it last. The table carries `destroy` right after its
  header; `&mut [u8]` crosses as an out-buffer (`BytesMut`, object kinds
  only). A Rust implementation becomes one with `value.into_object()`
  (the attribute appends that method); the receiver holds
  `Object<dyn K>` — `Deref`/`DerefMut` to the trait, `destroy` on drop,
  `into_raw()`/`unsafe from_raw(ObjectRaw)` for a C caller
  (`guatiao_object { table, size, ctx }`). Any kind's method may return
  `Object<dyn K>` or take one; both need `Result<_, ProviderError>`
  (validation can fail), and **ownership crosses with the call**: the
  callee destroys an argument it was handed, even when it refuses the
  call, and the caller owns a return. `Kind::OBJECT` says which a kind
  is; an object table's hash covers `object;` so it never passes for a
  provider table.

- **A kind is a C ABI, and its table renders to C.** cbindgen cannot see
  a macro-generated struct unless it expands macros, so a kind crate
  renders its own header with three settings (the pattern is
  `examples/greeter_kind/{build.rs,cbindgen.toml}`, header
  `include/greeter_kind.h`): `[parse.expand] crates = ["<kind crate>"]`
  with `RUSTC_BOOTSTRAP=1` and `CARGO_EXPAND_TARGET_DIR` set by the
  build script around the render (the stable compiler refuses
  `-Zunpretty=expanded` otherwise; the expansion re-enters cargo and
  needs its own target directory); `[export] include` naming every
  table, since a kind crate exports no function that references one;
  `parse_deps = false` plus an `[export.rename]` table mapping this
  crate's type names onto `guatiao.h`'s (`KindHeader` →
  `guatiao_kind_header`, `Str` → `guatiao_str`, …), so the tables are
  declared against the header that already defines those types. Each
  table carries `<Trait>Vtable::FLOOR_HASH` as a **literal** (computed
  by the attribute at expansion time), which renders as
  `#define <table>_FLOOR_HASH n` — the number a C implementation writes
  into its table's `header.floor_hash`, beside `sizeof` the table as
  `struct_size`. A C library then offers its table through a
  `guatiao_provider_info` from its own `guatiao_library_entry`, exports
  `guatiao_declares`, and links nothing; the registry validates it and a
  Rust host calls it through the same proxy a Rust table is called
  through. `tests/c_consumer/greeter_in_c.c` is one, compiled and
  loaded by `tests/c_library.rs`.

```rust
#[guatiao::kind(object)]
pub trait Session: Send { fn poll(&mut self) -> Result<(), ProviderError>; fn read(&mut self, dst: &mut [u8]) -> i64; }
fn open(&self, uri: &Value, observer: Object<dyn Observer>) -> Result<Object<dyn Session>, ProviderError>;  // on a provider kind
let mut s = backend.open(&uri, MyObserver { .. }.into_object())?;   // the host; the library now owns the observer
s.poll()?; let n = s.read(&mut buf);                                 // &mut through the handle
drop(s);                                                             // the library's destroy runs, and drops the observer -- the host's destroy
```

- **What may cross.** Receiver `&self`; the trait names `Send + Sync`.
  Arguments: integers, floats, `bool`, `&str`, `&[u8]`, `&Value`,
  `Option<&Value>`, `&Map`, any other type by value through `ToValue`.
  Returns: `()`, the scalars, `Value`, `Map`, `List`, `String`, any other
  type through `FromValue`; each optionally in `Result<_, ProviderError>`.
  A method that converts must return `Result`. Refused by name: generics,
  `async`, `&mut self`/`self`, borrowed returns, `impl Trait`, closures,
  associated items, a required method after a defaulted one.
- **Versioning.** Slots follow declaration order; a defaulted method is an
  appended slot an older table may lack (the proxy runs the default). The
  table header `KindHeader { struct_size, floor_hash }` carries FNV-1a
  over the required signatures; a mismatch is refused, never called.
- **Only a per-kind table** (`ProviderInfo::tables`) is validated as a
  kind. A hand-written `vtable` is neither an offer nor a mismatch; it
  keeps working through `provider.table_for(kind)` / `vtable_as`.
- **`Offer<K>`** derefs to `K` and carries `id`, `display_name`,
  `version`, `library`, `key`, `priority`, `meta`, `config_schema`,
  `available()` (asked live), `remote()`, `boxed()`, `shared()`.
  Unavailable providers ARE offered; the consumer chooses. `Remote<K>` is
  `Copy + Send + Sync + 'static`; `Box<dyn K>: From<Remote<dyn K>>`.
- **`KindMismatch`**: `NoTable`, `BelowFloor { size, floor }`,
  `HashMismatch { expected, found }`, `NullRequiredSlot(name)`. A
  `floor_hash` of `0` passes only through `from_raw`.
- **`ProviderError { status, message: Text }`** is the one error type
  both sides share: `From<Status>`, `From<ValueError>`, `message()`.
- **`#[provider(..)]` long form**: `kinds(A, B)`, `id = ".."` (default
  `{package}_{type}` snake case), `name = ".."` (default the type),
  `version = ".."` (default empty: the library's), `config = C` or bare
  `config` (below),
  `new = path` (`fn() -> Self`) or `new_with_host = path`
  (`fn(Host) -> Self`; default `Default`), `available = path` (`fn(&Self)
  -> Result<(), &'static str>`). `providers!(id = .., version = ..,
  providers = [A, B], declares = [".."])` is the long form of the library
  line; `providers!(A, B; declares = ["VIEWER=1"])` the short one
  with extra declarations. The library declares every kind its providers
  serve (`ProviderDecl::KINDS`) for a scanner to read before mapping it;
  `guatiao::declares!(..)` does the same for a hand-written library.
- **Instances from a configuration.** `config = C` means the provider is
  **built from `C`**: `C: Schema + FromValue`, `Self: TryFrom<C, Error:
  Into<ProviderError>>`. Bare `config`, or `config = Self`, makes the
  type **its own configuration**: derive `Schema` and `FromValue` on it
  and there is no second type and no `TryFrom`. The host reads the
  schema (`offer.config_schema()`), fills it in, and calls
  `offer.instantiate(&config) -> Result<Instance<dyn K>, ProviderError>`;
  the `Instance` derefs to the trait, its address is the `ctx` every call
  on it takes (what `&self` is in the impl), and dropping it runs the
  provider's `destroy`. Many instances per provider, each its own
  configuration. With `config` and no `new`/`new_with_host` there is no
  default instance (`ctx` null) and `available` is refused. A provider
  without `config` is its one instance and `instantiate` answers
  `GUATIAO_ERR_NULL`; `offer.builds_instances()` says which. The envelope
  slots are `ProviderInfo::create` / `destroy`; from C,
  `guatiao_registry_provider_create(reg, key, config, &instance, &err)`
  and `_destroy(reg, key, instance)`, the instance being the `ctx` for
  that provider's tables.
- A provider that cannot build its config schema makes the library
  decline the host.

### Ordering is `(priority DESC, key ASC)`

`providers`, `available` and `providers_of` all answer **best first**.

**Priority is the HOST's**, kept beside the registry rather than in a
descriptor: a library does not know how a person ranks it against the
others they installed, and two machines with the same libraries can rank
them differently. A frontend reads its own configuration and calls
`set_priority`; nothing in this crate reads a file.

Absent means 0, so an unranked provider sorts below any raised one and
alongside every other unranked one; negative sorts below them all. A rank
set **before** anything loads still applies, so the order a host ranks and
scans in does not change the answer.

The **key** tiebreak is deliberate. Load order follows directory
iteration, which no filesystem promises to keep stable across runs or
machines, so "whichever loaded first" is not a rule anyone can document or
reproduce. A key is arbitrary but deterministic and inspectable, and a
host that wants a different winner says so with a priority.

`best(kind)` is the head of what can actually **run**, which is a
different question from what ranks highest: a provider that ranks first
and refuses is passed over. Iterate `available` instead to show what was
passed over — "ssh → openssh (also: putty)".

In C: `guatiao_registry_set_priority`, `_priority`, `_best`; and every
provider map carries its `priority`.

### Can it actually run here?

`ProviderInfo::available` is an optional slot: `true` for yes, or `false`
with a borrowed reason written through an out-parameter. **Null means
yes** — a library that does not implement it is available, which is the
common case and the right default.

```rust
provider.available()            // Result<(), &'static str>
reg.providers("codec")          // who CLAIMS the kind
reg.available("codec")          // who can serve it now
reg.why_not("codec")            // Option<WhyNot>: NothingClaimsIt | NoneAvailable
```

**The envelope defines the slot, not the meaning.** What "available" means
and when to ask are between a host and a library, like everything a `kind`
agrees. The one promise this crate makes is that it **never caches the
answer**: a library may load an optional dependency, lose a device, or
fail its own integrity check while a process runs.

**The reason must outlive every reader** — a literal in the image, or
something leaked once. Never a buffer shared between callers: two threads
asking the same provider would each read what the other just wrote, and
what comes back is a fragment with nothing reporting it.

`why_not` keeps two refusals apart **because the remedies differ**:
nothing claims it (install something) versus everything claiming it is
unavailable here (fix what you have), the latter carrying each provider's
own words. One "unsupported" leaves a person with no idea which way to go.

In C: `guatiao_registry_available`, `guatiao_registry_why_not`, and
`guatiao_registry_provider_available(reg, key, &reason)`.

### What things are filed under is the host's choice

```rust
let reg = Registry::new("my-host", "1.0")
    .libraries_keyed_by("%id@%version")?    // hold two builds of a library
    .keyed_by("%id@%version")?;             // and tell their providers apart
reg.provider("acme_net_pve@1.2.0")
provider.key()  /  loaded.key               // what each answers to
```

`KeyTemplate` fields: a **library** has `%id` and `%version`; a
**provider** adds `%name` (display name) and `%library`. `%%` is a literal
`%`, everything else is text. Naming a field the subject does not have is
refused where it is written, as is a `%` beginning nothing and a template
naming **no** field (which would give everything one key). Both `keyed_by`
methods re-key what is already loaded and refuse a template that would
collide there (`KeyError::Collides`).

The library key is the "how many builds may I hold" knob. With `%id` and
`scan_dir`, **which** build survives is the scan's order —
`scan_dir_ordered(reg, dir, Order::Descending)` offers the
highest-sorting name first. That is byte order, not version order:
nothing here parses a version, because ordering one is a host's policy
with the semver library it already has.

**A library declares what it is, and a scan reads that before mapping
it.** The data symbol `guatiao_declares` (`DECLARES_SYMBOL`) holds
`key=value` strings, NUL-separated, ended by an empty string: `kind=<name>`
for every kind a provider serves, written by `providers!`, plus whatever
`guatiao::declares!("kind=greeter", "VIEWER=1")` or `providers!(A,
B; declares = ["VIEWER=1"])` adds. `probe(path) -> Probe { entry,
declared: Declared }` reads it as data (clamped to its section and 4 KiB;
malformed is `NotExaminable`); `declared.has(k, v)`, `values(k)`,
`kinds()`. A host filters with
`scan_dir_rules(reg, dir, order, &ScanRules::parse(&["!VIEWER=1",
"kind=session-backend"])?)` — `!KEY=VALUE` skips a file declaring the
pair, `KEY=VALUE` skips one that does not, a file declaring nothing
passes every `!` rule and fails every positive one — or with
`scan_dir_with(reg, dir, order, |declared| Ok(()) | Err(why))`. Either
reports the file as `Skipped::Filtered { by }` and never maps it. That is
the replacement for a filename denylist: the library says
`VIEWER=1`, the host writes one rule.

### Where a host looks is a search path

```rust
let path = SearchPath::parse(&std::env::var("MY_HOST_PLUGIN_PATH").unwrap_or_default())
    .with(exe_dir.join("plugins"));            // adjacency accumulates, never replaced
let report = scan_path(&mut reg, &path, Order::Ascending, &rules);
report.loaded / report.skipped / report.failed / report.unreadable
```

- **A list of directories or files**, split on the platform's own
  separator (`SearchPath::SEPARATOR`: `;` on Windows, `:` elsewhere),
  empty parts dropped, **each place once** — by canonical path when it
  exists, as written when it does not — so an override that names the
  directory adjacency already found is one entry.
- A directory is scanned as `scan_dir_rules` scans it. A **file** is
  probed and loaded on its own, under the same rules: naming it is
  asking for it, so its extension is not checked, but what it declares
  still is.
- A **bundle** is its binary: `Name.framework` resolves to `Name` inside
  it, flat (iOS) or under `Versions/Current` (macOS). `bundle_binary` is
  pure path logic and answers on every platform; a scan of a directory
  offers a bundle as a candidate too.
- An entry that **cannot be read** — missing, or not listable — is
  `LoadReport::unreadable` and the rest of the path is still walked.
  Reported rather than dropped for the same reason a skip is: a person
  looking for a plugin that did not appear needs to see the place was
  considered. A single-directory scan still answers that with its `Err`.
- A library reachable twice loads once; the second is
  `Skipped::AlreadyLoaded { from }` naming the first.

In C: `guatiao_registry_scan_path(reg, spec, descending, rules, alloc,
&answer)`, whose report gains `"unreadable": [{"path", "error"}…]` only
when there is one.

### Notices ride on `meta`

```rust
use guatiao::library::{Notice, declare_notice, notices};
declare_notice(&mut meta, Notice::text("openh264", "OpenH264 Video Codec provided by ..."))?;
declare_notice(&mut meta, Notice::markdown("", "# Licence\n..."))?;
for n in notices(provider.meta()) { n.component; n.text; n.is_markdown(); }
```

`meta["notices"]` is a list of `{component, text, format}` maps, one per
licence. **A convention, not a field**: a notice is text a host must be
able to show — some licences require the attribution to be displayed —
and only the library knows what it links, so it belongs on the escape
hatch every descriptor already has, under a key named once here rather
than invented per host.

- `component` says what the licence covers; empty is the whole library
  or provider, and a host shows the display name.
- `text` blank contributes nothing: refused on write, dropped on read.
  Never trimmed — leading indentation is part of a licence.
- `format` is **declared, never sniffed**: `text` (the default) or
  `markdown`. Anything else reads as text, the safe direction; a prose
  licence rendered as Markdown is a licence that was not reproduced.
- **Read on display, never cached.** A library that loads an optional
  component on demand owes its attribution only while it is in use.

**A loaded library is never unloaded.** Everything it hands over —
strings, schemas, vtables — points into its mapping, so unloading would
dangle every borrow the host holds. That is also why a `ProviderView`
holds `&'static str` rather than `String`: the text is already in the
image, and copying it would be a waste and would stop a C accessor
handing back the library's own pointer.

### A host in C, or Python, or anything with an FFI

The whole registry is `extern "C"`, so Rust is one user and not the
audience:

```c
guatiao_alloc alloc = GUATIAO_ALLOC_MALLOC;
guatiao_registry *reg = guatiao_registry_new(guatiao_cstr("my-host"),
                                             guatiao_cstr("1.0"), &alloc);
guatiao_value answer;
guatiao_registry_load_file(reg, guatiao_cstr(path), &alloc, &answer);
guatiao_registry_scan_dir(reg, dir, /*descending=*/false, &alloc, &answer);
guatiao_registry_scan_dir_rules(reg, dir, false,
                                guatiao_cstr("!VIEWER=1\nkind=session-backend"),
                                &alloc, &answer);       // rules, one per line
guatiao_registry_providers(reg, guatiao_cstr("greeter"), &alloc, &answer);
guatiao_registry_provider(reg, key, &alloc, &answer);
guatiao_registry_keyed_by(reg, guatiao_cstr("%id@%version"));
guatiao_registry_libraries_keyed_by(reg, tmpl);
guatiao_registry_libraries(reg, &alloc, &answer);
const void *vt = guatiao_registry_provider_vtable(reg, key, &size);
void *ctx = guatiao_registry_provider_ctx(reg, key);
const guatiao_value *schema = guatiao_registry_provider_config(reg, key);
guatiao_registry_free(reg);
```

`guatiao_registry` is the **one opaque handle** in this crate: it owns
growable collections and changes over time, which is what a handle is for
and a `repr(C)` struct is not.

**Answers come back as values**, so there is no accessor per field and a
language that can already read a value can already read the answer. A
provider map is `{key, id, version, library, display_name, from, kinds,
has_config, vtable_size}`; a library map is `{key, id, version, path,
providers, skipped}`. Free each answer with `guatiao_value_free`.

**The exceptions are what is not data**: the vtable, its size, the `ctx`,
and the borrowed config schema have their own accessors. A pointer inside
a map would be a number a caller has to cast back, and reading one is the
moment a caller takes on the kind's contract.

**A skip is an answer, not a failure.** `load_file` writes `{"loaded":…}`,
`{"skipped":"<why>","from":…,"abi":…}` or `{"failed":"<the loader's own
message>"}` and returns `GUATIAO_OK` for all three; a non-OK status means
it could not answer at all. `why` is one of `no-entry-symbol`,
`declined-this-host`, `unsupported-abi` (with `abi`), `already-loaded`
(with `from`), `provider-already-loaded` (with `from` and `id`), or in a
scan report `not-examinable` and `filtered` (with `by`, the rule). A rule
that is not `[!]KEY=VALUE` is `GUATIAO_ERR_BAD_VALUE`. `guatiao_registry_providers` with an empty
kind lists every provider; both listings are best first.

**These need the `load` feature** — the only part of the C surface that
does, because loading needs `libloading` and a library author takes no
dependency. Build the shipped artifact with `--features load` (or
`--all-features`), or its export table will not have them.

## Appending to a descriptor

Every descriptor leads with `struct_size`, and a reader decides field by
field what the writer covered. **A slot is appended, never changed**: a
changed meaning at an existing offset is silent memory corruption, not a
version error. `floor()` on each type is the frozen size of its v1
prefix, and each appended field has its own guard — `HostInfo::alloc_end()`,
`HostInfo::meta_end()`, and so on.

The guard is `>=` against offset-plus-size, not `>` against the offset, so
a descriptor declaring only PART of a slot reads it as absent rather than
splicing half a pointer with whatever followed.

**`struct_size` cannot version an ARRAY's element size.** It places the
fields within one element; finding element `i` needs the size the library
laid the array out at. So `Providers` carries `{ptr, len, stride}`, a
reader walks by bytes, and it refuses a stride below `ProviderInfo::floor()`
or an element whose own `struct_size` exceeds the stride (which would
overlap its neighbour). `Providers::new(&SLICE)` sets the stride for you —
a hand-set one is a number to get wrong exactly once.

Appending `meta` is what this machinery is for, and it did not move
`ABI_VERSION` or any `floor()`: a library built before it declares a
shorter `struct_size`, and the host reads `meta` as absent.

Reading a caller's struct must not start by making a reference to it — a
reference asserts the whole pointee is dereferenceable, which a caller
compiled before the last field was appended does not have.

---

# The C surface

**The `extern "C"` surface is always compiled.** It is not behind a
feature: this is an FFI library, and Rust is one of its consumers rather
than the privileged one — a surface that appears only when somebody
remembers a flag is one a C caller cannot rely on.

The crate is `crate-type = ["rlib", "cdylib"]`, so one package is both the
Rust library and the artifact a C consumer links: `cargo build` produces
`guatiao.dll` / `libguatiao.so`. An rlib exports nothing, which is why the
cdylib exists at all.

A consequence worth knowing: **a plugin cdylib exports these too**,
alongside its own entry symbol, because a cdylib that references this
crate takes its object code. Two copies in one process are harmless, and
that is a property rather than luck — every one of these functions is
pure over the structs plus the allocator pointer the tree carries, so a
tree allocated through one copy frees correctly through another.

**`cargo test` does not build a cdylib for the package under test**, only
for a dev-dependency, so `tests/c_exports.rs` needs a `cargo build` first
and states a skip otherwise. CI builds before testing.

The header at
`include/guatiao.h` is rendered by `build.rs` on every build with this
feature on, committed so a C consumer needs no Rust toolchain, and
compared byte for byte by a test. Refresh the committed copy with
`GUATIAO_WRITE_HEADER=1 cargo build -p guatiao --features c-header`.

Values: `guatiao_value_{free,clone,null,absent,bool,map,list,string,
number,bytes}`, `guatiao_map_{set,discard,clear,copy_from}`,
`guatiao_list_{push,discard,clear}`, `guatiao_string_push`,
`guatiao_buffer_push`, `guatiao_alloc_default`.
Also `guatiao_merge`, and the schema surface:
`guatiao_schema_{validate,resolve,flat_keys,flatten,unflatten}`.

Every one returns `Status` **except `guatiao_schema_resolve`**, which
answers a `const guatiao_value *` borrowed from the schema it was given,
or null — a function returning a pointer has no way to report a status,
so null is the only failure it can express.

The statuses:

```
GUATIAO_OK 0   BAD_VALUE 7   ALLOC 8   WRONG_KIND 9
NOT_FOUND 10   NULL 11       INTERNAL 100
```

Contracts that are not in the signatures:

- **Null is refused, not dereferenced.** Every exported function checks.
- **Every boundary catches unwinds**, so a panic never crosses.
- **Restricted-validity types never cross.** A tag is a `uint32_t`, not an
  enum, and a boolean is a `uint8_t`: an out-of-range integer read at an
  enum or `bool` type is undefined behaviour at the moment of the read,
  before any check could reject it. A reader **skips** a tag it does not
  know; it never stops.
- **`guatiao_map_clear` is the map one** and refuses a list, unlike the
  Rust `Value::clear`, which dispatches on the tag.
- **Every out-parameter is written `absent` on entry**, before anything
  can fail, so a caller reading one back after a failure reads what the
  call produced rather than its own uninitialised local.
- **A null or unusable allocator is `GUATIAO_ERR_ALLOC`**, in every
  module, however it is wrong.
- **The two pointers of `guatiao_map_set`, `guatiao_list_push` and
  `guatiao_map_copy_from` must not overlap**, and the same pointer for
  both is `GUATIAO_ERR_BAD_VALUE`. `guatiao_map_copy_from` does accept a
  `src` stored inside `dst`: it copies the source whole first.
- **`guatiao_string_push` and `guatiao_buffer_push` accept a view of the
  node's own buffer**; an overlapping source is copied out first.
- **The schema vocabulary is in the header** as `GUATIAO_KEY_*` macros,
  so a C consumer compares keys without spelling them.

## The allocator

**An `Allocator` may be called from any thread**, the way Rust's global
allocator can, because a value built through it is `Send` and `Sync` and
is freed wherever it ends up. A host handing out an arena synchronises
it; the crate does not lock on the host's behalf.

An owned container carries the allocator that made it, so growth and free
never take one. `cap == 0` means the buffer is **not owned** — a literal
or a borrow, never freed, copied out of on first growth. So `cap >= len`
does **not** hold on input.

**Only growth copies out.** Removing, clearing and replacing write
through the buffer the container already has, whatever its capacity says,
so a literal that will be mutated must live in **writable storage** — a
C `static` without `const`, or a local. One in read-only memory may be
read, cloned, merged, grown and freed, but not emptied.

```c
typedef struct guatiao_alloc {
  uint32_t struct_size;  void *ctx;
  void *(*alloc)(void *ctx, size_t size, size_t align);
  void  (*free)(void *ctx, void *p, size_t size, size_t align);
  void  (*release)(void *ctx);   /* optional */
} guatiao_alloc;
```

An implementer must guarantee, and `struct_size` cannot express:

- `alloc` returns memory aligned to at least `align`, or null. Returning
  a misaligned pointer is treated as null.
- `free` receives the same `(size, align)` the block was allocated with.
  Pairing `_aligned_malloc` with `free` corrupts the heap.
- Neither may unwind, throw, or `longjmp`. A foreign unwind entering Rust
  across a `"C"` boundary cannot be defended against on this side.
- **The allocator outlives every tree allocated through it.** This is a
  contract, not a borrow: `Alloc` carries no lifetime, because
  `Alloc::from_raw` is `unsafe` and hands back whatever lifetime is asked
  for, so the check would be real only for a stack-allocated allocator —
  the case nobody has.

`Alloc::rust()` is the crate's own, borrowed from a constant.
`rust_alloc()` is a `const fn` returning the vtable, for a `static`.

---

# Gotchas

- **A `Value` is not `Copy`**, and neither are the four containers. They
  have `Drop`.
- **The fields of `Value`, `Payload`, `Entry` and the four containers are
  private.** Safe code cannot forge a node, corrupt a length, or copy a
  container's fields into a second owner; the compiler refuses all three
  (pinned as `compile_fail` doctests). The one door for a literal or a
  buffer another language owns is `unsafe fn from_raw_parts` on each
  container and on `Value` (with `Payload::text/bytes/list/map/bool`),
  and `into_raw_parts` is its safe inverse. The borrowed views (`Str`,
  `Bytes`, `Values`, `Entries`) keep public fields: they own nothing. The
  C header is unchanged — cbindgen renders private fields.
- **`try_into` on a value needs `TryFrom`, not a bespoke trait.** A trait
  method named `try_into` is ambiguous against std's blanket impl at
  every call site: it compiles inside the crate and fails in a doctest.
- **Interior NUL is legal everywhere.** Keys and string values are
  pointer and length; comparing only to the first NUL makes two different
  keys look identical.
- **Key comparison is raw bytes.** No case folding, no normalisation, no
  trimming: `"Host"` and `"host"` are two keys.
- **`#![forbid(unsafe_code)]` is per module**, and a test names the paths
  allowed to contain `unsafe`: the value model, `library/raw.rs` and
  `exports`. Every other module must carry the attribute.
