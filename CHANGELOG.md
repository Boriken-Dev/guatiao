# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **`Skipped` implements `Display`**: one clause to print after the file's
  name, so every host reports a skip in the same words.
- **`scan_path_with`**: a search-path scan under a closure over what each
  library declares, as `scan_dir_with` is for one directory. A host with
  several kinds can say "any of mine", which a rule set cannot.
- **An `Instance<dyn K>` implements `K`.** `#[guatiao::kind]` writes the
  impl and a `From` for `Box<dyn K>`, so an instance built from a
  configuration is stored as `Arc<dyn K>` or `Box<dyn K>` like any other
  implementation and released when its last owner goes. A host holding
  "a provider, configured or not" no longer wraps the two by hand.
- **`#[map(default)]` and `#[map(default = <expr>)]`** on a field: an
  absent key reads as the type's `Default` or the expression, the field
  leaves the schema's `required` list, and the schema's `default` is the
  same value. A setting with a default is then a plain `T` rather than an
  `Option<T>` resolved by hand, and a document the schema accepts is one
  the type reads. `#[schema(default = ..)]` is unchanged: an annotation
  on a field that stays required.
- **`#[map(other)]`: a unit enum can keep a name it does not know.** One
  variant of the shape `Other(String)` holds any text no other variant is
  stored as, and writes it back, so a model crossing a boundary does not
  fail to decode because the other side knows a newer name. Its schema is
  the new `KindBuilder::open_enumeration`: a string with the known names
  as `examples`, read with `FieldRef::suggestions()`.
- **`crate = <path>` on the derives and on `#[kind]`**: `#[map(crate =
  ..)]`, `#[provider(.., crate = ..)]` and `#[kind(crate = ..)]` root the
  generated code at a re-export of `guatiao` instead of at `::guatiao`.
  A crate that defines kinds can `pub use guatiao;` and be the one thing
  its providers depend on, rather than every provider pinning `guatiao`
  at the same version beside it.
- **`guatiao_json_parse_ex`, `guatiao_toml_parse_ex` and
  `guatiao_yaml_parse_ex`**: the `_parse` calls with one more, nullable,
  `guatiao_parse_error *`. Its `format` (`GUATIAO_FORMAT_JSON`, `_TOML`,
  `_YAML`) is the call's on every return; a document that does not parse,
  or text that is not UTF-8, also fills the line and column and a
  `message` string value that never quotes the document. A C, Python or
  Dart caller can show where a configuration file is wrong instead of a
  bare `GUATIAO_ERR_BAD_VALUE`.

### Changed

- **Breaking: no safe function reads a descriptor safe code could have
  forged.** The registry reads every pointer in a descriptor and calls its
  slots from safe code, so the promise that they are real is now made in
  an `unsafe` constructor, once:
  - `Registry::register_local` takes `fn(Host) -> Option<LocalLibrary>`.
    Migration: `local_providers!` writes the new signature, so a host
    using it changes nothing; a hand-written describe returns
    `Some(unsafe { LocalLibrary::new(info) })`.
  - `Registry::register_entry` takes an `EntryPoint`. Migration:
    `register_entry(name, unsafe { EntryPoint::new(entry) })`. The C
    `guatiao_registry_register_entry` is unchanged.
  - `ProviderInfo::{view, as_kind, offer, instantiate}` are `unsafe fn`.
    Migration: wrap the call in `unsafe` with the descriptor's contract;
    inside a library, `Host::get`/`list`/`offers` are the safe door.
  - A hand-built provider's descriptor comes from the `unsafe fn`
    `ProviderPartsBuilder::build` (see `ProviderParts::builder` below),
    and `Object::from_cell` is `unsafe fn` too (the derives write the
    `unsafe`). Migration: a hand-built provider wraps the call.
