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

### Changed

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
  should not. The C and Python/Dart artifacts are built `--all-features` and
  keep exact numbers.
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
  longer shares a key with the member `b` of a field `a`. A key written the
  old way still resolves to the field. *Migration:* a front end that
  builds store keys itself asks `flat::keys` for them.

### Fixed

- **`guatiao-intake`: `is_visible` no longer overflows the stack on a long
  `visibleWhen` chain, and `check` no longer takes cubic time on one.** The
  chain is walked in a loop (about 3000 links overflowed the stack of a
  release build, which `catch_unwind` cannot catch, and `check` accepts such
  a form), and cycles are found in one pass over the conditions instead of
  one walk per field.
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
