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
| `c-header` | regenerating the committed `include/guatiao.h` | off |

MSRV 1.85. No required dependencies; `derive` pulls `guatiao-derive`,
and the loader pulls `libloading`.

---

# Building a value

**Building in Rust names no allocator.** The short constructors use the
crate's own allocator and abort on allocation failure, exactly as
`String::from` does. The `_in` forms name one and stay fallible, because a
foreign allocator refusing is recoverable.

```rust
use guatiao::{Map, ReadValue};

let mut options = Map::new();
options.set("compression", 6)?;

let mut map = Map::new();
map.set("host", "10.0.0.1")?;
map.set("port", 5900)?;
map.set("tls", true)?;
map.set("options", options)?;
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
```

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
nothing, null is a stored value. Absent is never stored in a list.

## Ownership

**A `Value` owns its tree and frees it on drop.** Handing it to something
else is a move, which is when Rust stops dropping it.

```rust
let mut out = Value::absent();
let status = unsafe { guatiao_merge(..., &mut out, ...) };
// `out` now owns the result and frees when it goes out of scope.
```

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
let n: i64 = map.get("options").get("compression").ok_or_missing()?.try_into()?;
```

`ReadValue` is implemented for `&Value` **and** for `Option<&Value>`, so a
path chains and a missing key does not need unwrapping at each step.
`ok_or_missing()` is `Option::ok_or` with the one error it could be.

`TryFrom<&Value>` exists for every scalar plus `&str`, `&[u8]`,
`&[Value]` and `&[Entry]`. Integer reads **never truncate**: a fractional
or exponent spelling is refused rather than rounded.

Inherent readers on `Value`:

```rust
v.tag() -> Result<Tag, ValueError>     // the FIELD `v.tag` is the raw u32
v.as_bool() / as_str() / as_bytes() / as_number_str()
v.as_map() / as_map_mut() / as_list() / as_list_mut()
v.entries() -> Option<&[Entry]>        v.items() -> Option<&[Value]>
v.get(key) / get_mut(key) / contains_key(key)
v.set(key, impl Into<Value>) / push(..) / push_into(key, ..)
v.remove(key) / discard(key) / remove_at(i) / discard_at(i) / clear()
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
order, advanced, sensitive, default)]`. Named-field structs only —
enums, unions, tuple structs and generics are refused with a message
naming the derive you wrote.

An `Option<T>` field is omitted when `None` rather than written as null,
and both spellings read back as `None`.

Generated code names only `::guatiao::` paths, checked by a test.

---

# Schema

**A schema is an ordinary value**, with a documented key vocabulary in
`schema::vocab`, so a consumer in any language reads one by walking a map.
There is no serialisation here: how a schema is written down belongs to a
layer above.

Build:

```rust
SchemaBuilder::new(alloc)
    .option(OptionBuilder::new(alloc, "port", KindBuilder::int_range(alloc, 1, 65535))
        .label("Port").help("...").required())
    .section("net", "Network", "...")
    .finish() -> Result<Value, ValueError>
```

Kinds: `bool`, `string`, `int`, `int_range`, `int_bounds`, `float`,
`float_bounds`, `bytes`, `list(items)`, `map(fields)`,
`enumeration(choices)`, `union(arms)`, `variant(tag, arms)`.

Read (borrowed views over the value, no copying):

```rust
SchemaRef::new(&value) -> Option<SchemaRef>
  .options() / .sections() / .find(key) / .extra(key) / .as_value()
OptionRef: .key() .kind() .label() .help() .section() .default() .order()
           .is_advanced() .is_sensitive() .is_required() .extra(key)
Kind: .choices() .alternatives() .arms() .items() .fields() .name()
```

Validate:

```rust
validate_map(schema, &values)   -> Result<(), ValidationError>
validate_value(option, &value)  -> Result<(), ValidationError>
validate_text(option, text)     // for a string-typed front end
validate_texts(schema, &BTreeMap<String, String>)
```

`ValidationError` is `UnknownOption { .. }` or `BadValue { .. }`. **An
error never quotes the value it refused** — an option may be sensitive —
it says what would have been accepted.

Flat projection, for a front end that only has `key -> text`:
`flatten`, `unflatten`, `keys`, `resolve`, `is_sensitive`, `check_keys`,
separator `.`.

---

# Layering

```rust
MergeMode::{Simple, Deep, Substitute}
  .merge(earlier, later, alloc) -> Result<Value, MergeError>
  .merge_with(earlier, later, alloc, options, overrides)
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
`merge_with_schema`).

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
`ProviderInfo { struct_size, vtable_size, kind, id, display_name, config,
vtable, ctx, meta }`; `HostInfo { struct_size, abi_version, host_id,
host_version, alloc, meta }`. `ABI_VERSION` is 1.

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
reg.load_file(&path)?;                     // Result<Option<&Loaded>, LoadError>
reg.providers("greeter")                   // impl Iterator<Item = &Provider>
reg.provider("greeter", "hello")           // Option<&Provider>
provider.config_schema()                   // Option<&'static Value>
provider.vtable() -> (*const c_void, usize)
provider.view().vtable_as::<T>()           // unsafe; checks vtable_size >= size_of::<T>()
provider.meta()                            // Option<&'static Map>
loaded.meta                                // Option<&'static Map>
```

**A loaded library is never unloaded.** Everything it hands over —
strings, schemas, vtables — points into its mapping, so unloading would
dangle every borrow the host holds.

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
Also `guatiao_merge` and `guatiao_schema_validate`.

Every one returns `Status`:

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

## The allocator

An owned container carries the allocator that made it, so growth and free
never take one. `cap == 0` means the buffer is **not owned** — a literal
or a borrow, never freed, copied out of on first growth. So `cap >= len`
does **not** hold on input.

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
- **Copying one container's fields into another is a double free.** Both
  then describe one allocation and both free it. Transfer with
  `ManuallyDrop` and say so.
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
