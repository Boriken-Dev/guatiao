# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Dart bindings**, `bindings/dart/`: `dart:ffi` over the same C ABI,
  with `package:ffi` as the only dependency and no Flutter dependency, so
  one package serves a Flutter app and a command-line program. `Value`,
  `Ref`, `MapRef` and `ListRef` for the value tree; `Registry` and
  `Instance` for hosting plugins; `kindTable` for a provider's function
  table, checked against the floor hash the kind's C header declares;
  `package:guatiao/serde.dart` and `.../form.dart` for the two optional
  libraries. `lib/src/bindings.g.dart` is ffigen output from the three
  committed headers. CI runs `dart analyze`, `dart format` and `dart
  test` on Linux, Windows and macOS, and the docs site carries the
  generated reference under `/dart/`.
- **A host registers a library it links through its C entry point**:
  `Registry::register_entry(name, entry)` beside `register_local`, and
  `guatiao_registry_register_entry` in C -- for a library written in C
  and linked into a Rust host, or a host written in C registering
  anything it links. The same absorb, keys, dedup and `<name>` path.
  Safe to call, as `load_file` is: handing over an entry point is
  choosing to run it. `library::EntryFn` names the entry point's type.
- **Object kinds**: `#[guatiao::kind(object)]` declares a kind whose
  instances are handles one caller owns -- a session, a scan, a stream
  -- rather than providers a registry offers. The trait names `Send`,
  its methods may take `&mut self`, a `&mut [u8]` argument crosses as an
  out-buffer (`library::BytesMut`), and the table carries a `destroy`
  slot after its header. The attribute appends `into_object(self) ->
  Object<dyn Trait>` to the trait; `library::Object<K>` is the handle
  (`Deref`/`DerefMut` to the trait, `destroy` on drop, `into_raw` /
  `from_raw` as `library::ObjectRaw`). Any kind's method may return an
  `Object<dyn K>` or take one as an argument: ownership crosses with the
  call, so a host implementing an object kind and handing it in is how a
  callback crosses. A shim takes its object arguments before anything
  else can fail, so a refused call never leaks what it was handed.
  `Kind` gains `OBJECT` and `as_dyn_mut`; the floor hash now covers the
  shape (`provider;` or `object;`) as well as the signatures, so every
  existing table's hash changes. In C: `guatiao_object`,
  `guatiao_bytes_mut`. `examples/greeter_kind` declares `Conversation`
  and `Listener`, `derived_greeter` starts one, and `greeter_host` drives
  it and hands the listener in.
- **A kind's table renders to C.** `<Trait>Vtable::FLOOR_HASH` is a
  literal the attribute computes at expansion time, so cbindgen renders
  it as `#define <table>_FLOOR_HASH n` -- the number a C implementation
  writes into its table's header. `examples/greeter_kind` renders
  `include/greeter_kind.h` from its own `build.rs` through cbindgen's
  macro expansion (`parse.expand`, `RUSTC_BOOTSTRAP=1` and
  `CARGO_EXPAND_TARGET_DIR` set around the render, every table named in
  `export.include`, guatiao's types renamed onto `guatiao.h`'s) and
  commits it; a test compares the two. `tests/c_consumer/greeter_in_c.c`
  is a greeter written in C against that header, offered through the
  envelope and linking nothing; `tests/c_library.rs` compiles it into a
  shared library, loads it, and calls it as `dyn Greeter` -- the slots it
  leaves null run the trait's default bodies on the host.
- `Registry::register_local(name, describe)`: a library the host LINKS
  rather than loads -- its providers compiled into the host, `describe`
  what its entry point would have called -- goes through the same
  absorb as a loaded file: the same keys, dedup, refusals and `Loaded`
  record, from `<name>` instead of a path. `guatiao::local_providers!`
  writes the `library` function to hand it, and nothing else: no entry
  symbol and no declaration, so the host's own binary never looks like
  a plugin to a scan. The host example carries a built-in greeter this
  way, offered beside the ones it loads.
- `guatiao-form` says `links = "guatiao-form"` and publishes its header
  path as `DEP_GUATIAO_FORM_INCLUDE`, as `guatiao` does.