- **Breaking: `ProviderView`'s fields are private**, so a view can only
  come from reading a descriptor and its safe `available()` cannot be
  pointed at another `ctx`. Migration: `view.id` is `view.id()`, and so
  on for `kinds`, `display_name`, `config`, `meta`, `version`, `vtable`,
  `vtable_size`, `ctx`, `tables`, `raw`, `create` and `destroy`
  (`version()` is `Option<&str>`, `config()`/`meta()` are
  `Option<&_>`).
- **Breaking: `Host::get` and `Host::list` answer `ProviderView`s**, read
  under the size each descriptor declares, rather than `&'static
  ProviderInfo`: a reference to an older library's shorter descriptor
  claimed bytes it never wrote. `Host::mismatches` pairs a view with its
  mismatch. Migration: drop the `.view()` call; `view.raw()` is the
  descriptor's address.
- **Breaking: an `Offer` no longer derefs to its kind.** A provider built
  only from a configuration has no default instance, so calling it
  through its offer could only fail, and an infallible method panicked
  the host. `Offer::default_instance()` is `Option<&K>`, and `boxed()`,
  `shared()` and `remote()` answer `Option`, `None` for such a provider;
  `Provider::as_kind` and `ProviderInfo::as_kind` answer
  `Result<Option<Remote<K>>, KindMismatch>`, `Ok(None)` for it;
  `From<Offer<K>> for Remote<K>` is gone. Migration: `offer.greet(..)` is
  `offer.default_instance().expect(..).greet(..)`, or skip the offers
  answering `None` and `instantiate` them; `Remote::from(offer)` and
  `offer.remote()` are `offer.remote().expect(..)`, and
  `provider.as_kind()?` is `provider.as_kind()?.expect(..)`. A caller
  speaking C that wants the table of such a provider reads it with
  `table_for(kind)`.
- **Breaking: `instantiate` checks the configuration against the
  provider's schema first**, and answers `InstantiateError`:
  `Invalid(ValidationError)` names a misspelled or ill-typed setting
  before the provider is asked, `Refused(ProviderError)` is the
  provider's own answer. Applies to `Offer`, `Provider` and
  `ProviderInfo::instantiate`, and to the C
  `guatiao_registry_provider_create` (so the Python and Dart `create`),
  which answers `GUATIAO_ERR_BAD_VALUE` with the refusal in `err`.
  Migration: match the two variants, or call `.status()` where `.status`
  was read; a separate `validate_map` before `instantiate` is no longer
  needed.
- **Breaking: `LoadReport.loaded` is `Vec<(PathBuf, String)>`**, each
  loaded file with the library key the registry filed it under, so a host
  can tell which providers a scan added, or retire what it loaded,
  without counting. Migration: `for (path, key) in &report.loaded`; the C
  report is unchanged.
- **Breaking: `ProviderParts::builder(id)` replaces `ProviderParts::new`**
  and its ten positional arguments: `.name`, `.version`, `.kinds`,
  `.table`, `.config`, `.ctx`, `.available`, `.create`, `.destroy`, then
  the `unsafe` `.build()`. Every slot defaults to absent, so a provider
  written without the derive keeps compiling when the envelope appends
  one. Migration: `ProviderParts::new(id, name, version, kinds, tables,
  config, ctx, available, create, destroy)` is
  `ProviderParts::builder(id).name(name).version(version).kinds(kinds)`,
  a `.table(..)` per table, `.ctx(ctx)`, a call per slot that is not
  `None`, then `unsafe { .build() }`.
- **Breaking: `Status` is a `u32`, not an enum.** A library or host may
  return a code of its own, as `guatiao.h` allows, and reading one into a
  `repr(u32)` enum was undefined behaviour in the host. `Status` is now
  `#[repr(transparent)]` over `u32` with the same `GUATIAO_*` associated
  constants, `is_ok()`, `Display`, and `From` both ways; any code is a
  `Status`. The C names and values are unchanged; the header's constants
  now belong to `enum guatiao_status_code`, and `guatiao_status` is
  `typedef uint32_t` in C and C++ alike. *Migration:* a `match` on a
  `Status` gains a `_` arm; `status as u32` is `u32::from(status)`, and a
  code read from C is `Status::from(code)`.
