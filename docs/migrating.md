# Migrating from 0.0.0-alpha.0

Every change since `0.0.0-alpha.0` that asks something of code written
against it, grouped by what that code touches. Each entry says what
changed, what you wrote before and what to write now. The Rust you write
now is compiled by the test suite, so it is current; what you wrote before
is shown as text. Changes that ask nothing of you are listed at the end,
with the reason.

The full record, additions included, is the [changelog](changelog.md).

## Moving the pins

The crates move together, each naming the others at the exact same
version. A pre-release is only selected when it is named exactly:

```toml
[dependencies]
guatiao = { version = "=0.0.0-alpha.1", features = ["provider", "load"] }
guatiao-serde = { version = "=0.0.0-alpha.1", features = ["json"] }
guatiao-intake = "=0.0.0-alpha.1"
```

What the changelog lists under `[Unreleased]` is not in a release yet; a
pin on a git revision reaches it, `clone()` below among it.

**Build a host and its libraries from the same release.** A library built
against `0.0.0-alpha.0` may load, but a kind whose table hash changed
refuses it (see [Rebuild every provider](#rebuild-every-provider-against-the-new-kind-crate)),
and the checks a descriptor now passes are the new release's.

## Values

### Building in Rust names no allocator; the `_in` forms do

`ToValue::to_value` and `Schema::schema` take no allocator and build in
the crate's own, as every other short form does. `to_value_in(alloc)` and
`schema_in(alloc)` take one, and `to_value_in` is the method a
hand-written `impl ToValue` implements. The derives write it for you.

Before:

```text
impl ToValue for Port {
    fn to_value(&self, alloc: Alloc) -> Result<Value, ValueError> { .. }
}
let port = Port(5900).to_value(Alloc::rust())?;
let schema = Vec::<String>::schema(Alloc::rust())?;
```

Now:

```rust
use guatiao::{Alloc, Schema, ToValue, Value, ValueError};

struct Port(u16);

// A hand-written impl names the allocator in `to_value_in`.
impl ToValue for Port {
    fn to_value_in(&self, alloc: Alloc) -> Result<Value, ValueError> {
        self.0.to_value_in(alloc)
    }
}

// Building in this binary's allocator names none.
let port = Port(5900).to_value()?;
let schema = Vec::<String>::schema()?;

// Building in another one says so with `_in`.
let port_there = Port(5900).to_value_in(alloc)?;
let schema_there = Vec::<String>::schema_in(alloc)?;
```

A hand-written `impl Schema` is unchanged: it implements `kind(alloc)`.

### `set`, `push` and `push_str` return nothing

`Map::set`, `List::push`, `Text::push_str` and `Buffer::push` return `()`
and abort on an allocation failure, as `Vec::push` does. `Text::new` and
`Buffer::new` abort the same way rather than panicking. The fallible
forms are `try_set`, `try_push` and `try_push_str` (`AllocError`), and the
`_in` forms, which name an allocator, are unchanged.

Before:

```text
map.set("port", 5900)?;
list.push("a").unwrap();
let _ = text.push_str("b");
```

Now:

```rust
use guatiao::{List, Map, Text};

// The short forms grow in the container's own allocator and abort if it
// refuses, as `Vec::push` does: nothing to `?` or `unwrap`.
let mut map = Map::new();
map.set("port", 5900);
let mut list = List::new();
list.push("a");
let mut text = Text::new("a");
text.push_str("b");

// Where the allocator may refuse and you handle it, say `try_`.
let mut settings = Map::new_in(alloc);
settings.try_set("port", 5900)?;
list.try_push("b")?;
text.try_push_str("c")?;
```

### A mutable map iterates key and value

`for .. in &mut map` and the new `Map::iter_mut` yield `(&Text, &mut
Value)`, as `HashMap::iter_mut` does. They yielded `&mut Entry`, through
which a whole entry, key included, could be overwritten.

Before:

```text
for entry in &mut map {
    *entry.value_mut() = Value::null();
}
```

Now:

```rust
for (_key, value) in &mut map {
    *value = Value::null();
}
```

### `clone()` copies into this binary's allocator

*Not released yet: under `[Unreleased]`.* `clone()` and `to_owned()`
on `Value`, `Map`, `List`, `Text`, `Buffer` and `Number` build the copy in
`Alloc::rust()`, whatever allocator the source recorded, so a copy of a
tree in an arena, a caller's allocator or a library's never depends on
it. `clone()` also aborts on an allocation failure; `clone_in` reports
one.

Before, `value.clone()` copied beside the source. Now:

```rust
// A copy in this binary's allocator, whatever the source recorded.
let copy = value.clone();
// A copy beside its source, as `clone()` made before.
let beside = value.clone_in(value.alloc()?)?;
```

### `Status` is a number, and `Tag` may grow

`Status` is `#[repr(transparent)]` over `u32` with the same `GUATIAO_*`
associated constants, so a code a library or host made up is a `Status`
too. `Tag` is `#[non_exhaustive]`. A `match` on either keeps a `_` arm.

Before, a `match` listed every variant, and a code crossed as
`status as u32`. Now:

```rust
use guatiao::{Status, Tag};

fn describe(status: Status) -> &'static str {
    match status {
        Status::GUATIAO_OK => "done",
        Status::GUATIAO_ERR_BUSY => "try again later",
        // Any code is a `Status`, a host's own included.
        _ => "refused",
    }
}

let status = Status::from(14u32);
let code: u32 = u32::from(status);

let is_map = match value.tag()? {
    Tag::GUATIAO_MAP => true,
    // Tags are appended, so a `match` keeps a `_` arm.
    _ => false,
};
```

## Schema and derives

### A typed field is checked as its type

`validate_value` and `validate_map` check a field as the type it declares,
as that type's `FromValue` reads it: a boolean field takes a boolean, an
integer or number field a `Number`, a string field text, and `enum` is
enforced by value on every type. Text that used to pass because it
spelled the right thing (`"yes"`, `"5900"`, `"inf"`) is refused.
`validate_text` reads a number as JSON writes one (`inf`, `+1`, `007`
and `1e999` are refused).

Before:

```text
config.set("port", "5900")?;
config.set("verbose", "yes")?;
validate_map(schema, &config.into())?;
```

Now:

```rust
use guatiao::Map;
use guatiao::schema::{validate_map, validate_text};

// Each field holds its own type: a number for an integer, a boolean for
// a boolean, never their text.
let mut config = Map::new();
config.set("port", 5900);
config.set("verbose", true);
validate_map(schema, &config.into())?;

// Text a person typed is checked as text, one field at a time.
let port = schema.find("port").expect("declared");
validate_text(port, "5900")?;
```

### A schema that says nothing about extra keys accepts them

An object without `additionalProperties` accepts a key it does not declare,
as JSON Schema reads it; one whose `additionalProperties` is a schema
checks the extra key's value against it. The builders and the derives
write `false`, so their documents still refuse an undeclared key. A
hand-written schema that relied on the refusal says so:

```json
{
  "type": "object",
  "properties": { "port": { "type": "integer" } },
  "additionalProperties": false
}
```

### A derived `Option<T>` admits null

The derived schema of an `Option<T>` field is `T`'s accepting null as
well: `type: ["string", "null"]`, or `anyOf: [T, {"type": "null"}]` for
an enum, a union or a variant. Integer widths write their bounds as exact
digits, and an `f32` field writes the bounds past which no finite `f32`
exists. Code that read `type` as one string reads the field instead:

```rust
use guatiao::Schema;
use guatiao::schema::{KindRef, SchemaRef};

#[derive(Schema)]
struct Settings {
    /// `type: ["string", "null"]` now, as `FromValue` reads it.
    proxy: Option<String>,
}

let document = Settings::schema()?;
let settings = SchemaRef::new(&document).expect("a schema");
for field in settings.fields() {
    // Read through the field, never the `type` key: it is either shape.
    let optional = field.is_nullable();
    let is_text = matches!(field.kind(), KindRef::Str { .. });
    assert!(optional && is_text);
}
```

`Kind` (`schema::KindRef`) gains `Null`, and an integer `enum` reads as
`Kind::Enum`, not `Kind::Int`. `Kind` was already `#[non_exhaustive]`.

### Merging: fallible schema reads, paths as segments

`schema::merge::merge_overrides` and `merge_options` return a `Result`, as
`merge_with_schema` can fail: a schema nested past `MAX_DEPTH` is
`MergeError::Build(ValueError::TooDeep)`. A merge path is a list of
segments, so the top-level key `tls.ca` and the key `ca` inside `tls` are
different places: `Provenance::source_of`, `Provenance::leaves` and
`MergeOverrides::{set, with, get}` take or yield segments. A
`MergeError::Kind` path and the C override table's `path` use
`guatiao-intake`'s path text (`tls.ca`, `peers[0].name`, `["a.b"]`).

Before:

```text
let overrides = merge_overrides(schema).with("tls.ciphers", MergeMode::Deep);
let from = provenance.source_of("tls.ca");
for (path, layer) in provenance.leaves() { println!("{path} from {layer}") }
```

Now:

```rust
use guatiao::MergeMode;
use guatiao::schema::merge::{merge_options, merge_overrides};

// Reading the `x-merge` annotations can fail: past `MAX_DEPTH`.
let overrides = merge_overrides(settings)?;
let options = merge_options(settings)?;

// A path is its segments: `tls.ca` as a key is `["tls.ca"]`.
let overrides = overrides.with(["tls", "ciphers"], MergeMode::Deep);
let (merged, provenance) = MergeMode::Substitute.merge_layers_with(
    [("system", &system), ("user", &user)],
    guatiao::Alloc::rust(),
    options,
    Some(&overrides),
)?;
let from = provenance.source_of(["tls", "ca"]);
for (path, layer) in provenance.leaves() {
    println!("{} from {layer}", path.join("."));
}
```

### A raw identifier is keyed without `r#`

A field `r#type` is written, read, described and given form hints under
`type`, as serde keys it. A stored value keyed `r#type` is renamed, or the
field keeps the old key:

```rust
use guatiao::{FromValue, ToValue};

#[derive(ToValue, FromValue)]
struct Record {
    // Keyed `type` now, as serde keys it. To keep reading a stored
    // `r#type`, name that key:
    #[map(rename = "r#type")]
    r#type: String,
}
```

### Refused where they used to pass

- **A field key declared twice**: `SchemaBuilder::finish`,
  `KindBuilder::map` and the arm builders answer
  `ValueError::DuplicateKey`, and `KindBuilder::variant` the same for two
  arms stored as one discriminant. The second used to replace the first.
- **A `pattern`'s `\d`, `\D`, `\w` and `\W` are ECMA-262's ASCII
  classes**, as JSON Schema specifies, where the `regex` crate's are
  Unicode: Arabic-Indic digits no longer match `\d`. Write the class you
  mean (`[0-9]`, or `\p{Nd}` for any decimal digit).
- **Numeric bounds are compared exactly and all of them apply**:
  `exclusiveMinimum` and `exclusiveMaximum` are enforced, a bound past
  `i64` (`u64::MAX`) is a bound, and below a minimum of zero `-0` is
  refused. A value one of these refuses was accepted by mistake.
- **`export_schema!`'s symbol uses the crate's name**
  (`CARGO_CRATE_NAME`): a crate `acme-net` exports `acme_net_conn_schema`
  rather than `acme-net_conn_schema`, which C could not declare. A C
  caller declares the new name.

## Kinds and providers

### Rebuild every provider against the new kind crate

A kind's table hash (`FLOOR_HASH`) reads standard and `guatiao` paths as
their bare names: `std::vec::Vec<T>` and `Vec<T>`,
`guatiao::library::ProviderError` and `ProviderError` hash alike, and a
reference's lifetime is left out (`&'_ str` is `&str`). A kind whose
required methods spelled one of those paths has a new hash, and a host
built against it refuses every provider built before with
`KindMismatch::HashMismatch` until the provider is rebuilt; a C table
takes the new `*_FLOOR_HASH` from the regenerated header. A kind spelled
with bare names keeps its hash.

A kind's table hash also covers every slot now, optional ones too, in
table order: a kind with an optional slot has a new hash (the example
greeter kind's is 2032737757), and a hash written by hand, in a C table,
a Julia `@kind`, an R `kind()` or a Python `KindProtocol` class, is taken
from the regenerated header or kinds document.

Rebuilding is the answer either way: a provider built now also builds its
answers in the caller's allocator (below) and passes the new checks.

### An `Offer` is not the kind

An `Offer` no longer derefs to its kind: a provider built only from a
configuration has no default instance, and calling one through its offer
could only fail. `default_instance()` is `Option<&K>`; `boxed()`,
`shared()` and `remote()` answer an `Option`, `None` for such a provider;
`Provider::as_kind` and `ProviderInfo::as_kind` answer
`Result<Option<Remote<K>>, KindMismatch>`. `From<Offer<K>> for Remote<K>`
is gone.

Before:

```text
let offer = registry.offer::<dyn Greeter>("acme_hello").expect("filed")?;
offer.greet("ana")?;
let remote = Remote::from(offer);
```

Now:

```rust
use guatiao::library::Remote;

let offer = registry
    .offer::<dyn Greeter>("acme_hello")
    .expect("filed under that key")?;
// A provider built only from a configuration has no default instance.
if let Some(greeter) = offer.default_instance() {
    greeter.greet("ana")?;
}
// `remote()`, `boxed()` and `shared()` answer an `Option` for the same
// reason.
let remote: Option<Remote<dyn Greeter>> = offer.remote();
```

### `instantiate` checks the configuration first

`instantiate` on an `Offer`, a `Provider` or a `ProviderInfo` validates the
configuration against the provider's schema before the provider is asked,
and answers `InstantiateError`: `Invalid(ValidationError)` names a
misspelled or ill-typed setting, `Refused(ProviderError)` is the
provider's own answer. `.status()` gives the code either way, and a
separate `validate_map` beforehand is no longer needed.

Before:

```text
validate_map(schema, &config)?;
let instance = offer.instantiate(&config).map_err(|e: ProviderError| ..)?;
```

Now:

```rust
use guatiao::library::InstantiateError;

let shouter = registry
    .offer::<dyn Greeter>("acme_shouter")
    .expect("filed under that key")?;
match shouter.instantiate(&config) {
    Ok(instance) => println!("{:?}", instance.greet("ana")?),
    // The configuration does not fit the provider's schema: the
    // provider was never asked.
    Err(InstantiateError::Invalid(why)) => println!("check the settings: {why}"),
    // The provider's own refusal.
    Err(InstantiateError::Refused(why)) => println!("refused: {why}"),
    Err(other) => return Err(other.into()),
}
```

### `Remote` is `Clone`, not `Copy`

Every handle (`Remote`, `Offer`, `Instance`, `Object`, `ProviderView`)
holds a lease on its library, which is what keeps the library loaded while
the handle lives. A `Remote` used after a move is cloned, and
`Instance::remote()` answers `&Remote<K>`, borrowed from the instance.

Before:

```text
let kept = remote;            // and `remote` still usable: it was `Copy`
let owned = instance.remote();
```

Now:

```rust
// `Remote` is `Clone`, not `Copy`: each holds its library open.
let remote = offer.remote().expect("a default instance");
let kept = remote.clone();
// `Instance::remote()` lends the instance's own; clone it to keep it.
let owned: Remote<dyn Greeter> = instance.remote().clone();
```

### `available()` lends its reason

`available()` on `ProviderView`, `Provider` and `Offer` answers
`Result<(), &str>`, the reason borrowed from the handle, rather than
`&'static str`.

```rust
// The reason is borrowed from the offer, which holds the library open.
let why_not: Option<String> = offer.available().err().map(str::to_string);
```

### `ProviderParts::builder` replaces `ProviderParts::new`

A provider described without the derive is built slot by slot; every slot
defaults to absent, so the description keeps compiling when the envelope
appends one.

Before:

```text
let parts = ProviderParts::new(
    "acme_hello", "Hello", "1.0.0", &["greeter"], tables,
    None, ctx, None, None, None,
);
```

Now:

```rust
use guatiao::library::kind::ProviderParts;

let parts = ProviderParts::builder("acme_hello")
    .name("Hello")
    .version("1.0.0")
    .kinds(&["greeter"])
    // then `.table(..)` per table, `.ctx(..)`, and one call per slot
    // you fill (`.available`, `.create`, `.destroy`, `.config`, `.meta`)
    .build();
```

### An object's table is chosen by its value's type

`Object::from_cell` takes `Box<ObjectCell<K::Vtable, T>>`, and
`ObjectCell::new(value)` fills the table the kind attribute built for `T`;
a table built for another type or kind does not compile. `into_object()`
is unchanged and is the usual door.

Before:

```text
let cell = Box::new(ObjectCell { table: NoteVtable::of::<Sticky>(), value: sticky });
let note = Object::<dyn Note>::from_cell(cell)?;
```

Now:

```rust
use guatiao::library::Object;
use guatiao::library::kind::ObjectCell;

struct Sticky(String);

impl Note for Sticky {
    fn size(&self) -> i64 {
        self.0.chars().count() as i64
    }
}

// What the kind attribute builds for you:
let note: Object<dyn Note> = Sticky("hi".into()).into_object();
// or the cell spelled out, its table chosen by the value's type:
let cell = Box::new(ObjectCell::new(Sticky("hi".into())));
let same: Object<dyn Note> = Object::from_cell(cell)?;
```

An `Object<dyn K>` is `Sync` only when `K`'s trait names `Sync`. Share
the handle of a kind naming `Send` alone through a `Mutex`, or name
`Sync` on the kind.

### Refused when the kind or provider compiles

Each is a compile error now, with a sentence naming the fix; code that
compiled with one of them changes as said:

- an object kind's method named `destroy` or `into_object`, or any kind's
  method named `header`: rename it;
- an argument whose name starts with `__`: rename it;
- a kind named twice in `#[provider(K, K)]`: drop the repeat;
- a method `<m>_in` beside a method `<m>` answering a tree:
  `#[guatiao::kind]` adds `<m>_in(.., alloc)` itself, so rename yours;
- an object kind (`#[kind(object)]`) in `#[provider(..)]`: an object is
  made by a method and handed over, never served by a provider. A registry
  answers `KindMismatch::ObjectKind` for one.

### A description's `bytes_mut` belongs to an object kind

`#[guatiao::kind]` has always refused `&mut [u8]` on a kind that is not
an object kind; the library's description reader now refuses
`"type": "bytes_mut"` there too, and so does every binding that builds
a kind (Node's `kind()` with `t.bytesMut`, Python's `Kind` with
`BYTES_MUT` or a `bytearray` annotation, Go's glue, Dart's describer,
Julia's `@kind` with `Lent{Vector{UInt8}}`, R's `kind()` and MATLAB's
`guatiao.Kind.describe` with a lent `bytes`). A
description written by hand with one makes its kind an object kind
(`"object": true`), or passes the bytes as `bytes` and answers what the
callee would have written.

### A description declares only what a trait could

The library's description reader, and every binding that builds a kind
(Node's `kind()` and `kindsOf`, Python's `Kind`, Go's glue, Julia's
`@kind`, R's `kind()`, MATLAB's and Octave's `guatiao.Kind.describe` and
`guatiao.write_kinds`), now refuse three more things `#[guatiao::kind]`
has always refused:

- a slot that passes or answers an `object` without `"fallible": true`:
  make the slot fallible (its table gains `guatiao_provider_error *err`
  last), since the object's table is checked where it arrives;
- a `list` passed `borrowed`: lend it (`"pass": "lent"`) or hand it over
  (`"pass": "taken"`);
- a slot named `header`, the table's first field: rename it.

Node's TypeScript reader, Dart's `describe` and Go's `-document` refuse the
same declarations through the library: mark such a method `@throws`,
`@fallible` or fallible (no `//guatiao:infallible`), and pass a list
`Lent<ListRef>`/`Taken<ListRef>` or marked `lent`/`taken`.

## Loader and handles

### `unload` is safe, and refuses while something holds the library

`Registry::unload` is a safe `fn`. It answers `UnloadError::Busy { key,
leases }` while a handle taken from the library, or an answer another
library pinned through the host's services, is alive; then it asks the
library's own `unload` slot; then it closes the mapping. A refusal leaves
the library registered. `unload_unchecked` is the `unsafe` form that skips
the count and a missing slot.

Before:

```text
unsafe { registry.unload("acme")? };
```

Now:

```rust
use guatiao::library::UnloadError;

// Drop what you took from the library first: every `Remote`, `Offer`,
// `Instance`, `Object` and `ProviderView` holds it open.
drop(offer);
match registry.unload(&library_key) {
    Ok(()) => println!("unloaded"),
    Err(UnloadError::Busy { leases, .. }) => println!("{leases} handles still hold it"),
    Err(other) => return Err(other.into()),
}
```

`retire` takes a library out of the registry and leaves it mapped until
nothing holds it; `Registry::collect()` then closes it and answers the
keys it closed, as does the registry's next load, register, retire or
unload. Dropping a handle never closes anything, and **dropping a
`Registry` no longer unmaps** its libraries: unload or collect them first
if you need them gone.

### `ProviderView` is read through getters

Its fields are private, so a view only comes from reading a descriptor.
`view.id` is `view.id()`, and so on for `kinds`, `display_name`, `config`,
`meta`, `version` (`Option<&str>`), `vtable`, `vtable_size`, `ctx`,
`tables`, `create` and `destroy`. `ProviderView::raw` is gone.

```rust
let view = offer.view();
let id: &str = view.id();
let version: Option<&str> = view.version();
let has_config = view.config().is_some();
```

### A host's services answer views

`Host::get` and `Host::list` answer `ProviderView`s, read under the size
each descriptor declares, rather than `&'static ProviderInfo`, and each
holds the provider's library open. `Host::mismatches` pairs a view with
its mismatch. Drop the `.view()` call you made on the answer.

```rust
// The `Host` a library is handed; a registry gives its own.
let host = registry.host();
// A view, read under the size its descriptor declares, holding the
// provider's library open while it lives.
if let Ok(Some(view)) = host.get("derived_greeter_hello") {
    println!("{} {}", view.id(), view.display_name());
}
```

### `register_local` takes a closure and owns what it is given

`Registry::register_local(name, describe)` takes any `FnOnce(Host) ->
Option<L>` where `L: Into<LocalLibrary>`: owned `LibraryParts`, kept by
the library's lease and freed once it is retired and unused, or a
borrowed `&LibraryInfo`, copied at the registration. `local_providers!`
writes `library(Host) -> Option<LibraryParts>`, built on each call.
`register_local(name, library)` and a describe answering
`Option<&'static LibraryInfo<'static>>` compile unchanged; a describe that
leaked its parts into a `OnceLock` returns them instead, and a caller of
`library(host)` itself gets `LibraryParts` (`.info()` for the descriptor).

```rust
use guatiao::library::Host;
use guatiao::library::kind::LibraryParts;

// `local_providers!` writes `library(Host) -> Option<LibraryParts>`,
// built on each call; a describe of your own may answer one by value.
fn describe(host: Host) -> Option<LibraryParts> {
    acme::library(host)
}
registry.register_local("acme", describe)?;
```

### A descriptor is checked where it enters

Loading or registering a library checks what its descriptor says before
reading through it: pointers aligned, outside the low 64 KiB and inside
the library's image where one is mapped, lengths and strides capped,
every slot of a table null or code. `LoadError::Malformed` gains `reason`,
naming the part and the fault, so a pattern on it adds `..`:

```rust
use guatiao::library::LoadError;

match registry.load_file(&path) {
    Err(LoadError::Malformed { path, reason, .. }) => {
        println!("{} refused: {reason}", path.display());
    }
    Err(other) => println!("{other}"),
    Ok(loading) => println!("{loading:?}"),
}
```

### `LoadReport.loaded` pairs each file with its key

`LoadReport.loaded` is `Vec<(PathBuf, String)>`, each loaded file with the
library key the registry filed it under. The C report is unchanged.

```rust
use guatiao::library::scan_dir;

let report = scan_dir(&mut registry, dir)?;
for (path, key) in &report.loaded {
    println!("{} filed as {key}", path.display());
}
```

### `KeyFields` has a `source`

A key template may name `%source`, the source a library came from, so
`KeyFields` gained a field for it. A `KeyFields` written out field by
field names it too; `KeyFields::library` sets it empty, and
`with_source` sets it.

```rust
use guatiao::library::{KeyFields, KeyTemplate};

let template = KeyTemplate::library("%source/%id")?;
let fields = KeyFields::library("device", "1.0.0").with_source("device-0");
assert_eq!(template.render(fields), "device-0/device");
```

### A provider descriptor carries its kinds' descriptions

`ProviderInfo` gains `kind_descriptions`, appended after `destroy`: the
JSON document describing the kinds the provider serves. A descriptor
built with `ProviderParts::builder` or `#[derive(Provider)]` needs
nothing; a `ProviderInfo` written as a Rust struct literal names the new
field, `kind_descriptions: Str::new("")` for none. A library compiled
before the field keeps loading, read as carrying none, and C, C++ and
every binding's descriptor are unchanged unless they set it.

### On Windows, what a loaded file imports is not looked for on `PATH`

The libraries a library or source file imports (its DLLs) are looked
for beside the file, beside the host's executable, in the system
directory and in directories the process added, and no longer in the
current directory or on `PATH`. A file whose imports live elsewhere,
such as the Python bridge's `python3.dll` in a host that is not Python,
is refused, the reason saying where the search did not look, until the
host names the search that adds them:

```rust
use guatiao::library::{ImportSearch, Registry};

let mut registry = Registry::new("my-host", "1.0");
// What the files opened from now on import may also be found in
// the current directory and on PATH, as python3.dll is.
registry.set_import_search(ImportSearch::Path);
```

From C, `guatiao_registry_set_import_search(reg,
GUATIAO_IMPORT_SEARCH_PATH)`; every binding's registry has the same
setting ([where a library's imports are found](index.md#where-a-librarys-imports-are-found)).
Shipping what a plugin imports beside it needs nothing. Linux and macOS
are unchanged.

## guatiao-serde

### Reading a JSON number exactly is its own feature

`json` no longer turns on `serde_json/arbitrary_precision`, which broke
`#[serde(untagged)]` and `#[serde(flatten)]` numbers in every crate of a
build. Without the new `exact-numbers` feature an integer that fits
`i64`/`u64` is still exact, anything else reads through an `f64` (`1.10`
as `1.1`), and `1e400` is refused. A binary that needs exact numbers says
so; a library should not:

```toml
guatiao-serde = { version = "=0.0.0-alpha.1", features = ["exact-numbers"] }
```

A C host and every binding read numbers exactly when the `guatiao_serde`
library they load was built with it.

### A document that does not parse is `Error::Syntax`

It carries the parser's message and a `Position` (line and column, both
from 1, the column in characters), and never quotes the document.
`Error::Format` is now only a writer refusing a value.

Before:

```text
Err(Error::Format { detail, .. }) => /* scrape the line out of `detail` */
```

Now:

```rust
use guatiao_serde::text::{Error, json};

match json::from_str(text, guatiao::Alloc::rust(), Default::default()) {
    Ok(value) => println!("{value:?}"),
    // Where it stopped, and the parser's words; never the text itself.
    Err(error @ Error::Syntax { .. }) => {
        if let Some(at) = error.position() {
            println!("line {}, column {}: {error}", at.line, at.column);
        }
    }
    Err(other) => return Err(other.into()),
}
```

## guatiao-intake

### The path grammar is stricter

`a[0]b`, `a["x"]y`, `a.[0]` and `[a[b]` are refused
(`PathError::MissingSeparator`, `EmptyField`, `UnclosedIndex`), and digits
in a bracket are a position only without a leading zero: `a[01]` is the
key `01`, no longer position 1. Write a key starting with `0` as itself
(`env[01]`) and a position as plain digits.

### `flat::split` answers a `Cow` and splits before the last segment

It reads the key with the path grammar, so it agrees with `flatten` and
`keys` for a member whose name needs a bracket.

Before: `split(key) -> Option<(&str, &str)>`. Now:

```rust
use guatiao_intake::flat::split;

// The member is a `Cow`: borrowed unless the key had to be unquoted.
if let Some((owner, member)) = split("tls[\"a.b\"]") {
    assert_eq!((owner, &*member), ("tls", "a.b"));
}
```

### A top-level field whose key holds a `.` is stored under a bracket

A field `a.b` is stored as `[a.b]`, so it no longer shares a key with the
member `b` of a field `a`. A store written the old way still reads back:
`unflatten`, `resolve`, `resolve_in` and `validate_texts` take the bare
name too, and the new spelling wins where both are present. A front end
that builds store keys itself asks `flat::keys` for them.

## The C surface

No function's signature changed; what follows changes what a C caller
may rely on.

### `guatiao_status` is a plain integer

`guatiao_status` is `typedef uint32_t` in C and C++ alike, and the
constants belong to `enum guatiao_status_code`. Code that named the enum
(C++, or C23) names the typedef:

```c
/* before */ enum guatiao_status st = guatiao_map_clear(&v);
/* now    */ guatiao_status st = guatiao_map_clear(&v);
```

### An out parameter is written first

Every function with an out parameter writes it ABSENT (or null) on entry,
as each header's preamble says, so a caller can always free it afterwards.
`out` must not hold a tree you still own: free or move it first.
`guatiao_registry_provider_create` writes `out` null and `err` an empty
error first.

### Some statuses say what was wrong

- `guatiao_map_discard` and `guatiao_list_discard` on the wrong kind
  answer `GUATIAO_ERR_WRONG_KIND` (they said `GUATIAO_ERR_NOT_FOUND`).
- A string or map key that is not UTF-8 is `GUATIAO_ERR_BAD_VALUE` (it
  was `GUATIAO_ERR_WRONG_KIND`); in Rust, `MapError::BadValue` (it was a
  `WrongType`).
- A registry function given text that is not UTF-8 answers
  `GUATIAO_ERR_BAD_VALUE`, and one given an unusable allocator
  `GUATIAO_ERR_ALLOC` (both were `GUATIAO_ERR_NULL`).
- `guatiao_registry_provider_create` checks the configuration against the
  provider's schema first: `GUATIAO_ERR_BAD_VALUE`, the refusal in `err`.
- `guatiao_registry_unload` answers `GUATIAO_ERR_BUSY` while an instance
  built through `guatiao_registry_provider_create`, or an answer another
  library pinned, holds the library.

### A raw table or `ctx` is valid until its library closes

A table, `ctx` or borrowed schema taken from a library is valid until the
library is closed, which after `guatiao_registry_retire` may be the
registry's next call (`guatiao_registry_collect` closes retired libraries
nothing holds, and so does the next load, retire or unload). Let it go
before then.

### Calling a kind's table: initialise the out slot

A caller of a slot answering a tree passes an initialised out slot: a
carrier (an empty map or list, or an ABSENT value, whose `alloc` names
where the answer should be built) or zeroes. A provider then builds its
answer there, so it outlives the library. The contract is beside
`guatiao_kind_header` in `guatiao.h`. A C provider builds into the
carrier's allocator by reading its `alloc`.

### A library reading other libraries through its host

`get` and `list` on the host's services answer descriptors the registry
built, valid until that provider's library is retired, rather than the
library's own. A library that keeps such a descriptor past the call, or
may race a retire, uses the appended `get_pinned` and `list_pinned`, then
`unpin` when done.

### `guatiao_intake.h` spells its separator `GUATIAO_INTAKE_SEPARATOR`

The header defined a bare `SEPARATOR` macro, which took that name from
every file that included it. It is now `GUATIAO_INTAKE_SEPARATOR` (still
`'.'`); write that name where you used the old one. Rust's
`guatiao_intake::SEPARATOR` is unchanged, and so is Dart's value, now the
constant `GUATIAO_INTAKE_SEPARATOR`.

## Serde and intake are packages of their own

Every binding offers JSON, TOML and YAML, and forms, as packages beside
the core one, each over its own library and depending on the core
package. Add the packages you use and change the imports. Each finds its
library through `GUATIAO_SERDE_LIBRARY` or `GUATIAO_INTAKE_LIBRARY`, then
`GUATIAO_LIBRARY` as before (a directory holding it, or a file exporting
its functions, else the file beside it), so a setting that worked keeps
working.

The packages also moved inside the repository, to
`bindings/<language>/<name>` with each directory named as its registry
spells the package (`bindings/python/guatiao-serde`,
`bindings/julia/GuatiaoSerde`, `bindings/r/guatiao.serde`,
`bindings/fortran/guatiao`, ...). Only what names a repository path
changes: an install from a subdirectory, a CMake `add_subdirectory`, and
the Go module paths below. An fpm git dependency is unchanged.

### Python

```bash
pip install guatiao-serde guatiao-intake
```

```python
# before
from guatiao import intake, serde
from guatiao.errors import ParseError
```

```python
import guatiao_intake as intake
import guatiao_serde as serde
from guatiao_serde import ParseError
```

`guatiao.use_library` names only the core library now; the others have
their own `use_library`.

### Dart

```bash
dart pub add guatiao_serde guatiao_intake
```

```dart
// before
import 'package:guatiao/serde.dart' as serde;
import 'package:guatiao/intake.dart' as intake;
```

```dart
import 'package:guatiao_serde/guatiao_serde.dart' as serde;
import 'package:guatiao_intake/guatiao_intake.dart' as intake;
```

`Format` and `ParseError` come from `guatiao_serde` now (`serde.Format`).
A package generating its own bindings over `guatiao.h` imports the symbol
file `package:guatiao/ffi_symbols.yaml` (it was
`package:guatiao/src/symbols.yaml`). `useLibrary` registers a library for
the serde and intake surfaces as before.

### Node.js

```bash
npm install guatiao-serde guatiao-intake
```

```ts
// before
import * as serde from 'guatiao/serde';
import * as intake from 'guatiao/intake';
import { ParseError } from 'guatiao';
```

```ts
import * as serde from 'guatiao-serde';
import * as intake from 'guatiao-intake';
import { ParseError } from 'guatiao-serde';
```

`useLibrary` from `guatiao` names only the core library now;
`guatiao-serde` and `guatiao-intake` each have their own.

### Go

The binding is three modules, one per crate, released together under one
version: `github.com/Boriken-Dev/guatiao/bindings/go/guatiao` (package
`guatiao`), `.../bindings/go/guatiao-serde` (package `serde`) and
`.../bindings/go/guatiao-intake` (package `intake`). The serde and intake
modules require the core at the same version.

```sh
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde
go get github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake
```

```go
// before
import (
    "github.com/Boriken-Dev/guatiao/bindings/go"
    "github.com/Boriken-Dev/guatiao/bindings/go/intake"
    "github.com/Boriken-Dev/guatiao/bindings/go/serde"
)
```

```go
import (
    guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
    intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"
    serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
)
```

An import of `.../bindings/go/guatiao/serde` or
`.../bindings/go/guatiao/intake` changes the same way. The package names
did not change, so only the imports do; an import names the serde and
intake packages, since their paths no longer end in their names.

A program that named its library with `guatiao.UseLibrary` and relied on
`guatiao_serde` and `guatiao_intake` being found beside it names them too,
or sets `GUATIAO_LIBRARY` to their directory:

```go
guatiao.UseLibrary(dir)
serde.UseLibrary(dir)
intake.UseLibrary(dir)
```

### Julia

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/GuatiaoSerde")
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/GuatiaoIntake")
```

```julia
# before
using Guatiao
Serde.parse(text); Intake.check(schema, form)
```

```julia
using Guatiao, GuatiaoSerde, GuatiaoIntake
GuatiaoSerde.parse(text); GuatiaoIntake.check(schema, form)
```

`ParseError` is `GuatiaoSerde.ParseError` (exported by `GuatiaoSerde`).
`Guatiao.use_library` names only the core library; the others have their
own `use_library`.

### R

```r
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r/guatiao.serde")
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r/guatiao.intake")
```

```r
# before
library(guatiao)
serde_parse(text); intake_check(schema, form)
```

```r
library(guatiao.serde)     # attaches guatiao too
library(guatiao.intake)
serde_parse(text); intake_check(schema, form)
```

The functions keep their names. `use_library(path)` names only the core
library; the others are named with `use_library(path, library =
"guatiao_serde")` and `"guatiao_intake"`, or found through
`GUATIAO_SERDE_LIBRARY`, `GUATIAO_INTAKE_LIBRARY` or `GUATIAO_LIBRARY`.

### MATLAB and Octave

Nothing to install: `guatiao.serde` and `guatiao.intake` stay in the
package. A session that named its library with `guatiao.use_library`
and relied on `guatiao_serde` and `guatiao_intake` being found beside it
names them too, or sets `GUATIAO_LIBRARY` to their folder:

```matlab
guatiao.use_library(folder);
guatiao.serde.use_library(folder);
guatiao.intake.use_library(folder);
```

### Fortran

Nothing to add: `guatiao_serde` and `guatiao_intake` stay modules of the
package. A program that named its library with `guatiao_use_library`
and relied on the others being found beside it names them too, or sets
`GUATIAO_LIBRARY` to their directory:

```fortran
call guatiao_use_library(dir)
call guatiao_serde_use_library(dir)
call guatiao_intake_use_library(dir)
```

## Python and Dart

### `create` checks the configuration first

`Registry.create` validates the configuration against the provider's
schema before the provider is asked, and raises `BadValue` with the
refusal's words. A separate check beforehand is no longer needed.

### `unload` refuses while something holds the library

`unload` raises `Busy` while an `Instance` built from the library is open,
or another library holds a pinned answer from it. Close the instance
first, or `retire` the library and `collect()` once nothing uses it
(`collect()` answers the keys it closed):

```python
instance.close()
reg.unload("acme")
```

```dart
instance.close();
reg.unload('acme');
```

A table and `ctx` taken from a retired library (`provider_table`,
`providerTable`) are valid until it closes, which may be the registry's
next call. Closing a registry (Python `Registry.close()`) no longer
unmaps its libraries.

### Dart 3.10 or newer

The three Dart packages ask for Dart 3.10 (Flutter 3.38), the first with
build hooks, and their hooks now build the libraries for the app (see
[Dart and Flutter](flutter.md)). Raise the application's `sdk:` constraint
to `^3.10.0`. An application that supplies its own copy of the library, as
before, says so in its `pubspec.yaml`:

```yaml
hooks:
  user_defines:
    guatiao:
      build: false
```

### Stricter intake answers

- `intake.unflatten` raises `TypeError` (Python) or throws `ArgumentError`
  (Dart) for a store that is not text to text; `None`/`null` means only
  "nothing stored".
- What `intake.resolve` and `intake.at` return is a read-only view: every
  write through it raises (`TypeError`, `StateError`), and reading it after
  the schema or value it came from is closed raises (`ValueError`,
  `StateError`) instead of reading freed memory.

## Node.js kinds

### A kind that exists is loaded, not described

A kind described by hand with `kind()` still works, and a description
copied from a header still has to be kept in step with it by hand. Load
the kind crate's description instead: write the module once with
`guatiao-kind` and import the kinds from it.

```ts
// before: greeter_kind.ts, written by hand
export const Greeter = kind('greeter', { floorHash: 2695856491, methods: { /* ... */ } });
export type Greeter = Implementation<typeof Greeter>;
```

```bash
npx guatiao-kind greeter_kind.kinds.json --out greeter_kind
```

```ts
import { Greeter } from './greeter_kind.js';   // the value, and the interface
export class Hello extends Provider implements Greeter { /* unchanged */ }
```

`Remote<typeof Greeter>` is the generated `GreeterRemote`. A slot's name
is its C name in camelCase (`greet_into` is `greetInto`), as the
hand-written descriptions in this repository already spelled them.

## Sources

### The source crates are `guatiao-source-<kind>`, and manifests are `<name>.guatiao.toml`

No release carried the source crates or their manifests: this is for a
checkout, or a pin on a git revision, since `0.0.0-alpha.1`.

| before | now |
| --- | --- |
| `crates/guatiao-py`, `guatiao_py.dll`, `libguatiao_py.so` | `crates/guatiao-source-py`, `guatiao_source_py.dll`, `libguatiao_source_py.so` |
| `crates/guatiao-js`, `guatiao_js` | `crates/guatiao-source-js`, `guatiao_source_js` |
| `crates/guatiao-lua`, `guatiao_lua` | `crates/guatiao-source-lua`, `guatiao_source_lua` |
| `crates/guatiao-wasm`, `guatiao_wasm` | `crates/guatiao-source-wasm`, `guatiao_source_wasm` |
| `crates/guatiao-rhai`, `guatiao_rhai` | `crates/guatiao-source-rhai`, `guatiao_source_rhai` |
| `guatiao-remote`, `use guatiao_remote::..` | `guatiao-source-remote`, `use guatiao_source_remote::..` |

guatiao-source-remote's listeners admit peers by an access control list
since then: `TcpListen::bind(addr, &[Range::parse("10.0.0.0/8")?])`
becomes `TcpListen::bind(addr, acl)` with an `acl::Acl` permitting that
source (`acl.push(Rule::new(Action::Permit, "10.0.0.0/8".parse()?))`),
and the same for `WsListen` and `RdmaListen`; `from_listener` of the TCP
and WebSocket listeners no longer returns a `Result`.
`guatiao-remote-server --range` is unchanged.

The server is still `guatiao-remote-server`, the guest crate still
`guatiao-wasm-guest`, and the environment variables naming a build
(`GUATIAO_PY_BRIDGE`, `GUATIAO_WASM_SOURCE`, ...) are unchanged.

A manifest's file name no longer says which source reads it; its `type`
does, and a file named the old way is not read:

```toml
# before: greeters.guatiao-py.toml
module = "greeters"
kinds = ["greeter"]
```

```toml
# now: greeters.guatiao.toml
type = "python"
module = "greeters"
kinds = ["greeter"]
```

The types are `python`, `js`, `lua`, `wasm` and `rhai`
([the format](encodings.md#a-library-manifest)). A bridge on
`guatiao-bridge` names its type, `ManifestFormat { serves: "toy", .. }`,
where it named `suffix: ".guatiao-toy.toml"`, and `manifest::read`
answers `Option<Found>`: `None` for another source's manifest.

## Needs nothing from you

- **A required method may follow one with a default body** in
  `#[guatiao::kind]`, and a required slot an optional one in every
  binding's declaration: a kind declared required-first is described,
  hashed and laid out as before.
- **Loosened, never tightened**: the `Kind` bound is off `Remote`,
  `Instance`, `Offer` and `Object` and their `Send`/`Sync` impls; the
  `ProviderInfo` doors take `&self` on any lifetime; `ToValue`,
  `FromValue` and `Schema` for `BTreeMap` and `HashMap` take any `MapKey`
  key and any hasher; `GuatiaoError(status=None)` in Python.
- **`MergeMode::merge_layers` and `merge_layers_with` dropped an unused
  lifetime parameter** (`<'a, 'l>` is `<'l>`). No argument named it, so no
  call spelled it.
- **New variants of `#[non_exhaustive]` enums**: `UnloadError::Busy`,
  `KindMismatch::{Misaligned, Unreadable, ObjectKind}`,
  `ValueError::{DuplicateKey, Unrepresentable, Unparsed}`,
  `Skipped::Source`, `PathError::MissingSeparator`, `Error::Syntax` in
  `guatiao-serde`. Each enum was already `#[non_exhaustive]`, so a `match`
  had its `_` arm.
- **A table's `&self` slots of an object kind take `&`**: slot signatures
  and hashes are unchanged.
- **Moved Python names stay importable**: `guatiao.kinds.table` and
  `FloorMismatch` are re-exported from where they now live.
- **A secret typed as text is masked in a created form** (`widget:
  "password"`), and **`ParseError`** from `serde.loads` is a `BadValue`, so
  an existing `except BadValue` / `on BadValue` still catches it.
- **The derives reach `alloc` through `guatiao`**, so the crate that
  expands them declares nothing; and `#[derive(Form)]` reaches `guatiao`
  through `guatiao-intake` alone.
- **The Dart package's generated bindings follow the header's
  `guatiao_status_code`**; no name the package exports changed.
- **Fixes that change no correct program**: merging and its provenance,
  the scan, `is_visible` and `unflatten` each stopped answering wrongly in
  a case the changelog's "Fixed" sections name.