- `examples/greeter_host`: the host side as a program. It scans a search
  path under a kind rule, prints the report, offers every `dyn Greeter`
  it found, calls the one that is its own instance through both of its
  kinds, and builds the configured one from a struct the host declared
  with `#[derive(ToValue, Schema)]`, checked against the provider's
  schema with `validate_map` before `instantiate`. Run by `cargo test`.
- `Declared::parse` is public, so a host can check its scan rules
  against a declaration written by hand.
- `Registry` and `Provider` are `Send` and `Sync`: a host keeps its one
  registry behind a lock and reads it from any thread. Every pointer they
  hold addresses a library's image, which is never unloaded.
- `library::SearchPath` and `scan_path`: a host's search path is a list
  of directories or files on the platform's own separator, each place
  visited once; a file entry is probed and loaded on its own under the
  same rules; a `.framework` bundle is its binary (`bundle_binary`); an
  entry that cannot be read goes under the new `LoadReport::unreadable`
  and the rest of the path is still walked. From C,
  `guatiao_registry_scan_path`.
- `library::notices`: legal notices as a convention on `meta` --
  `notices` is a list of `{component, text, format}` maps, read with
  `notices(meta)` and written with `declare_notice`. The format is
  declared, never sniffed, and anything but `markdown` reads as text.
- `flat::resolve_in` resolves a flat key in a particular store: a payload
  key is checked against the arm the store selects, or the field's
  default. `validate_texts` goes through it, so a field belonging to an
  unselected arm is refused in the text form as it already was in the
  nested one.
- `SchemaRef::extras` and `FieldRef::extras` enumerate every annotation
  -- each key the vocabulary does not claim, with its value -- so a
  consumer keeping a mirror of a field copies them across without
  naming each one.
- `Value::into_map` and `into_list` take the container out of a value by
  value, handing back a value of another kind untouched.
- **`Value`, `Map`, `List`, `Text` and `Buffer` are `Clone`, `PartialEq`,
  `Send` and `Sync`.** A clone is a deep copy through the allocator the
  source recorded; equality is structural. The allocator contract gained
  the line that makes the last two sound: an `Allocator` may be called
  from any thread.
- `Cargo.toml` says `links = "guatiao"` and the build script publishes
  the committed header's directory as `DEP_GUATIAO_INCLUDE`, so a
  consumer whose own header `#include`s `guatiao.h` copies it from there.
- Workspace scaffold: the `guatiao` crate, its `guatiao-derive`
  proc-macro companion and the `guatiao-serde` format crate; feature flags
  `derive`, `c-header` and `load`; and a test workflow over Linux, macOS and
  Windows on stable and on the minimum supported Rust version, **1.89**.
- **The C ABI is always compiled.** A plain `cargo build` produces a cdylib
  whose export table carries every `guatiao_*` symbol; `c-header` only
  renders the committed `include/guatiao.h`, and a default build of
  `guatiao` still pulls in no dependency at all.
- **The value model, as plain C structs.** Null, bool, number, string,
  bytes, list and map; one node type; `{ptr, len}` views and
  `{ptr, len, cap}` owned containers, each carrying the allocator that
  made it. A number is stored as the exact text that declared it, so
  `1.10` reads back as `1.10` and a 200-digit integer survives; map
  iteration is insertion order; nothing coerces between kinds. A C, C++ or
  Dart consumer reads a whole tree through the committed header with no
  call into any library.
- **Building from Rust names no allocator.** `Map::new()` hands back a
  `Map` you grow with `set`; `map.into()` makes it a value at the one
  point something wants one. `map.set("port", 5900)?` is the whole call,
  because `set` takes anything a value can be made from. A container that
  must live in a host's arena says so with the `_in` constructors, which
  stay fallible because a foreign allocator refusing is recoverable.
- **A value owns its tree** and frees it on drop, like any other Rust
  value; handing one to something else is a move. A value crossing a
  boundary is forgotten, and the far side frees it with
  `guatiao_value_free` — the same walk, which is iterative because a tree
  may have arrived from a caller this crate cannot see.