- **Breaking: `Tag` is `#[non_exhaustive]`**, as its documentation already
  said kinds are appended. *Migration:* a `match` on a `Tag` outside this
  crate gains a `_` arm.
- **Breaking: the short `set`, `push` and `push_str` abort on an
  allocation failure, as `Vec::push` does, and return no `Result`.**
  `Map::set` and `List::push` return `()`, as do `Text::push_str` and
  `Buffer::push`; each grows through the allocator its container recorded,
  or Rust's own for one that recorded none (a literal), and a refusal goes
  to `alloc::alloc::handle_alloc_error`. The short constructors
  (`Text::new`, `Buffer::new`) abort the same way rather than panicking.
  The fallible paths stay: `try_set`, `try_push` and `try_push_str`
  (`Buffer::try_push`) report an `AllocError` through the recorded
  allocator, for a container built in an arena that may be full, and the
  `_in` forms are unchanged. `Status` converts from an `AllocError`.
  *Migration:* drop the `?`, `.unwrap()` or `let _ =` after
  `set`/`push`/`push_str`; where the container lives in an allocator that
  may refuse and the caller handles it, call `try_set`/`try_push`/
  `try_push_str` (or the `_in` form) instead.
- **Breaking: iterating a `&mut Map` yields `(&Text, &mut Value)`**, as
  `HashMap::iter_mut` does, through the new `Map::iter_mut` and its
  `IterMut`. It yielded `&mut Entry`, so safe code could overwrite a whole
  entry and give a map a repeated key that its own wire encoding then
  refused. *Migration:* `for entry in &mut map { *entry.value_mut() = v }`
  is `for (_key, value) in &mut map { *value = v }`.
- **`clone()` on `Value`, `Map`, `List`, `Text` and `Buffer` aborts on an
  allocation failure**, as the short forms do, and its documented panic is
  the one left: content this build refuses (an unknown tag, text that is
  not UTF-8, a number outside the grammar, nesting past `MAX_DEPTH`),
  which only a foreign producer can write. Its message points at
  `clone_in`, the form that reports it.
- **A scan reads a candidate on demand rather than whole**: headers,
  export and dynamic symbol tables, and the 4 KiB declaration window.
- **An object kind is never served by a provider.** A registry or host
  answers the new `KindMismatch::ObjectKind` for one, and
  `#[provider(..)]` naming one is a compile error on the kind: its shims
  read `ctx` as an object one caller owns, and a provider's is shared.
- **`guatiao-serde`'s C functions write `out` ABSENT on entry**, as the
  `guatiao` and `guatiao-intake` ones do. A failed call used to leave `out`
  as the caller had it; it now reads `GUATIAO_ABSENT`, and `out` must not
  hold a tree the caller still owns. The headers say so, and the
  `guatiao_intake_flatten`, `_unflatten` and `_flat_keys` docs now describe
  lists, open maps and "nothing stored" and not only variants.
- **A `std::` path now marks something that needs the operating system.**
  Everything `core` or `alloc` also holds is spelled that way, enforced by
  the `std_instead_of_core`, `std_instead_of_alloc` and
  `alloc_instead_of_core` clippy lints for the whole workspace. Error types
  implement `core::error::Error`, and the derives and `providers!` reach
  `alloc` items through a hidden `guatiao::__private::alloc`, so the crate
  that expands them declares nothing.