- **Reading from Rust**: `ReadValue` for a lookup that composes, and
  `TryFrom<&Value>` for every scalar, so `try_into` is the conversion.
- **The schema**, which describes a value so a consumer that did not write
  it can understand it: what a provider needs to be configured, or a record,
  a set of capabilities or metadata passed between libraries. It is an
  ordinary value, **and that value is a JSON Schema
  (2020-12)**: `properties`, `required`, `type`, `enum`, `oneOf` and the
  rest, spelled the specification's way, so a schema written out as text is
  a document existing JSON Schema tools already read. What this crate adds
  is `x-` prefixed. Two things are deliberately not the specification's:
  `type: "bytes"`, which a strict meta-schema check refuses, and
  `x-variant-tag`, because JSON Schema has no discriminator keyword.
  A third-party 2020-12 validator is part of the test suite and accepts
  what the builders write.
- **Building a schema** reads as a declaration: `SchemaBuilder`,
  `FieldBuilder`, `KindBuilder` and `ArmBuilder` for what a value *is*, and
  the `FormBuilder` / `FormFieldBuilder` traits for how it is *shown* —
  label, help, section, order, advanced, sensitive. Presentation is
  optional and never decides validity. `option(key, value)` sets any key,
  including a vendor's own annotation. Readers, validation that never quotes
  the value it refused, and a flat `key -> text` projection for a front end
  that only has text — reachable from C as `guatiao_schema_validate`,
  `_resolve`, `_flat_keys`, `_flatten` and `_unflatten`.
- **`export_schema!`**: a library exports a described type's schema as a C
  symbol without writing the wrapper. The symbol carries the calling
  crate's name, and `version = major | minor | patch` keeps two builds of
  one library apart in one process.
- **Configuration layering**: three merge strategies, per-path overrides a
  schema can declare through `x-merge`, and provenance recorded per leaf
  path.
- **Three derives**: `ToValue`, `FromValue` and `Schema`, which read one
  declaration so a type cannot describe a value it refuses. For a struct
  with named fields, an enum of unit variants (stored as the variant's
  name, described as a choice), and an enum naming its tag with
  `#[map(tag = "...")]` (stored as a map, described as a tagged variant).
  An enum whose variants carry fields and names no tag is refused, because
  the key that tells variants apart is yours to choose.
- **Libraries and providers.** A library exports one entry symbol and
  answers with descriptors that borrow from its own image; every descriptor
  leads with `struct_size`, so a field appended later reads as absent from
  a library built before it, and an array states its element stride. A
  library is never unloaded.
  - **Finding one maps nothing that is not one**: `scan_dir` reads each
    file's export table as data (PE, ELF or Mach-O), and only a file
    declaring the entry symbol is loaded, because mapping runs a library's
    static initialisers.
  - **A library declares what it is, and a scan filters on that before
    mapping it.** The data symbol `guatiao_declares` holds `key=value`
    pairs (`kind=<name>` for every kind, plus the library's own, such as
    `VIEWER=1`); `guatiao::providers!` writes it from the types,
    `guatiao::declares!(..)` writes it for a hand-written library, and
    `probe(path)` reads it as data, clamped to its section and 4 KiB.
    `scan_dir_rules(reg, dir, order, &ScanRules::parse(&["!VIEWER=1",
    "kind=session-backend"])?)` keeps a file out as `Skipped::Filtered {
    by }` naming the rule; `scan_dir_with(.., filter)` takes a closure over
    `Declared` instead. From C, `guatiao_registry_scan_dir_rules` with
    newline-separated rules, reported as `{"skipped": "filtered", "by":
    ..}`. A library declaring nothing loads as before. `ProviderDecl`
    gained `const KINDS`, which the derive fills.
  - **The registry is the host's.** A repeat load is a skip that names
    where the first came from, not an error. What a library or a provider
    is filed under is a key template the host chooses (`%id` by default;
    `%id@%version` keeps two builds). A provider serves many kinds, and one
    may say whether it can run here and why not — asked live, never
    cached — so `why_not` tells "nothing offers this" apart from "something
    does and cannot run".
  - **Order is `(priority DESC, key ASC)`**, and priority is set by the
    host, not declared by a library.