- **`guatiao-serde`: reading a JSON number exactly is the opt-in
  `exact-numbers` feature, and `json` no longer turns on
  `serde_json/arbitrary_precision`.** With the default feature set on,
  every crate in the build that deserialised `#[serde(untagged)]` or
  `#[serde(flatten)]` numbers through `serde_json` stopped working, because
  cargo unifies the feature. Without `exact-numbers`, an integer that fits
  `i64`/`u64` is still exact, anything else reads through an `f64` (`1.10`
  as `1.1`) and `1e400` is refused by the parser; writing a number
  verbatim through `text::json` is unchanged. *Migration:* a binary that
  relies on exact reading enables `guatiao-serde/exact-numbers`; a library
  should not. A C host, and the Python and Dart bindings, which ship no
  library of their own, read numbers exactly when the `guatiao_serde`
  library they load was built with `exact-numbers`; a workspace build with
  `--all-features` is.
- **`guatiao-serde`: a document that does not parse is `Error::Syntax`, with
  a `Position`, and its message never quotes the document.** A parse
  refusal used to be `Error::Format` with the parser's whole `Display`,
  which for TOML shows the offending line and for YAML up to three lines
  around it. `Error::Syntax { format, position, detail }` holds the
  parser's message and where it stopped (`Error::position()`, a new
  `Position { line, column }`, both from 1 and the column counted in
  characters in every format) and nothing from the text.
  `Format` is now only a writer refusing a value. *Migration:* match
  `Error::Syntax` where you matched `Error::Format` on a `from_str`, and
  read the position instead of scraping the message.
- **`guatiao-intake`: the path grammar is stricter.** `a[0]b`, `a["x"]y`,
  `a.[0]` and `[a[b]` are refused (`PathError::MissingSeparator` is new, and
  `EmptyField` and `UnclosedIndex` now cover the others), and digits in a
  bracket are a position only without a leading zero: `a[01]` is the key
  `01` and `a[007]` the key `007`, no longer the positions 1 and 7.
  `path::needs_quoting` follows. *Migration:* write a key that starts with
  `0` as itself (`env[01]`) and a position as plain digits; a text that
  relied on the old reading now fails to parse and names its offset.
- **`guatiao-intake`: `flat::split` reads the key with the path grammar.**
  It returns `Option<(&str, Cow<str>)>` (it was `Option<(&str, &str)>`),
  splits before the **last** segment, and agrees with `flatten` and `keys`
  for a member whose name needs a bracket. *Migration:* compare or borrow
  the member through the `Cow` (`&*member`, `member == "x"`).
- **`guatiao-intake`: a top-level field whose key holds a `.`, a bracket or
  nothing is stored under a bracket key** (`a.b` as `[a.b]`), so it no
  longer shares a key with the member `b` of a field `a`. A store written
  the old way still reads back: `unflatten`, `resolve`, `resolve_in` and
  `validate_texts` take a top-level field under its bare name (`a.b`,
  `x.y[k]`) and an open map's key in any spelling the grammar reads
  (`env["01"]` as well as `env[01]`); where both spellings are present the
  new one wins. *Migration:* a front end that builds store keys itself
  asks `flat::keys` for them.

### Fixed

- **Every `guatiao` C function with an out-parameter writes it first.**
  The value constructors (`guatiao_value_null` to `_bytes`) and the
  registry's answers (`guatiao_registry_load_file`, `_register_entry`,
  `_scan_dir`, `_scan_dir_rules`, `_scan_path`, `_libraries`,
  `_providers`, `_provider`, `_available`, `_why_not`, `_best`) left `out`
  as the caller had it on failure, so a caller freeing it afterwards freed
  garbage; they now write it ABSENT on entry, as the header's new preamble
  says of every such function. `guatiao_registry_provider_create` writes
  `out` null and `err` an empty error first.
- **A refused `guatiao_map_set` or `guatiao_list_push` leaves the caller's
  value alone.** An allocation failure used to answer `GUATIAO_ERR_ALLOC`
  after the value had been moved in and freed.