- **A C or Python program can be the host**: `guatiao_registry_*` loads
  files and directories, lists libraries and providers, sets priorities,
  asks availability, and reaches a provider's table, context and schema.
  Answers come back as ordinary values the caller already knows how to
  walk, freed with `guatiao_value_free`.
- **`guatiao-serde`**: one `Serialize` and one `DeserializeSeed` over a
  value, so any serde format carries one — JSON, TOML and YAML are named
  features, and a format this crate has never heard of works too. A
  `Presentation` says how bytes are written (a data URI by default, bare
  base64, an array, or refused). With JSON a number keeps its **spelling**,
  not only its magnitude: `1.10` reads back as `1.10`. From C:
  `guatiao_json_parse` / `_emit` / `_emit_pretty`, and the same for TOML and
  YAML; emitting answers a value holding a string, freed like any other.
- **`guatiao-form`**: how a schema is shown, as a value beside it that
  never repeats it. A form names fields by path (`port`, `auth.username`)
  and adds only what no single field can say about itself: what a section
  is called and the order sections come in, which widget draws a field and
  what an empty one shows, and when a field is visible. `check` says
  whether a form fits its schema without ever quoting a value; `layout`
  groups and orders the fields; `is_visible` evaluates conditions, where a
  condition on a hidden field is not met. All three are exported to C in
  `include/guatiao_form.h`.
  - **`#[derive(Form)]`** (`guatiao-form`'s `derive` feature) writes a
    type's default screen beside `#[derive(Schema)]`: `#[form(section(id,
    label, help))]` on the type in display order, `#[form(widget,
    placeholder, visible_when(field, equals), nested)]` on a field, keys
    following `#[map(rename)]`. It implements the new `Screen` trait
    (`form(alloc)`, and `hints(prefix, form)` for composing a member's
    hints under `owner.`). `Form::alloc()` is the builder's allocator.
- The core crate carries no serialisation: how a value or a schema is
  written down is `guatiao-serde`'s job, or a consumer's own.
- Licensed under the Mozilla Public License 2.0. Using the library imposes
  nothing on your own code; modifying its files does. Contributions need a
  one-time contributor licence agreement.
- **A library keeps its host.** The entry point receives a `Host`
  (`Copy + Send + Sync + 'static`): a pointer to a block the registry
  leaks on first use, carrying the host's id, version, allocator and a
  `HostServices` table (`get`, `list`, `alloc`). The table answers from a
  snapshot the registry publishes after every change, so a library asks
  from any thread, a lookup from an entry point sees every earlier
  library, and a dropped registry answers `GUATIAO_ERR_GONE`. From C,
  `guatiao_registry_host` hands the same block to a host driving a
  library by hand. `hello_library_echo` reaches the greeter through it.
- **A kind is a trait.** With the `provider` feature, `#[guatiao::kind]`
  on a trait generates the `repr(C)` table, the shims and the proxy;
  `#[derive(Provider)] #[provider(Kind, ..)]` makes an implementation a
  provider; `guatiao::providers!(Type, ..)` is the whole library, with id and
  version from Cargo. A consumer gets every provider with a valid table as
  `Offer<dyn Kind>` — the trait itself, plus id, version, priority and a
  live `available()` — through `Registry::offers`, `offer` and
  `mismatches`, or `Host::offers` from inside a library; `Remote::from_raw`
  is the one `unsafe` door for a table from anywhere else. A provider
  carries one table per kind (`ProviderInfo::tables`); a hand-written
  `vtable` stays the untyped path and is never mistaken for a typed table.
  `examples/derived_greeter` is two `impl`s and one line.
- **A provider builds instances from a configuration.** `#[provider(config
  = C)]` means the type is built from `C` (`FromValue` then `TryFrom<C>`);
  the host reads the schema, fills it in, and `offer.instantiate(&config)`
  hands back an `Instance<dyn Kind>` whose address is the context every
  call on it takes, released on drop. Many instances per provider, each
  its own configuration; a provider without `config` is its one instance.
  Two envelope slots, `ProviderInfo::create` and `destroy`, and from C
  `guatiao_registry_provider_create` / `_destroy`.
- `Text`, `Buffer`, `Entry`, `Tag`, `Str`, `MAX_DEPTH` at the crate root;
  `Default` for `Text` and `Buffer`; `Bytes::new`/`empty`;
  `Entry::value_mut`; `Schema for f32`; `Registry::all_ranked`;
  `Provider::table_for`; `Skipped::UnsupportedAbi`; the schema keys as
  `GUATIAO_KEY_*` macros in the header.

### Changed

- **Breaking: the fields of `Value`, `Payload`, `Entry`, `Text`, `Buffer`,
  `List` and `Map` are private.** Safe code can no longer forge a node,
  write a length, or copy a container into a second owner. The one door
  for a literal or a buffer another language owns is `unsafe fn
  from_raw_parts` on each container and on `Value` (with
  `Payload::text/bytes/list/map/bool`); `into_raw_parts` is the safe
  inverse, `Entry::new`/`into_parts` and `capacity()` on the containers
  are the safe conveniences. The views keep public fields; the C header
  is unchanged.
- **Every object the schema builders seal carries `additionalProperties:
  false`** — a struct's document and each arm's subschema — so a general
  JSON Schema validator refuses exactly what `validate` refuses. The key
  reaches C as `GUATIAO_KEY_ADDITIONAL_PROPERTIES`.
- `#[provider(Greeter, config)]` (or `config = Self`) makes a type that
  derives `Schema` and `FromValue` its own configuration, with no second
  type and no `TryFrom`; `examples/derived_greeter`'s `Shouter` is written
  that way.
- **Breaking**: `describe` takes `Host` instead of `&HostInfo`;
  `HostInfo` gained `services` and `ProviderInfo` gained `tables`, both
  appended under `struct_size`; `ProviderView` gained `raw` and `tables`.
  `Provider::key_str` is gone. `guatiao-form`'s `is_visible` returns
  `Result<bool, FormError>` and refuses a path no field declares. The
  value model's arm accessors (`as_text_mut` and kin) are crate-private.
  The header's `MAX_DEPTH` macro is `GUATIAO_MAX_DEPTH`. A null or
  malformed allocator answers `GUATIAO_ERR_ALLOC` from every export.
- `Value::clone_in` is safe. Every export with an out-pointer writes
  `absent` through it at entry, so a failed call leaves nothing
  uninitialised.
- A provider is deduplicated by its rendered key, not its id, so
  `%id@%version` holds two builds of one provider.
- `guatiao-serde`'s header gates the TOML and YAML declarations behind
  `GUATIAO_SERDE_TOML` / `GUATIAO_SERDE_YAML`, since a default build
  exports neither; its docs say `arbitrary_precision` is on with `json`
  and that a number is verbatim only under `Numbers::RawText`.

### Fixed

- `guatiao_map_copy_from` with the source inside the target was a
  use-after-free; the source is now copied whole first, and the exports
  refuse identical pointers. A push from a view of the node's own text
  copied from freed memory. `equal` called any two non-UTF-8 strings
  equal. Validation and deserialisation had no depth bound.
  `Schema::schema()` dropped every key of a non-object kind. An `x-merge`
  on a nested field was ignored.
- The loader reported a library that declined the host, a malformed
  descriptor and a library speaking another envelope version all as "not
  a library"; each is now its own answer, and the library's `abi_version`
  is checked. The C `guatiao_registry_providers` listing was not ranked.
  A corrupt provider array could be allocated for before it was checked.
- `guatiao-derive` ignored container attributes on a struct;
  `guatiao-serde` ignored `GUATIAO_PRETTY` and wrote a malformed node as
  zero; `guatiao-form` answered "visible" for an unknown path and read an
  arm's field whichever arm was picked. `guatiao-serde` and `guatiao-form`
  now ship their LICENSE and docs.rs metadata, and their generated headers
  carry the Exhibit A notice.

[Unreleased]: https://github.com/Boriken-Dev/guatiao/commits/main