- **Statuses say what was wrong.** `guatiao_map_discard` and
  `guatiao_list_discard` on the wrong kind answer `GUATIAO_ERR_WRONG_KIND`
  (they said `NOT_FOUND`); a string or map key that is not UTF-8 is
  `GUATIAO_ERR_BAD_VALUE` (it was `WRONG_KIND`), and in Rust
  `MapError::BadValue` (it was a `WrongType` naming the kind it was); a
  registry function given text that is not UTF-8 answers
  `GUATIAO_ERR_BAD_VALUE`, and one given an unusable allocator
  `GUATIAO_ERR_ALLOC` (both were `GUATIAO_ERR_NULL`).
- **Freeing a literal whose arrays are in read-only memory no longer
  faults.** The free walk moved each element out of a list or map by
  writing a null back into the array and released each key in place, so a
  `GUATIAO_VALUE_LIST_LIT` or `_MAP_LIT` over a `const` array crashed
  `guatiao_value_free` (and a `Value`'s drop), though `guatiao.h` says such
  a literal may be freed. Elements are now read out and nothing is written
  into a literal's storage.
- **Dropping a `Registry` leaves its libraries mapped.** It used to close
  every library it had loaded, while a `Remote`, an `Offer`, a boxed
  trait or an `Instance` taken from it is `'static` and could still be
  called, which jumped into unmapped memory from safe code. A library is
  unmapped only by `unload`, as the documentation already said;
  `guatiao_registry_free` and a Python `Registry.close()` go through the
  same drop. A file passed over by its library key, or refused as a
  duplicate, after its entry point ran keeps its mapping too.
- **`unload` unmaps a library that a hard link to it was passed over
  for.** On Linux the loader answers a hard link to a loaded file with
  the image it already mapped and counts a second reference; the registry
  kept that handle, so `unload` reported success while the image stayed
  mapped. A skipped file whose handle names an image the registry already
  holds is now closed, which only gives that reference back.
- **A scan reads hostile files as data, and answers what it read.** A file
  that only imports the entry symbol is not a library (an undefined
  dynamic symbol was taken for an export); an export table that cannot be
  read is `Skipped::NotExaminable` rather than `NoEntrySymbol`; and a
  universal Mach-O is probed through the slice for the running
  architecture instead of being refused, so a universal plugin loads on
  macOS.
- **A descriptor's array lengths are walked, never reserved for.** A
  corrupt `len` aborted the process allocating for it before a single
  element was read.
- **`providers!(.., unload = <expr>)` gives the expression no `unsafe`.**
  It was evaluated inside the macro's own `unsafe` block, so a raw
  pointer dereference there compiled with none written; it is now bound
  to a typed `let` first, and the slot the macro writes is a safe
  `extern "C" fn`. The macros also name `::core::env!` and
  `::core::option::Option::Some`, and `#[derive(Provider)]` names
  `::core::env!` for the id it gives a provider that names none, so a
  caller's own `env!` or `Some` cannot change their expansion.
- **A panic in a configured instance's `Drop` no longer aborts the host.**
  The `destroy` slot `#[derive(Provider)]` writes runs under the same
  catch as every other shim.
- **A descriptor built by `ProviderParts`, `LibraryParts` or a registry's
  host block keeps its pointers valid when its owner moves.** The text
  and the schema it points into were held in `Box`es, which a move
  retags as unique and Stacked Borrows then treats as invalidating every
  pointer taken earlier; they are `String`s and `Vec`s now.
- **The host block a registry leaks is written through one raw pointer**,
  so the services' `ctx` it holds stays valid under Stacked Borrows.
- **`guatiao-serde`: TOML is read in document order, and a TOML date or time
  is its text.** Reading sorted every table's keys, unlike JSON and YAML
  (`toml/preserve_order` is on now). An offset date-time, local date-time,
  local date or local time read as a map under toml's private key
  `$__toml_private_datetime` and were written back as a table; each now
  reads as the text it was written as. A YAML `!!binary` scalar read as the
  base64 payload decoded as UTF-8, or failed when it was not; it now reads as
  its base64 text.
- **`guatiao-intake`: `is_visible` no longer overflows the stack on a long
  `visibleWhen` chain, and `check` no longer takes cubic time on one.** The
  chain is walked in a loop (about 3000 links overflowed the stack of a
  release build, which `catch_unwind` cannot catch, and `check` accepts such
  a form), and cycles are found in one pass over the conditions instead of
  one walk per field.
- **`guatiao-intake`: a `visibleWhen` naming a field by its store spelling
  is read.** `check` accepted `[a.b]` or `["a.b"]` for the field `a.b`,
  and `is_visible` then never found it, so the field waiting on it was
  never shown; that spelling now names the field there too, its own
  condition and any cycle through it included.
- **`guatiao-intake`: `unflatten` no longer loses a list or an open map
  when a sibling key sorts between the field name and its `[`.** A store
  holding `agent[0]` beside `agentName` or `env[PATH]` beside `env-file`
  read back as nothing, while `validate_texts` called it fine. `flatten`
  also stops at `MAX_DEPTH` (answering `false`) as `unflatten` already did,
  and `flat::keys` lists what `flatten` writes for a member with a `.` in
  its name.

- **A handle held across an `.await` no longer makes the future `!Send`.**
  `Remote`, `Instance` and `Object` were `Send` only for a kind that is
  `'static`, which a future cannot show, so `tokio::spawn` refused any task
  holding one with "implementation of `Kind` is not general enough". The
  `Kind` bound is off those types and their marker impls, and `Offer` and
  `ProviderView` are `Send + Sync` as well.

## [0.0.0-alpha.0] - 2026-09-27

First release. An alpha: the API can change between releases, and this
changelog says what did and what to do about it.

### Added

- **`guatiao`** — values as plain C structs: null, bool, number (kept as its
  exact text), string, bytes, list and map, each container carrying the
  allocator that made it, so a tree built in one library frees in another.
  `#[derive(ToValue, FromValue, Schema)]` behind `derive`; merging layered
  values; a byte format and framed channel (`value::wire`) for moving values
  between processes.
- **A schema is a JSON Schema 2020-12 document**, held as an ordinary value:
  builders, a reader and `validate_*`. Text, integers and reals with bounds
  and steps, bytes, lists, objects with declared or open keys, choices,
  unions and tagged variants; `minLength`/`maxLength`, `pattern` (enforced
  with the `regex` feature), `format`, `multipleOf`, `minItems`/`maxItems`,
  `readOnly`, `deprecated`, `examples`, and `x-sensitive` for a secret.
- **A library envelope**: one entry symbol through which a shared library
  offers any number of providers of a kind (`#[guatiao::kind]`,
  `#[derive(Provider)]`), and a `Registry` a host uses to scan for, load and
  retire them (`load` feature).
- **`guatiao-serde`** — one `Serialize`/`DeserializeSeed` pair for every serde
  format, with `text::json`, `text::toml` and `text::yaml`; numbers stay exact
  through JSON.
- **`guatiao-intake`** — how a schema is shown: sections, widget, placeholder
  and unit hints, conditions (`equals`, or `in` for several values),
  sub-forms, and forms written with `#[derive(Form)]` or created from the
  schema alone; a jq-shaped path grammar (`agent[1].name`) and a flat
  `key -> text` projection.
- **A C ABI** for all of it: `guatiao.h`, `guatiao_serde.h`,
  `guatiao_intake.h`.
- **Python** (3.9+) and **Dart** (SDK 3.6+) bindings over that ABI: pure
  `ctypes` and `dart:ffi`, nothing to compile — point them at the library.

Published as `guatiao`, `guatiao-derive`, `guatiao-serde` and
`guatiao-intake` on crates.io, `guatiao` 0.0.0a0 on PyPI, and `guatiao` on
pub.dev.

[Unreleased]: https://github.com/Boriken-Dev/guatiao/compare/v0.0.0-alpha.0...HEAD
[0.0.0-alpha.0]: https://github.com/Boriken-Dev/guatiao/releases/tag/v0.0.0-alpha.0
