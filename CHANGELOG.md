# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

Moving from `0.0.0-alpha.0`: the docs site's
[migration page](https://boriken-dev.github.io/guatiao/migrating/) gathers
every change since then that asks something of code using it, with what to
write now.

### Added

- **guatiao-remote: RDMA, experimental and Linux only** (feature `rdma`,
  off by default). `rdma://host:port` sets a link up over a trusted TCP
  connection, admitted by address range as TCP's is, then carries every
  frame as one-sided RDMA writes into a ring each side registered; the
  TCP connection stays open as the peer's liveness. libibverbs (rdma-core)
  is opened at run time, so building needs none of it; elsewhere than
  Linux the transport answers `Unsupported`. `RdmaConnect`,
  `RdmaListen`, `RdmaPort` and `devices()` in `transport::rdma`;
  `guatiao-remote-server --listen rdma://...` and the source file's
  `connect: rdma://...` take it too. Tested on Soft-RoCE
  (`crates/guatiao-remote/rdma-tests`, a workspace of its own, since its
  tests need a device); numbers measured there say nothing about RDMA
  hardware.
- **Node: a kind born in TypeScript or JavaScript is written as a
  description.** In TypeScript a kind can be declared as interfaces, the
  way a Rust trait declares one: `guatiao-kind tally_kind.ts --out ...`
  reads them through the TypeScript 7 compiler (an optional peer
  dependency; `describeInterfaces` in `guatiao/describe` from code) and
  writes the kinds' document beside the module. The types come from
  `guatiao/kinds`: `u32`, `i64` and the other scalars, `Taken<MapRef>`
  and `Lent<MapRef>` for how a tree crosses, `extends ObjectKind` for an
  object kind; a method with `?` is an appended slot, `@throws` marks a
  slot that can refuse, doc comments become the description's, and
  anything a table cannot hold is refused with its file, line and column.
  A floor hash not given is the one the same required slots get from
  `#[guatiao::kind]`. A kind defined in JavaScript as a class is
  described by `describeKinds()`, with no compiler (`slot(...).named()`
  and `.doc()`, `static doc`). Either document is the one `kindsOf()`
  loads and Dart's `dart run guatiao:kind` writes classes from:
  `examples/tally_kind` is a kind that exists only as TypeScript
  interfaces, served from Node and from Dart.
- **`guatiao_value_make_local`: a C caller keeps an answer past the
  unload.** A slot that ignored the carrier answers a tree in its
  library's allocator; this moves it into the caller's where it stands,
  allocating nothing when it is already there. Call it while the library
  is loaded, on a tree answer or a lent tree; a map or list moves inside
  a value tagged `GUATIAO_MAP` or `GUATIAO_LIST` holding it. Every
  binding's kind caller now moves home what it keeps (Python, Dart, Node,
  Go, Julia, R, MATLAB and Octave, and Fortran through this export, C++
  through its own walk): a tree answer on arrival, and a lent tree once
  the call returns, answered or refused, into the allocator the tree
  recorded when it was lent, so a slot that wrote a fresh map over it in
  its own allocator is moved home too. Neither then crashes when freed
  after the unload. In R and MATLAB an answer whose move is refused now
  refuses the call, where it was kept as the slot built it. A Fortran
  kind module's remote calls `guatiao_settle_lent(v, tag, home)` right
  after a slot it lent a tree to, with the `home` that
  `guatiao_lend_tree(v, tag, ptr, stat, home)` answers, and reads a
  refused move of an answer from `guatiao_settle_home(answer, stat)`;
  the binding's `AGENTS.md` lists every helper a remote uses.
- **C++: `guatiao_check_kinds` holds a kind's declaration to its
  description.** A CMake function the package defines adds a test that
  reads the `GUATIAO_KIND` and `GUATIAO_OBJECT_KIND` lines of a header
  and the kind's `<crate>.kinds.json`, and fails on a lent tree declared
  handed over (or the reverse), a required slot declared optional, a slot
  left out, or any other signature than the one described: the
  difference the compiler cannot see, since both are the same C pointer.
  The example greeter kinds are checked.
- **`guatiao-js`: libraries written in JavaScript, served to any host
  without Node.** A source file (`crates/guatiao-js`, built from this
  repository, not published) with QuickJS built in. A library is an ES
  module whose exported classes extend `Provider`, implement the kinds
  they list in `static kinds` and refuse with `BadValue`, the Node
  binding's authoring shape, so the same file runs under Node and here;
  it is described by a `*.guatiao-js.toml` manifest, with its version and
  entry taken from the `package.json` beside it, and its kinds come from
  their `<crate>.kinds.json`. Each library runs in a context of its own,
  and holds only the host objects it was given:
  refresh serves an edited module (or any file it imports) while the old
  build answers its instances. Limits stop runaway code (operations and
  time by QuickJS's interrupt, which a `try` cannot catch, depth by stack
  size, memory; the promise reactions a call queued included, and the
  size of what a script hands back) and the host carries on. 64-bit
  integers cross as
  `bigint`, a decimal a double would change as an `ExactNumber`. See
  `docs/js.md` for what differs from Node.
- **`guatiao-wasm`: libraries compiled to WebAssembly, served to any host,
  sandboxed.** A source file (`crates/guatiao-wasm`, built from this
  repository, not published) with wasmtime built in: a library is a core
  WebAssembly module speaking guatiao ABI 1 (five exports, five imports,
  every value as its wire encoding in the module's memory), described by a
  `*.guatiao-wasm.toml` manifest and its kinds' `<crate>.kinds.json`.
  Guests in Rust (`guatiao-wasm-guest`) and in Go 1.24 or later (the
  `guest` package, Go's own `wasip1` toolchain), with example greeters in
  both (`examples/wasm_greeter_rust`, `examples/wasm_greeter_go`). A module
  sees its own memory and what a call hands it: no files, network or
  environment. Operations, time and memory limits stop runaway code, and
  count what the host does for a module (copies, randomness, bulk memory
  instructions, decoding) and takes from it; a trap or a limit discards
  the instance, the next call makes it afresh, and the host carries on.
  Calls run on the caller's thread, each library behind a lock of its own.
- **A provider's meta, and its form, from Go, Julia, R, MATLAB and Octave,
  and Fortran.** `Registry.ProviderMeta` (Go), `Guatiao.provider_meta`
  (Julia), `reg$provider_meta` (R), `reg.provider_meta` (MATLAB and
  Octave) and `reg%provider_meta` (Fortran) answer a copy of what a
  provider declared beyond its schema, or that binding's "none"; the form
  key is `intake.ProviderFormKey`, `GuatiaoIntake.PROVIDER_FORM_KEY`,
  `intake_provider_form_key`, `guatiao.intake.provider_form_key()` and,
  in Fortran, the header's `GUATIAO_INTAKE_PROVIDER_FORM_KEY`. Each suite
  reads `derived_greeter_shouter`'s form and checks it against its
  schema.

- **`guatiao-rhai`: libraries written in Rhai, served to any host.** A
  source file (`crates/guatiao-rhai`, built from this repository, not
  published) with Rhai built in: a library is a script ending in a map of
  providers, whose function members are the kinds' slots and whose other
  members are their state, described by a `*.guatiao-rhai.toml` manifest
  and the kinds' `<crate>.kinds.json`; its version is the script's
  `const VERSION`, read from the parse. Refresh runs an edited script
  afresh while the old build answers the instances taken from it; limits
  stop runaway code (operations, time, depth, and the size of strings,
  arrays and maps), a `try`/`catch` around it included, and the host
  carries on; every call runs on a stack sized from them, so deep
  recursion and values nested within an `operations` limit stop or finish
  instead of aborting the process (nesting built up over many calls in a
  provider's state, or with no `operations` set, is not bounded). A script
  reaches no file, cannot import, and its `print` does nothing. Values keep
  null apart from absent, an exact number's text and bytes apart from
  text; a map's keys come back in byte order, as a Rhai map keeps them.
- **`guatiao-lua`: libraries written in Lua, served to any host.** A
  source file (`crates/guatiao-lua`, built from this repository, not
  published) with Lua 5.4 built in (`lua55` and `luau` as features): a
  library is a Lua module returning a table whose `providers` are tables
  of methods named for the kinds' slots, described by a
  `*.guatiao-lua.toml` manifest and its kinds' `<crate>.kinds.json`.
  Refresh runs an edited module afresh while the old build answers the
  instances taken from it; limits stop runaway code (operations, time,
  depth, memory), a `pcall` around it included, and the host carries on.
  Values keep every shape across: an exact number, bytes apart from text,
  null and absent, a list apart from a map, a map's order.
- **Lua as a host: `require "guatiao"`.** A module a stock Lua
  interpreter loads (`bindings/lua/guatiao`, built for Lua 5.4, 5.5 or
  LuaJIT with `luarocks make` or cargo), the guatiao library inside:
  a registry that loads libraries and sources, kinds read from their
  descriptions, providers bound to a kind and called as methods, refusals
  as `nil, err`, and Lua tables handed to providers as objects.
- **`RawObject::lend`**: an object a caller owns, lent to a slot that
  takes a lent one, without `unsafe`.
- **`guatiao-remote`: providers called across a process, a machine or a
  network.** A new crate, published with the others. A device runs
  `guatiao-remote-server` (or `Server` over a `Registry`); a host adds a
  `RemoteSource`, or in any language the crate's library file as a
  source, and after `refresh()` calls the device's providers through the
  kinds' own traits and C tables. Every slot shape crosses: borrowed,
  taken and lent trees, buffers, every answer, objects both ways, and a
  callback during a call, which runs on the thread that lent the object.
  A remote object holds its library on the far side until its handle is
  dropped, its call is cancelled or the link is lost; a call past its
  timeout or on a lost link answers `GUATIAO_ERR_GONE` and the host
  carries on: a crash in a server started as the host's child takes down
  the child, and the next refresh starts it again. Transports: that child
  process; a Unix socket or a named pipe (feature `local`); shared memory
  beside one (feature `shm`), a few microseconds a call; and TCP at the
  trusted level (feature `tcp`): the address ranges a listener accepts
  peers from, others closed unread, and a pre-shared key both sides
  prove. The protocol is
  stated for every language in `docs/remote.md`, with shared vectors and a
  C codec, client and server. `benchmarks/remote` measures each transport:
  a call's latency, calls a second, throughput, bytes and CPU per call.
- **`DescribedRemote::implements` and `DescribedRemote::kind`**: whether
  a provider's table fills a slot, read under the table's own size, so a
  caller that lists a table's slots leaves the unfilled ones to its own
  default.
- **`guatiao-bridge`: what every bridge to a language runtime shares.** A
  new crate, `#![forbid(unsafe_code)]`, published with the others. A
  runtime implements `Engine` (start from the host's configuration, open
  the library a manifest describes, answer calls, close) and states its
  `ThreadModel`: called from any thread, behind its own lock, or pinned to
  an engine thread the bridge starts, which queues calls from other
  threads and runs one the engine makes into itself there and then; that
  thread's stack is the one `Engine::stack_size` asks for, reserved and
  used only as deep as the engine goes.
  `BridgeSource` is the engine as a guatiao `Source`, exported with
  `guatiao::source!`: manifests read without running code
  (`ManifestFormat` names the file suffix, the id key and the code a stamp
  follows), stamps that change with the manifest or the newest code file,
  refresh, an old build kept open for the instance taken from it.
  `Opened::Built` is a library whose tables the engine built itself;
  `Opened::Described` declares providers and the slots they implement, and
  their tables are built from the kinds' descriptions (`Kinds`, from
  `<crate>.kinds.json`) by the mechanism the source is given
  (`with_mechanism`; without one such a library is refused), every slot a `Call`
  into the engine with owned arguments, lent ones written back, and
  objects both ways. `Limits` (operations, memory, time, depth) come from
  the configuration and `BridgeError` maps every refusal to a status.
  `Convert` is an engine's values both ways, and `conformance::round_trip`
  checks one against the shared wire vectors.
- **`guatiao-libffi`: libffi as the mechanism for tables built at run
  time.** A new crate, published with the others, depending on `guatiao`
  alone: `Libffi` prepares a slot's signature as a libffi call interface,
  makes each served slot a libffi closure, and calls a table through
  `ffi_call`. A bridge for a runtime with no FFI hands it to its source
  (`with_mechanism(Arc::new(Libffi))`); `guatiao-lua` does. It builds
  libffi from C and is not available on wasm32; `guatiao` and
  `guatiao-bridge` build no C.
- **A source written in Rust is a file any host adds.**
  `guatiao::source!(serves = "...", make = start)` exports a type
  implementing `Source` as a source file's `guatiao_source`, declaring
  `source=<serves>` so a scan reports it and never loads it; `start(host,
  config)` builds the source from the host's configuration map or refuses
  it in its own words. The table it answers keeps each descriptor `open`
  hands over until the registry closes it, and turns every panic into a
  status. `Source` and `Candidate` no longer need the `load` feature.
  `examples/source_library` is one, added by a Rust host and a C host in
  the tests.
- **Kind tables built at run time from a description.**
  `KindDescription` reads a kind's description from a value (a kind or a
  `{"format": 1, "kinds": [..]}` document), with no dependency, and
  `ServedTable` builds a kind's table from it, with no feature: each slot a
  closure calling one `Serve` handler
  with the slot and its arguments as `Arg`s, and writing its `Answer` back
  as the kind's C table promises (a tree in the caller's carrier, an error
  through `err`, a panic as `GUATIAO_ERR_INTERNAL`); a slot left out is
  null, so a caller runs the kind's default. `Caller` calls any table of a
  described kind, `RawObject` and `LentObject` an object one was handed,
  and `DescribedRemote` a provider a registry offers. What a bridge to an
  engine with no FFI of its own (Lua, JavaScript, Rhai, wasm) serves and
  calls kinds with. The closures and the calls come from a `Mechanism`
  (`ServedTable::new(.., &mechanism)`, `Caller::new(kind, &mechanism)`):
  the one thing Rust cannot do for a signature it learns at run time, an
  `unsafe` trait of two operations over a slot's `CSignature`. `ProviderPartsBuilder::keep` keeps what a provider's tables
  point into with its parts, and `kinds` and `table` take names that are
  not `'static`.
- **A kind describes itself as data.** `#[guatiao::kind]` adds
  `<Trait>Vtable::description()`: the kind's name, floor hash, whether it
  is an object kind, and every slot in table order with its arguments,
  answer, whether it is required and whether it can refuse, as JSON. A
  kind crate commits the document of its kinds beside its C header
  (`examples/greeter_kind/include/greeter_kind.kinds.json`), written by
  the same `GUATIAO_WRITE_HEADER` switch and compared by a test, so a
  binding loads a kind instead of declaring it again. The format is in
  the docs' encodings page.
- **Node: a kind that exists is loaded, and a kind is a class.**
  `kindsOf(document)` and `kindOf(description)` load kinds from their
  description. The package installs `guatiao-kind`, which writes a module
  from a description: each kind a value in JavaScript and, under the same
  name in its `.d.ts`, an interface (required slots as methods, appended
  ones optional) and a `<Kind>Remote`, so `class Hello extends Provider
  implements Greeter` is checked by `tsc --strict` with nothing declared
  by hand; `--check` fails on a stale module. A kind defined in
  JavaScript, with no header behind it, is a class extending `Kind` whose
  static fields (`kind`, `floorHash`, `object`, `slots` of `slot(...)`)
  describe it. Plain JavaScript providers run with no build step, and a
  missing required method is a `TypeError` naming it. `kind()` stays as
  the primitive underneath. `examples/node_js` is a library and a host in
  plain JavaScript.
- **Dart: kinds and providers as classes.** `dart run guatiao:kind` writes
  a kind description's Dart: an abstract interface class per kind and per
  appended slot, a `<Kind>Remote`, and the kind as the interface's static
  `kind`, so `reg.bind(key, Greeter.kind)` calls a table through a class
  and `class Hello extends Provider implements Greeter` is checked by the
  analyzer. `Library` and `Offer` serve providers written in Dart to any
  host in the process; `dart run guatiao:kind identity` writes a
  library's id and version from `pubspec.yaml`. It runs under AOT and in
  Flutter, Android included. `examples/dart_host` calls the Rust, C,
  Python and Dart greeters and has the Rust caller call the Dart one;
  `examples/dart_provider` is a library written in Dart.
- **C++ bindings**, a header-only CMake package per crate:
  `bindings/cpp/guatiao` (`guatiao::guatiao`), `bindings/cpp/guatiao-serde`
  (`guatiao::serde`) and `bindings/cpp/guatiao-intake` (`guatiao::intake`),
  C++17 or later, linking the libraries: the one `GUATIAO_LIBRARY` (or the
  package's own variable) names, else built with cargo when CMake
  configures. Values are owners freed by their destructors, views and
  writable places borrowed like iterators, `std::string_view` text; every
  call that can fail throws `guatiao::error` or, in its `try_` form,
  answers `guatiao::result<T>`, so code built without exceptions uses the
  same headers. A kind is declared once over its C table with
  `GUATIAO_KIND`; `remote<K>`, `object<K>` and `lent<K>` call any table of
  it, and any class whose methods fit serves it (a method that does not
  fit, or a required slot left out, is a compile error). A
  `guatiao::library` of such classes becomes a library file any host's
  scan finds with `GUATIAO_LIBRARY(f, kinds...)`; `guatiao::registry` is
  a host with sources and `refresh()`. Examples: `examples/cpp_greeter`
  (called by the Rust and Python example hosts too), `examples/cpp_host`,
  `examples/cpp_consumer`; docs page `cpp.md`.
- **Values as events, with no tree in the middle.** `value::visit` is
  guatiao's own data model as traits, with no dependency: a type that
  `Walk`s hands absent, null, bool, number text, string, bytes, list and
  map events to a `Visitor`, and a type that is `FromWalk` pulls them
  from a `Walker`. A `Sink` is the same events on one type with its own
  stack, and `Flat` makes one a visitor; `TreeSink` builds a value from
  any walk. Both traits are implemented for what `ToValue` and
  `FromValue` cover, writing the same tree's events and refusing with the
  same errors, and `&Value` is a walker. The wire codec streams on them:
  `wire::to_bytes_of`/`encode_of` write any walk as exactly the bytes its
  tree encodes to, `wire::decode_as` reads bytes into any `FromWalk` type,
  and `wire::Encoded` walks bytes into any visitor, each checking what
  `decode` checks. A streaming walk follows at most `MAX_DEPTH`
  containers (`WireError::TooDeep`); a type refusing well-formed bytes is
  `WireError::Refused`; a walker whose own input failed answers the new
  `MapError::Stopped` and keeps its error.
- **Derived types stream.** `#[derive(ToValue)]` also implements `Walk`
  and `#[derive(FromValue)]` also `FromWalk`, so a derived configuration
  goes to wire bytes and back with no tree (about twice as fast and more,
  measured locally: `benchmarks/README.md`). Every `#[map]` attribute
  means what it means in the tree impls, and a field type with only
  `ToValue`/`FromValue` is walked and read through its own tree, so every
  type that derived before still does; enums and a flattened member go
  through the type's tree.
- **serde formats with no tree in the middle.** `guatiao-serde` joins
  serde and the visitor four ways: `Walked` writes anything that walks
  with any serde format; `from_deserializer` reads any `FromWalk` type
  from any serde deserializer; `deserialize_into` hands a document's
  events to any sink (a tree, the wire encoder); and `Serde<T, S>` walks
  and reads through `T`'s own serde impls. Each format gains
  `to_string_of` and `from_str_as` (`text::json`, `toml`, `yaml`): the
  same text and value as the tree forms, a refusal the type makes being
  the new `text::Error::Refused` naming its key. A JSON configuration
  reads into a derived type about twice as fast, and writes about four
  times as fast, measured locally. `Serializable`, `ValueSeed` and the
  `adapt` modules are those bridges now, with the same results.
  `Number::is_number` checks text against the number grammar with nothing
  allocated.
- **Values as events from C.** `guatiao_visitor` is a table of one C
  function per event and `guatiao_walker` a C function that hands its own
  data to one: `guatiao_value_visit` and `guatiao_wire_visit` walk a
  value or wire bytes into a C visitor, and `guatiao_value_from_walker`
  and `guatiao_wire_encode_walker` build a value or write wire bytes from
  a C walker, a callback's status stopping the walk. In `guatiao_serde.h`,
  each format reads a document straight into a C visitor
  (`guatiao_json_visit`), writes one from a C walker
  (`guatiao_json_emit_walker`), and turns a document into wire bytes and
  back (`guatiao_json_to_wire`, `guatiao_json_from_wire`), so a program
  in any language parses into its own objects and writes from them with
  no value in between: through callbacks, or through the wire bytes as a
  tape decoded with one crossing. Rust: `text::*::visit_str` hands a
  document to any sink.
- **Python and Dart: a document straight to plain objects and back.**
  `guatiao_serde.parse(text, format=, how=, numbers=)` answers what
  `loads(...).to_python()` does and `render(obj, format=, pretty=, how=)`
  writes what `dumps(Value.from_python(obj))` does, with no `Value`
  built; Dart's `parse` and `render` likewise, on the VM and the web. The
  document crosses once as wire bytes, read and written in the
  binding's own language by `guatiao.ffi.to_python`/`from_python`
  (Dart: `wireToDart`/`dartToWire` in `ffi.dart` and `wasm.dart`),
  which measured faster than a callback per event in both. Measured
  locally on a structured document: 2.4x to 10x faster in Python and
  1.4x to 5x in Dart than going through a `Value`; about even on one
  large string (`benchmarks/README.md`).
- **A page on configuration files.** Reading a configuration through
  guatiao rather than serde: the TOML text becomes a value, `FromValue`
  makes the type, and the schema and form come from the same derive, so
  one set of attributes describes it; with a table from serde's
  attributes to guatiao's and a worked `#[map(with = ..)]` module.
- **A field that crosses as its text.** `#[map(as_text)]` writes a field
  with `Display`, reads it with `FromStr` (a refusal carries the parse
  error's words under the field's key) and describes it as a string, to
  which `#[schema(format, pattern)]` add: a type with only those two
  traits needs no value impls to sit in a derived struct, as `T` or
  `Option<T>`.
- **Enums told apart by kind.** `#[map(untagged)]` on an enum whose
  variants each write a different kind of value (unit variants a string
  of their name, named fields a map, `Name(T)` whatever `T` reads) stores
  no tag: a reader picks the variant by the value's kind, the same way in
  every language, and the schema is an `anyOf` of the arms. The rule is
  on the encodings page. Two variants reading one kind are a compile
  error naming both. `FromValue::KINDS` and `Kinds` say which kinds a
  reader accepts; `KindBuilder::title` and `description` label a union's
  arms.
- **Any serde type as a field.** `guatiao-serde` converts a type that
  speaks serde (`toml::Table`, `serde_json::Value`, a type of your own)
  to and from a value, so a derived configuration type can hold a table
  it does not describe. On a bare field, `#[map(with =
  guatiao_serde::adapt::object)]` describes it as an open object and
  `adapt::any` as any value; the wrapper `guatiao_serde::Serde<T, S>`
  takes its schema from `S` (any value by default). A module of your own
  reuses `adapt::to_value_in` and `adapt::from_value` beside its own
  `kind`. Reading names the key it failed at and never quotes the value.
  `guatiao` itself depends on no data-format crate, not even optionally.
- **Dart and Flutter build the libraries from source.** Each Dart package
  has a build hook that builds its crate with cargo for the target being
  built (Android, once per ABI; iOS; macOS; Linux; Windows) and bundles
  the library with the app, so `flutter build` is the whole build. The
  one-time setup is rustup and the Rust targets; a missing piece (cargo,
  the Rust target, the Android NDK, Xcode) stops the build with one
  sentence naming it, the `rustup target add` line included. Cargo's
  output is cached in the app's `.dart_tool/`, and a build with nothing
  changed skips the hooks. An application's own crate is built instead,
  one file carrying every surface, with `hooks: user_defines: guatiao:
  crate: <path>` in its `pubspec.yaml`; `build: false` builds nothing.
  The packages find the bundled library through its asset id, after
  `GUATIAO_LIBRARY`. Inside a checkout the hook builds the checkout's
  crates, from pub.dev the crates.io ones at the package's version; no
  binary is downloaded. `package:guatiao/hook.dart` is that build,
  `examples/flutter_app` an app using it, and the docs site's "Dart and
  Flutter" page the setup.
- **Memory from the library's heap.** `guatiao_memory_alloc(size,
  align)` answers zeroed bytes from the library's own heap, and
  `guatiao_memory_free(ptr, size, align)` releases them. A host that
  shares no heap with the library, a page calling the WebAssembly module
  above all, puts what it passes by address there: a view's bytes, an
  out-parameter, a root it keeps.
- **Dart on 32-bit platforms.** `armeabi-v7a` and armhf Linux, whose
  4-byte pointers the package refused: its code reads the layout in
  pointer widths, and its suites run on 32-bit ARM.
- **Dart on the web.** `package:guatiao`, `guatiao_serde` and
  `guatiao_intake` run under dart2js and dart2wasm with the same API, over
  the one WebAssembly module a page loads (`await loadModule(url)`):
  `guatiao_intake.wasm`, built by `cargo build -p guatiao-intake --target
  wasm32-unknown-unknown --release`, or an application's own. Values live
  in the module's memory; what a browser cannot do (the registry, kind
  tables) throws `Unsupported`. The shared vectors run in Chrome under
  both compilers.
- **One library for an application.** A docs page, "One library for your
  application", gives the recipe: a `cdylib` crate of the application's
  that depends on the guatiao crates it wants exports their C surfaces
  beside its own functions, for any target, WebAssembly included.
  `examples/app_library` is that crate and `examples/app_host` a Rust
  program linking it beside the `guatiao` crate. Every binding's suite runs
  against it alone as well as against the crates' own libraries.
- **Channels from C.** `guatiao_wire_announce`, `guatiao_wire_send` and
  `guatiao_wire_close` write a channel frame as a BYTES value;
  `guatiao_wire_receiver_new(alloc)`, `guatiao_wire_receiver_accept(recv,
  frame, out_event)` and `guatiao_wire_receiver_free` keep each channel's
  schema and check every value against it. A `guatiao_wire_event` says
  what the frame was (`kind`, `channel`, and the schema's copy or the
  checked value, the caller's) or why it was refused: `refusal`
  (`GUATIAO_WIRE_MALFORMED`, `_NO_SCHEMA`, `_NOT_A_SCHEMA`, `_INVALID`)
  beside the status, the byte offset, the error's words, and for a value
  its schema refuses, the field and what it accepts. Nothing received is
  quoted. The wasm module exports them too.
- **Values on the wire and channels in every binding.** One value as
  bytes, frames, and a receiver whose accept answers an event or raises
  the refusal as the status's own error: Python `Value.from_wire`,
  `ref.to_wire()` and `guatiao.wire`; Dart `Value.fromWire`, `toWire()`
  and `package:guatiao/wire.dart`; Node `Value.fromWire`, `toWire()` and
  `guatiao/wire`; Go `FromWire`, `ToWire`, `WireAnnounce`/`WireSend`/
  `WireClose` and `NewReceiver`; Julia `from_wire`, `to_wire` and
  `Guatiao.Wire`; R `from_wire`, `to_wire`, `wire_announce`/`wire_send`/
  `wire_close` and `wire_receiver()`; MATLAB and Octave
  `guatiao.Value.from_wire`, `to_wire` and `guatiao.wire`; Fortran
  `guatiao_from_wire`, `guatiao_to_wire`, `guatiao_wire_*` and
  `guatiao_open_receiver`. The encodings page states the frame grammar
  and the refusals beside the value encoding.
- **Shared wire vectors**, `crates/guatiao/tests/vectors/wire.json`:
  values with their bytes, malformed inputs with the byte each is refused
  at, the frames the builders write, and a conversation through one
  receiver with every refusal. The Rust code, a C program and the
  bindings run it.
- **A migration page**, "Migrating from 0.0.0-alpha.0" on the docs site:
  every break since that release, grouped by what a consumer touches
  (values, schema and derives, kinds and providers, the loader and its
  handles, `guatiao-serde`, `guatiao-intake`, the C surface, Python and
  Dart), each with the call before and after; the Rust it shows is
  compiled by the test suite.
- **A provider's `meta` from C, Python, Dart and Node.**
  `guatiao_registry_provider_meta(reg, key, alloc, out)` writes a copy of
  the provider's `meta` map in `alloc`, which outlives the library: ABSENT
  for a provider that declares none, `GUATIAO_ERR_NOT_FOUND` for an
  unknown key. `Registry.provider_meta` (Python) and `providerMeta` (Dart,
  Node) answer it as a `Value` to close, or nothing. Its form is under
  `GUATIAO_INTAKE_PROVIDER_FORM_KEY` in `guatiao_intake.h`,
  `intake.PROVIDER_FORM_KEY` in Python and `providerFormKey` in Dart and
  Node, so a host in any of them reads a provider's form and checks it
  against `provider_config` with `intake.check`, as a Rust host does
  through `Offer::meta`.
- **The example configured greeter carries its form**:
  `derived_greeter_shouter` declares `#[provider(Greeter, config, form =
  Shouter)]`, two sections, a placeholder, a unit and a field shown only
  while `exclaim` is `true` (`exclaim` and `marks` default, so a
  configuration naming only `prefix` still builds it). The greeter host's
  tests read the form back and check it from Rust and from a C program.
- **A documentation page, *A provider's form***: how a provider writes its
  form with `#[provider(.., form = T)]`, and how a host reads it back and
  checks it from Rust, C, Python, Dart and Node, with the example greeter.
- **`into_local()`, `into_alloc(alloc)` and `is_local()` on `Value`,
  `Map`, `List`, `Text` and `Buffer`.** Owning a tree without copying one
  that is already this binary's: `into_local()` hands back a tree whose
  every container and key records `Alloc::rust()` unchanged, allocating
  nothing, and otherwise copies it there and frees the original through
  its own allocators. `into_alloc(alloc)` is the same for a named
  allocator, fallible as `clone_in` is. The proxy's return path for a
  tree is built on them.
- **`make_local_in(alloc)` on `Value`, `Map`, `List`, `Text` and
  `Buffer`**, the in-place counterpart of `into_alloc`: through `&mut`,
  every block another allocator made is moved into one from `alloc` and
  the old one freed through its own, elements moved rather than copied,
  nothing allocated when the tree is already there.
- **A kind method may take a `Value`, `Map` or `List` by value**, and it
  crosses as a move: the slot receives a pointer to the caller's node
  (`guatiao_value *`, `guatiao_map *`, `guatiao_list *`, without
  `const`), the caller forgets it and the callee owns it on every path,
  so nothing is copied. A `Text` or `Buffer` by value stays refused, with
  a message naming `&str`/`&[u8]`.
- **A kind method may take `&mut Map`, `&mut List` or `&mut Value`**:
  the caller's tree, lent for the call, which the callee may change and
  grow in place (`fn check(&self, config: &mut Map) -> Result<bool,
  ProviderError>`). It crosses as `guatiao_map *` (or `guatiao_list *`,
  `guatiao_value *`) to the caller's own node, marked in the C header as
  lent rather than handed over, and hashes apart from `&Map` and `Map`.
  Once the call returns, answered or refused, the proxy moves whatever
  the callee built in another allocator into the one the tree records
  (`make_local_in`), so nothing in the caller's tree needs the library's
  allocator; a callee that builds with the tree's own allocator causes
  no copy. `Option<&mut T>` is refused with a message.
- **`FromValue for Value`**, a copy in this binary's allocator. A
  `ToValue`/`FromValue` field may be a `Value`, `Option<Value>`,
  `Vec<Value>` or `BTreeMap<String, Value>`.
- **`Map`, `List`, `Text`, `Number` and `Buffer` are field types**, under
  all three derives and `#[derive(Form)]`: each writes as the kind it is,
  copied into the target allocator, and reads as a copy in this binary's.
  `Value` describes itself as `{}`, `Map` as an object of any keys and
  values, `List` as an array of anything, `Text` as a string, `Number` as
  a number (its exact text kept), `Buffer` as bytes. `KindBuilder::any()`
  / `any_in` write `{}`.
- **`Box<T>`, `Arc<T>`, `Rc<T>` and `Cow<'_, T>` cross as what they
  hold**, under all three derives; so do `Box<str>`, `Arc<str>`,
  `Rc<str>` and `Box<[T]>`, `Arc<[T]>`, `Rc<[T]>`, as a string and a
  list. A `Cow` reads back owned. `Schema` is implemented for `str`,
  `[T]` and `&T` too.
- **`char` is a field type**: text exactly one character (Unicode code
  point) long, `minLength` and `maxLength` 1 in its schema, read through
  `char::try_from(&value)` as well.
- **Sequences, sets, arrays, tuples and the unit are field types.**
  `VecDeque<T>` is a list; `BTreeSet<T>` and `HashSet<T>` a list with no
  element repeated (`uniqueItems: true`; a repeated element is refused on
  read); `[T; N]` a list of exactly `N` (`minItems` = `maxItems` = `N`);
  a tuple of up to 12 a list by position (`prefixItems`, `items: false`);
  `()` a null.
- **A map may be keyed by something other than text**: `BTreeMap<K, V>`
  and `HashMap<K, V, S>` take any `K: MapKey`, a new public trait
  implemented for the text types, every integer, `char` and `bool`.
  The object is keyed by the key's text and its schema states the key's
  shape in `propertyNames` (`^(0|[1-9][0-9]*)$` for an unsigned
  integer); a key that does not read back, or two that read as one, is
  refused. `HashMap`/`HashSet` take any hasher.
- **The schema reader and validator know `prefixItems`, `uniqueItems`
  and `propertyNames`** (`vocab::PREFIX_ITEMS`, `UNIQUE_ITEMS`,
  `PROPERTY_NAMES`, `GUATIAO_KEY_*` in the header): `Kind::Tuple`, with
  `Kind::positions()`, reads a tuple, and validation enforces each
  position, the length, unique elements and every key's shape.
  `KindBuilder::tuple`, `null`, `.unique_items()` and
  `.property_names(..)` write them. `guatiao-intake` flattens a tuple by
  position (`pair[0]`) and resolves a path into one.
- **Paths, addresses, non-zero integers and `SystemTime` are field
  types.** `PathBuf` (and `Box`/`Arc`/`Rc<Path>`) is its text, a path
  that is not UTF-8 refused on write with `ValueError::NotUtf8`;
  `IpAddr`, `Ipv4Addr`, `Ipv6Addr` and the three socket addresses their
  `Display` text, described with `format: ipv4` / `ipv6` or, for a
  socket address, a `pattern`; every `NonZero*` an integer, `minimum: 1`
  unsigned and `not: {const: 0}` signed; `SystemTime` RFC 3339 in UTC
  (`2026-10-21T13:45:00.5Z`, `format: date-time`), read with any offset
  and refused naming the byte it stopped at. Each of the addresses and
  `PathBuf` is a `MapKey`. The validator enforces `not`
  (`vocab::NOT`, `GUATIAO_KEY_NOT`); `format` stays an annotation.
- **`ValueError::Unrepresentable`**: a value with no spelling in the form
  it is written as, such as a time outside the years 0000 to 9999
  (`GUATIAO_ERR_BAD_VALUE` across the C surface).
- **`std::time::Duration` is a field type**, written as the list
  `[secs, nanos]` (a `u64`, then nanoseconds below 1 000 000 000, the
  order of C's `struct timespec`), described as that tuple. A read also
  takes a number of seconds, exactly from its text (`1.5`, `2e-3`; not
  negative, no finer than a nanosecond), and the `{"secs", "nanos"}` map
  serde writes.
- **`guatiao::Span`**, a length of time in calendar units: years, months,
  weeks, days, hours, minutes, seconds and nanoseconds, one sign for the
  whole span, equal unit by unit (`P1D` is not `PT24H`). Its text is an
  ISO 8601 duration (`P1Y2M3W4DT5H6M7.5S`, `-P1D`, `PT0S`), canonical
  from `Display`; `FromStr` also reads `1y 2mo 3w 4d 5h 6m 7s 500ms 1us
  1ns` (spaces optional, `min` for minutes). Builders `years(n)` ..
  `nanoseconds(n)`, getters `get_years()` .., `is_zero`, `is_negative`,
  `abs`, `negate`, `Neg`, `From<Duration>`, `TryFrom<Span> for Duration`
  (exact units only). A field type written as its text, `format:
  duration`.
- **`ValueError::Unparsed { at, expected }`**: text refused by a grammar,
  naming the byte offset and what was expected there, never the text.
- **`guatiao::{Date, Time, DateTime, Timestamp}`**: a calendar date
  (`2026-10-21`, `format: date`), a time of day (`13:45:00.5`), a civil
  date and time with no offset (`2026-10-21T13:45:00.5`), both described
  with a `pattern` because JSON Schema's formats for them require an
  offset, and an instant (RFC 3339 read with any offset, written in UTC,
  `format: date-time`). No time zones; the years 0000 to 9999; a refusal
  names the byte offset. Each is a field type and a `MapKey`, `Copy`,
  `Ord` and `Hash`. `Date::checked_add` and `DateTime::checked_add` apply
  a `Span`: years and months first with the day clamped to the month's
  end (`2026-01-31` + `P1M` = `2026-02-28`), then weeks and days, then
  the time units, carried across midnight. `SystemTime` reads and writes
  through `Timestamp`, and an offset that moves an instant outside the
  years is now refused when read rather than when written again.
- **Dates, times and spans from C.** `guatiao_{date,time,datetime,
  timestamp,span}_{parse,format,normalize}` read text into a plain struct
  (`guatiao_date`, `guatiao_time`, `guatiao_datetime`, `guatiao_timestamp`,
  `guatiao_span`), write a struct as canonical text and rewrite text
  canonically, with the Rust types' own grammar; a refusal is
  `GUATIAO_ERR_BAD_VALUE` and a `guatiao_text_error` holding the byte
  offset and what was expected there. `guatiao_date_add_span` and
  `guatiao_datetime_add_span` apply a span, and `guatiao_duration_read`
  reads an exact length in any of its three spellings. A C program reads
  every vector of `tests/vectors/time.json` through them in the suite.
- **`guatiao::Secret<T>`**, a value nobody prints: it crosses as `T`, its
  schema is `T`'s with `writeOnly: true` and `x-sensitive: true`, and its
  `Debug` and `Display` print `[redacted]`. `expose()`, `into_inner()`,
  `From<T>`. No constant-time comparison and no wiping on drop, both
  stated.
- **`guatiao_schema_redact`**, redaction from C: a copy of a value with
  every secret replaced by a placeholder, local `$ref`s followed first.
- **`guatiao-intake`: a secret typed as text is masked in a created
  form.** `for_schema` (and `for_field`, `form_for`) give a text field
  marked `x-sensitive` or `writeOnly` the `password` widget.
- **Python: `guatiao.time` and `guatiao.redact`.** Dates, times of day,
  civil date-times and instants as `datetime.date`, `datetime.time`,
  naive and aware `datetime.datetime`, an exact length as `timedelta`, a
  calendar length as `guatiao.time.Span`, all through the C functions and
  checked against the shared vectors; `normalize` keeps every nanosecond.
  `datetime` counts microseconds and starts at year 1, and the docs say
  what that loses. A refused text raises `guatiao.Unparsed`, a
  `ValueError` with the byte offset.
- **Dart: `package:guatiao/time.dart` and `redact`.** `PlainDate`,
  `PlainTime` and `PlainDateTime` for the civil forms (a `DateTime` would
  carry a zone), a UTC `DateTime` for an instant, a `Duration` for an
  exact length, and a `Span` class, all through the C functions and
  checked against the shared vectors. A refused text throws `Unparsed`, a
  `FormatException` with the byte offset.
- **The derives take every struct shape, and generics**, as serde writes
  them: a newtype struct (or one marked `#[map(transparent)]`) is its
  field's value and describes its field's kind; a tuple struct is a list
  by position, described with `prefixItems`; a unit struct is a null. A
  generic type's parameters are each bound by the trait derived;
  lifetimes and const parameters pass through. `#[derive(Form)]` takes
  generics too.
- **A tagged enum's variant may hold one type, `Name(T)`**: the name
  under the tag, then `T`'s map beside it, the arm declaring `T`'s
  fields (serde's internally tagged newtype). A `T` that writes no map
  is refused when written, with `ValueError::WrongKind`.
- **`#[map(flatten)]`, `#[map(with = path)]`,
  `#[map(skip_serializing_if = path)]` and
  `#[map(deny_unknown_fields)]`** mean what serde's do: a flattened
  member's keys are written beside the others and its fields join the
  schema's (two on one key refused with `ValueError::DuplicateKey`); a
  `with` module's `to_value_in`, `from_value` and `kind` convert a field;
  a predicate leaves a key out (with `default` to read it back); and a
  key no field declares is refused on read. `alias` is refused by name: a
  schema refuses every key it does not declare.
- **An "Encodings" page** on the docs site states every form above for
  any language: the value, a JSON example, what a reader accepts, the
  schema, and the exact grammars of the RFC 3339 timestamp and of
  `Span`'s ISO 8601 and friendly forms. The C header and the crate's
  shipped `AGENTS.md` point at it; the latter lists every field type
  with its value and schema.
- **A guide to passing data**: the docs site's "Passing data" page and a
  section of the crate's shipped `AGENTS.md`, on returning trees by value
  and taking them by value when kept, building an answer in the
  allocator a call carries (`<method>_in`), lending a tree to be changed
  in place (`&mut Map`) and building what goes into it with its own
  allocator, moving rather than copying, every way to a tree you own and
  when it copies, and the C conventions.
- **Sources: libraries that come and go while the host runs.** A
  source is kept for the registry's life and serves ordinary libraries
  it lists as data, opens and is told it may close: a Rust `Source`
  trait (`Registry::add_source`), the C table `guatiao_source_info`
  (`add_source_table`, `guatiao_registry_add_source`), and a file
  exporting `guatiao_source` (`add_source_file`,
  `guatiao_registry_add_source_file`), which is never unmapped.
  `watch_dir` keeps a directory in step with disk. `refresh()` (C
  `guatiao_registry_refresh`) re-lists every source and diffs by source,
  id and stamp: new candidates load, a changed one loads before its old
  build is retired, a vanished one is retired, a library the host
  retired stays out until it changes, and every refusal is reported in
  `Refreshed` rather than failing the call. A served library closes
  through `collect`/`unload` once nothing holds it, and its source is
  told once. Nothing runs on its own: no thread, no watcher. A source
  file declares `source=<what it serves>`, and a scan reports it as
  `Skipped::Source` without loading it. Python and Dart:
  `add_source_file`/`addSourceFile`, `watch_dir`/`watchDir`, `refresh`.
- **Python: a kind is a typed `Protocol` class, called and served from
  Python.** `class Greeter(kinds.KindProtocol, Protocol, kind="greeter",
  floor_hash=...)` lists a kind's methods in table order; the shapes are
  read from the annotations (`str`, `bytes`, `bool`, `float`, the width
  aliases `kinds.I8`..`kinds.U64`, `Map`, `Ref`, `kinds.MapAnswer`,
  `Taken[...]`, `Lent[...]`, an object kind class as an answer), with
  `@required` and `@fallible` for the slot's place and `err`, and a
  `TypeError` at class creation names a method no shape fits. It builds
  the table's `ctypes.Structure` or checks one written by hand; mypy and
  pyright check code calling or implementing it. `Registry.bind(key,
  Greeter)` and `Instance.bind(Greeter)` answer a `Remote` whose methods
  take and return Python values: a map lent to a slot is changed where it
  stands, a `Value` handed over is left ABSENT, an answer is built in the
  binding's allocator, an absent slot raises `Unsupported`. A Python
  object, inheriting the kind class or not, is handed or lent to a slot
  as an object of an object kind, through a table this binding builds
  over its class; an object a slot answers is a `kinds.Object` whose
  `close()` calls its `destroy`. A refusal crosses with its status and
  words; an exception in a Python method crosses as
  `GUATIAO_ERR_INTERNAL` naming it. `kinds.Kind` and `kinds.Method`, with
  the shapes of `kinds.shapes`, describe a kind without a class.
- **Python: a module of provider classes is a library.** A provider is a
  class inheriting `guatiao.provide.Provider` and the kind classes it
  serves (`class Hello(provide.Provider, Greeter)`); a method a kind
  requires and the class lacks is a `TypeError` when the class is
  created. Its name defaults to the class name in snake_case; `named`,
  `display_name`, `version`, `meta`, `default` and `config_schema` are
  class keywords; `available` is a classmethod. An `__init__` taking
  nothing makes the provider its one instance; one taking a `config`
  annotated with a dataclass or a `TypedDict` declares the schema hosts
  check against and receives the configuration as that type. A module is
  the library: its `__name__` the id, its `__version__` the version, its
  providers the provider classes in `__all__` or, without it, those it
  defines. Its `meta` merges the installed distribution's metadata
  (summary, licence, authors, project URLs, licence files as `notices`),
  the module's docstring, `__author__`, `__license__` and
  `__copyright__`, and a `_meta_` mapping. `provide.library_of(module)`
  builds it once; `provide.library(id, version, providers=...)` builds
  one without a module; `Registry.register_library` takes either. A host
  in any language calls it as it calls a Rust or C library.
- **Python: `guatiao.use_library(path)`** names the guatiao library to
  load before the first native call, a file or a directory, ahead of
  `GUATIAO_LIBRARY`; naming another once one is loaded raises
  `RuntimeError`. A program that carries the guatiao functions under its
  own name points the binding at itself.
- **Python: the package is typed** (`py.typed`) and checked with
  `mypy --strict`, as are the Python examples; `Map.get(key, default)`
  answers plain data like a `dict`.
- **`guatiao-py`, a source of libraries written in Python.** A cdylib
  exporting `guatiao_source` and declaring `source=python`: a host in any
  language adds it with `add_source_file` and a configuration naming
  where Python libraries are described (`dirs`, `manifests`) and what to
  put on `sys.path` first (`paths`). Each library is a manifest,
  `*.guatiao-py.toml`, naming its `module`, the `kinds` it serves, the
  `paths` it imports from, and optionally its `version` (otherwise a
  literal `__version__`, read without importing). Listing imports
  nothing; opening imports the module afresh and refuses one whose name,
  version or kinds differ from the manifest; closing drops the modules it
  imported, so a refresh serves edited code while instances of the old
  build still answer. One interpreter serves the program and is never
  finalised. It builds for CPython's stable ABI from 3.9, is not
  published, and is built by whoever ships it.
- **Python: an error raised with a message alone carries its class's
  status** (`guatiao.BadValue("nobody to greet")`), which is how Python
  code refuses a call.
- **An example host in Python**, `examples/python_host/`: it scans for
  greeter libraries in any language, or adds the `guatiao-py` bridge, and
  calls every slot of the kind, objects included. The greeter kinds are
  Python classes beside their C header,
  `examples/greeter_kind/python/greeter_kind.py`, and
  `examples/python_provider/` holds two libraries written in Python.
- **Node.js and TypeScript bindings**, `bindings/node/guatiao/` (npm `guatiao`):
  typed ESM over koffi, for Node 22 and later, shipping no native code.
  `Value`, `Ref`, `MapRef` and `ListRef` over a tree (numbers keep their
  text and read as `number` or `bigint`; absent is `undefined`), `using`
  or `close()` to free; `guatiao/serde` (`loads`, `dumps`, `ParseError`),
  `guatiao/intake`, `guatiao/time` (`PlainDate`, `PlainTime`,
  `PlainDateTime`, `Span`, instants as `Date`, exact lengths as
  `{ seconds, nanoseconds }`), `redact`, and the status errors. The
  library is found through `GUATIAO_LIBRARY` or `useLibrary(path)`.
- **Node: the registry, kinds and providers.** `Registry` (scan, load,
  rank, `create`, `retire`, `collect`, `unload`, `addSourceFile`,
  `watchDir`, `refresh`) and `Instance`; `guatiao/kinds`: a kind is a
  typed value, `kind('greeter', { floorHash, methods: { greet: { args:
  [t.str], answer: t.map, required: true, fallible: true } } })`, whose
  `Implementation` a provider class implements and whose `Remote` a
  caller holds, every slot shape both ways, objects of object kinds
  handed over, lent and answered; `guatiao/provide`: `class Hello extends
  Provider implements Greeter` with `static kinds`, configurations from
  `s` schemas, `library(id, version, { providers })`, and
  `libraryOf(module)`, an ES module of a package as a library named and
  versioned by its `package.json`. A provider's answer is built in the
  allocator the caller's carrier names; a call from a thread Node did
  not start runs on Node's main thread when its event loop turns.
- **`examples/greeter_caller/`**, a cdylib whose C functions,
  `greeter_caller_run(host, key, name, out)` and
  `greeter_caller_wide(host, key, out)`, call every slot of a greeter, or
  of a `wide` provider with 64-bit integers at their limits, through the
  kind's Rust proxy, for a host in another language to have its providers
  called from Rust; and the Node examples,
  `examples/node_host/` (a host in TypeScript) and
  `examples/node_provider/` (a library written in TypeScript), with
  the greeter kinds described in `examples/greeter_kind/node/`.
- **Go bindings**, `bindings/go/guatiao/` (module
  `github.com/Boriken-Dev/guatiao/bindings/go/guatiao`): cgo over the ABI's C
  header, for Go 1.26 and later, loading the library at run time
  (`GUATIAO_LIBRARY` or `UseLibrary(path)`) and shipping no native code.
  `Value`, `Ref`, `MapRef` and `ListRef` over a tree (numbers keep their
  text as `Number`), `Close` to free; Go data converts both ways as
  `encoding/json` does, structs by their `guatiao` or `json` tags,
  `time.Time` as an instant and `time.Duration` as an exact length;
  `SchemaOf[T]()`, the JSON Schema a configuration struct describes;
  `serde` (`Parse`, `Emit`, `ParseError`), `intake`, `civil` (`Date`,
  `Time`, `DateTime`, `Span`, `Length`), `Redact`, and the status errors
  matched with `errors.Is`.
- **Go: the registry, kinds and providers.** `Registry` (scan, load,
  rank, `Create`, `Retire`, `Collect`, `Unload`, `AddSourceFile`,
  `WatchDir`, `Refresh`) and `Instance`. A kind is a Go interface marked
  `//guatiao:kind greeter greeter_vtable`, each optional slot an
  interface of its own; the command `guatiao-kind`, run by `go generate`,
  writes from the kind's C header the descriptor (`GreeterKind`), a
  remote implementing every interface (`GreeterRemote`) and the cgo
  calling and serving each slot. A provider is any Go type implementing
  the interface, named with `Single` or `Configured` (built from a
  configuration struct, its schema from the struct's tags) in a library
  `LibraryOf` names by the providers' package and versions from the build;
  registered in the process, or built with `-buildmode=c-shared` as a file
  any host's scan loads, which is never unmapped. A slot runs on the
  thread that calls it and builds its answer in the allocator the
  caller's carrier names. The Go examples: `examples/greeter_kind/go/`,
  `examples/go_host/` and `examples/go_provider/`, a module of their own.
- **Julia bindings**, `bindings/julia/Guatiao/` (package `Guatiao`): `ccall` over
  the ABI, for Julia 1.10 and later, with nothing to compile and no
  dependency outside the standard library; the library is found through
  `GUATIAO_LIBRARY` or `Guatiao.use_library(path)`. `Value`, `ValueRef`,
  `MapRef` (an `AbstractDict`) and `ListRef` (an `AbstractVector`, from
  1) over a tree, `close` to free; Julia data converts both ways
  (`Dict`s, vectors, `NamedTuple`s, structs, enums), `decode(T, n)`
  reading integers exactly, numbers keeping their text as `NumberText`;
  `Guatiao.schema(T)`, the JSON Schema of a type, and `Guatiao.@config`,
  `@kwdef` that also puts defaults and docstrings in it; `Dates.Date`,
  `Dates.Time`, `Dates.DateTime`, `Timestamp`, `Span` and `Length`;
  `Serde`, `Intake`, `redact`, and `GuatiaoError` with the status table.
- **Julia: the registry, kinds and providers.** `Registry` (scan, load,
  rank, `create`, `retire!`, `collect!`, `unload!`, `add_source_file!`,
  `watch_dir!`, `refresh!`) and `Instance`. `@kind` declares a kind from
  its C header: an abstract type, a generic function per slot, and a
  remote subtype calling the slots. A provider is a struct with methods
  for its kinds' functions, declared with `@provider` (`serves`,
  `config`, `available`, ...), in a library `library_of` reads from a
  package: id from the module, version, authors and licence from its
  `Project.toml`. A slot runs on the thread that calls it, a thread Julia
  did not start included, and builds its answer in the allocator the
  caller's carrier names; every call that can reach a library is made in a
  GC-safe region. The Julia examples: `examples/greeter_kind/julia/`,
  `examples/julia_host/` and `examples/julia_provider/`.
- **R bindings**, `bindings/r/guatiao/` (package `guatiao`, version `0.0.0.1`
  for this alpha: R takes no letters in a version, so `0.0.0-alpha.N` is
  written `0.0.0.N`): a source package for R 4.2 and later whose C glue,
  compiled when it installs, needs libffi and opens the library found
  through `GUATIAO_LIBRARY` or `use_library(path)`. `value()` builds a
  tree from R data (named lists are maps, unnamed lists and vectors are
  lists) and `$`, `[[`, `names`, `length`, `as.list`, `value_data()` read
  it, with `push`, `insert`, `delete`, `clear` and `merge_from` to change
  it; numbers keep their exact text (`number()`) where a double would
  lose it. `schema_of()` writes the JSON Schema of an S4 class and
  `from_value()` reads a value back as one; `config_class()` adds
  defaults and docs. `Date`, `civil_time`, `civil_datetime`, `instant`,
  `span` and `exact_length`; `serde_parse`/`serde_emit`, the `intake_*`
  functions, `redact`, and `guatiao_error` with the status table.
- **R: the registry, kinds and providers.** `registry()` answers an
  object whose methods (`$scan_dir`, `$providers`, `$bind`, `$create`,
  `$retire`, `$collect`, `$unload`, `$add_source_file`, `$watch_dir`,
  `$refresh`) host libraries. `kind()` declares a kind from its C header:
  a virtual S4 class, a generic per slot (`method()`, with `lent()` and
  `taken()` shapes) and a remote class calling the slots. A provider is an
  S4 or Reference class containing its kinds with methods of their
  generics, declared with `provider()`, in a library `library_of()` reads
  from a package: id, version, authors and licence from its `DESCRIPTION`,
  its licence and notice files as notices. A slot an R object serves runs
  on R's main thread only: called from any other thread it answers
  `Unsupported` without running, and a destroy asked for there waits for
  R's thread. The R examples: `examples/greeter_kind/r/`,
  `examples/r_host/` and `examples/r_provider/`.
- **R: 64-bit integers cross exactly.** A kind's integer slots take an R
  integer, a whole double, a `number()`, a string spelling an integer or a
  `bit64::integer64`, read exactly and range-checked, a refusal naming the
  slot (`wide.echo_i64(x): out of range for int64`). A 64-bit answer, and
  a 64-bit argument a provider written in R is handed, is what `int64`
  says on `kind()` or `call_slot()`: `"auto"` an R integer when it fits, a
  double up to 2^53, else a `number` with its exact text; `"number"`;
  `"double"`; or `"integer64"`. `span()` and `exact_length()` keep a unit
  past 2^53 as its exact text, and `time_parse("span")` and
  `read_length()` answer one rather than refusing it. `bit64` is
  suggested, never needed: `value_data(numbers = "integer64")` and
  `int64 = "integer64"` answer an `integer64` and name the package when
  it is not installed, and `"auto"` never answers one.
- **A `wide` example kind**, `examples/greeter_kind/`: one slot taking and
  answering an `int64_t`, one a `uint64_t`, served by the derived Rust
  greeter and the greeter written in C, so a binding can cross 64-bit
  integers at their limits both ways.
- **Every binding crosses 64-bit integers through the `wide` kind.** Go
  (`greeterkind.Wide`, written by `go generate`), Julia, MATLAB and Octave
  and Fortran declare it beside the other example kinds. The suites of
  Python, Node, Dart, Go, Julia, MATLAB and Octave, Fortran and C++ (R's
  already did) send 2^53 + 1, INT64_MIN and INT64_MAX through `echo_i64`
  and 2^53 + 1, INT64_MAX + 1 and UINT64_MAX through `echo_u64` to the
  provider written in Rust, the one in C and one written in the binding,
  exactly; where a suite has Rust call its provider, `greeter_caller_wide`
  reads the same six back. A value that does not fit is refused naming the
  call in Python (`OverflowError`), Node (`RangeError`), Julia
  (`InexactError`) and MATLAB and Octave (`guatiao:BadValue`); Go and C++
  take the slot's own 64-bit types. Dart and Fortran have no unsigned
  64-bit integer: a `uint64` is the signed integer holding its bits,
  negative past 2^63 - 1 (UINT64_MAX is -1; in Dart,
  `BigInt.from(x).toUnsigned(64)` is the number).
- **MATLAB and GNU Octave bindings**, `bindings/matlab/guatiao/`: the `+guatiao`
  package, `classdef` and functions that MATLAB R2024a and later and
  Octave 8.4 and later both run, over one MEX gateway in C built on the
  user's machine (`pkg install` in Octave, `guatiao.build()` in MATLAB)
  that needs libffi and opens the library found through `GUATIAO_LIBRARY`
  or `guatiao.use_library(path)`. `guatiao.Value` builds a tree from
  structs, cell arrays, text, numbers, `uint8` and `containers.Map`, with
  views (`v.at('tags')`) to read and change it in place and `data()` to
  read it back; an integer past 2^53 reads as `int64`/`uint64` and a wider
  number as `guatiao.Number`, its text kept. `guatiao.Config` and
  `guatiao.field` write a configuration's JSON Schema and read one back
  with its defaults. `guatiao.Date`, `TimeOfDay`, `DateTime`, `Timestamp`,
  `Span` and `Duration`, with MATLAB's `datetime` and `duration` written;
  `guatiao.serde.parse`/`emit`, the `guatiao.intake` functions,
  `guatiao.redact` and `guatiao.validate`. Errors carry identifiers
  `guatiao:<Status>`. The Octave package and the MATLAB toolbox are built
  as release assets.
- **MATLAB and Octave: the registry, kinds and providers.**
  `guatiao.Registry` hosts libraries (`scan_dir`, `providers`, `bind`,
  `create`, `retire`, `unload`, `add_source_file`, `watch_dir`,
  `refresh`). A kind is a class inheriting `guatiao.Kind`, described once
  with `guatiao.Kind.describe` and `guatiao.slot` (`guatiao.lent`,
  `guatiao.taken`), with a method per slot; a remote is an instance of it.
  A provider is a handle class inheriting its kinds and overriding their
  methods, marked by a `Provider = guatiao.provider(...)` constant, in a
  library `guatiao.library_of` reads from a folder: id, version, authors
  and licence from its `DESCRIPTION`, its licence and notice files as
  notices. A slot M code serves runs on the interpreter's thread while a
  call into guatiao is in progress (`guatiao.serving` makes one for C code
  of one's own); otherwise it answers `Unsupported` without running, and
  a destroy asked for there waits for the next call. The examples:
  `examples/greeter_kind/matlab/`, `examples/matlab_host/` and
  `examples/matlab_provider/`.
- **Fortran bindings**, `bindings/fortran/guatiao/`: the fpm package `guatiao`
  (a git dependency on the repository, whose root `fpm.toml` builds it)
  and the CMake target `guatiao::fortran`, pure Fortran 2008 over
  `iso_c_binding`, tested with gfortran, LLVM flang and Intel ifx. The
  library is opened at run time (`GUATIAO_LIBRARY` or
  `guatiao_use_library(path)`), so a program links nothing of it. A
  `guatiao_value` is a counted handle on a tree: copies share it, the last
  one going or `close()` frees it, and `set`, `push` and `get` are generic
  over Fortran's types, numbers keeping their exact text and reals written
  as their shortest round-trip text. `guatiao_config` is a configuration
  type that describes its schema field by field and decodes the checked
  value; `guatiao_validate`, `guatiao_redact`, `guatiao_serde`
  (`guatiao_parse`/`guatiao_emit`), `guatiao_intake` and `guatiao_civil`
  (dates, times, instants, spans, exact lengths). Every call takes an
  optional `stat`/`errmsg` pair.
- **Fortran: the registry, kinds and providers.** `guatiao_registry`
  hosts libraries (`scan_dir`, `providers`, `create`, `retire`, `unload`,
  `add_source_file`, `watch_dir`, `refresh`), every answer a value in its
  allocator. A kind is an abstract type extending `guatiao_impl` with a
  deferred procedure per required slot, written once in a kind's module
  with a remote type and `bind(c)` glue; a slot method reports through an
  allocatable `guatiao_error`. A provider is a type extending the kind,
  its optional slots declared where it is registered; a library is
  `guatiao_new_library(id, version)` with `single` and `configured`
  providers, registered in process (`guatiao_register_library`) or built
  as a library file whose entry and `guatiao_declares` are written in
  Fortran. A served slot runs on the thread that calls it; a Fortran
  library is never unmapped. The examples: `examples/greeter_kind/fortran/`,
  `examples/fortran_greeter/` and `examples/fortran_host/`.
- **The C headers are tested as C++.** `guatiao.h`, `guatiao_serde.h` and
  `guatiao_intake.h` compile alone and together as C++11, C++17 and
  C++20 with warnings as errors (C++14 under MSVC's `cl`, its oldest
  mode), and a C++ program calling one function from each links and
  runs. `GUATIAO_CXX` picks the compiler; `cl` and `clang-cl` take MSVC
  flags.

### Changed

- **`guatiao_intake.h` names its separator `GUATIAO_INTAKE_SEPARATOR`.**
  The bare `SEPARATOR` macro took the name from every C and C++ file
  including the header; the value is still `'.'`, and Rust's
  `guatiao_intake::SEPARATOR` keeps its name. Dart's generated constant is
  renamed to match.

- **`guatiao-py` is CPython as a `guatiao-bridge` engine.** Its
  manifests, stamps, refresh and source table are the bridge's, and its
  entry point is written by `guatiao::source!`; it keeps the interpreter,
  reading a module's `__version__`, and finding its own file. What a host
  sees is unchanged: the same configuration, manifests, statuses and
  words. *Migration:* none.
- **`guatiao-serde` writes a value through its walk**, so a value nested
  deeper than `MAX_DEPTH` containers is refused (`ValueError::TooDeep`
  in the format's error) where serde's own recursion used to follow it,
  and a node a foreign producer wrote wrong is refused in the value
  model's words (`not valid UTF-8`, `not a JSON number`). *Migration:*
  none for a tree built in Rust or read from a document, which never
  nests that deep.
- **Breaking: the Dart packages need Dart 3.10** (Flutter 3.38), the
  first with build hooks; they asked for 3.6. *Migration:* raise the
  application's `sdk:` constraint to `^3.10.0`.
- **Breaking: every binding's packages live in
  `bindings/<language>/<name>`**, each directory named as its registry
  spells the package: `bindings/python/guatiao`, `bindings/dart/guatiao_serde`,
  `bindings/node/guatiao-intake`, `bindings/julia/GuatiaoSerde`,
  `bindings/r/guatiao.intake`, `bindings/matlab/guatiao`,
  `bindings/fortran/guatiao` and so on; the Node packages are developed as
  the workspace `bindings/node`. *Migration:* only what names a path in
  the repository changes. The Go modules are
  `github.com/Boriken-Dev/guatiao/bindings/go/guatiao`,
  `.../bindings/go/guatiao-serde` and `.../bindings/go/guatiao-intake`
  (below), each tagged `bindings/go/<module>/vX.Y.Z`: change the imports
  and `go get` the new paths. A Julia or R install from the repository
  names the new subdirectory (`subdir = "bindings/julia/Guatiao"`,
  `remotes::install_github("Boriken-Dev/guatiao", subdir =
  "bindings/r/guatiao")`); a CMake `add_subdirectory` or `SOURCE_SUBDIR`
  names `bindings/fortran/guatiao`. An fpm git dependency is unchanged: the
  root `fpm.toml` follows the sources.
- **Breaking: JSON, TOML and YAML, and forms, are packages of their own in
  every binding**, each over its own library (`guatiao_serde`,
  `guatiao_intake`) and built on the core package, which offers what they
  need as documented API. Each finds its library through its own variable
  (`GUATIAO_SERDE_LIBRARY`, `GUATIAO_INTAKE_LIBRARY`), then
  `GUATIAO_LIBRARY` (a directory, or a file that exports the surface, else
  the file beside it), then the platform's search. *Migration:*
  - Python: `pip install guatiao-serde guatiao-intake`; `from guatiao
    import serde` is `import guatiao_serde as serde`, `from guatiao import
    intake` is `import guatiao_intake as intake`, and
    `guatiao.errors.ParseError` is `guatiao_serde.ParseError`.
    `guatiao.ffi` is new: the loader, the C structs and a tree crossing as
    its root node, for a package over another guatiao library.
  - Dart: `dart pub add guatiao_serde guatiao_intake`;
    `package:guatiao/serde.dart` is
    `package:guatiao_serde/guatiao_serde.dart` (`Format` and `ParseError`
    with it), `package:guatiao/intake.dart` is
    `package:guatiao_intake/guatiao_intake.dart`. `package:guatiao/ffi.dart`
    is new, and the ffigen symbol file is
    `package:guatiao/ffi_symbols.yaml` (it was `package:guatiao/src/symbols.yaml`).
  - Node: `npm install guatiao-serde guatiao-intake`; `guatiao/serde` is
    `guatiao-serde` (`ParseError` with it) and `guatiao/intake` is
    `guatiao-intake`, each with its own `useLibrary`. `guatiao/ffi` is
    new: `openSurface`, memory, `adopt`, `borrow`, `nodeOf`, `withNodes`
    and the one koffi, for a package over another guatiao library.
  - Go: three modules, one per crate, each with its own tag and
    requiring the core at the same version:
    `github.com/Boriken-Dev/guatiao/bindings/go/guatiao` (package
    `guatiao`), `.../bindings/go/guatiao-serde` (package `serde`) and
    `.../bindings/go/guatiao-intake` (package `intake`). `go get` the
    serde and intake modules; the import
    `"github.com/Boriken-Dev/guatiao/bindings/go/serde"` (or
    `".../bindings/go/guatiao/serde"`) is
    `serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"`, and
    `".../bindings/go/intake"` (or `".../bindings/go/guatiao/intake"`) is
    `intake "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-intake"`;
    the package names are unchanged, so the code using them is too. Each
    finds its own library and has its own `UseLibrary`:
    `guatiao.UseLibrary` names only the core's now (a file it named was
    the place the others were looked for beside). The core exports
    `Surface` (a guatiao C library other than its own: found, opened, its
    exports resolved), `Load`, `Adopt`, `WithNodes`, `NodeOf`, `Borrow`
    and `StatusError` for a package over another guatiao library.
  - Julia: `Guatiao.Serde` is the package `GuatiaoSerde` and
    `Guatiao.Intake` is `GuatiaoIntake` (`bindings/julia/GuatiaoSerde`,
    `bindings/julia/GuatiaoIntake`); `ParseError` moves to `GuatiaoSerde`.
    `Guatiao.FFI` is new: `@cdef` usable from any module, `surface!`,
    `find_library` and trees as root nodes.
  - R: `serde_parse`/`serde_emit` are the package `guatiao.serde` and the
    `intake_*` functions `guatiao.intake` (`bindings/r/guatiao.serde`,
    `bindings/r/guatiao.intake`), with the same names; `library(guatiao.serde)`
    attaches `guatiao` too. `use_library(path, library = "guatiao")` names
    any of the three libraries. `guatiao` registers the C table
    `guatiao_r_api` (`include/guatiao_r_api.h`) for a package over another
    guatiao library.
  - MATLAB and Octave: `guatiao.serde` and `guatiao.intake` stay
    sub-packages of the one toolbox and Octave package (one gateway), each
    calling its own library: they gain their own `use_library`, and
    `guatiao.use_library` names only the core's (a file it named was the
    place the others were looked for beside).
  - Fortran: `guatiao_serde` and `guatiao_intake` stay modules of the one
    fpm package (a git dependency reads only a repository's root manifest),
    each over its own library, and the core modules no longer use them:
    they gain `guatiao_serde_use_library` and
    `guatiao_intake_use_library`, and `guatiao_use_library` names only the
    core's.
- **Breaking: `clone()` copies into this binary's allocator.** `clone()`
  (and `to_owned()`) on `Value`, `Map`, `List`, `Text`, `Buffer` and
  `Number` builds the copy through `Alloc::rust()`, whatever allocator the
  source recorded, so a copy of a tree that lives in an arena, a caller's
  allocator or a library's never depends on it. *Migration:* a copy that
  must live beside its source is `v.clone_in(v.alloc()?)`.

### Fixed

- **Go: a tree a kind call borrows or lends stays alive until the call
  ends.** Glue that did not use its argument again let the collector
  free a `Value` while the slot was reading or changing it; the call now
  holds the tree's owner until it ends.

- **Reading a wide map is linear in its keys.** Every reader of wire
  bytes (`decode`, `decode_as`, `Decoder`, walking `Encoded` bytes,
  `guatiao_wire_decode`, `guatiao_wire_visit`) checked each key against
  every earlier key of its map, and a `Value` built from a walk
  (`decode_as::<Value>`, `TreeSink`, `guatiao_value_from_walker`, and
  through them JSON, TOML and YAML read by guatiao-serde) looked each key
  up the same way. So a document anyone wrote could make a reader spend
  seconds on a few hundred kilobytes and hours on megabytes. Past 16 keys
  a map's keys are now found through a hash. A repeated key is still
  refused on the wire at the byte where it starts, and where a format
  allows one it still replaces the first in its place.

- **On Windows a library's own directory is searched for what it imports.**
  A plugin linking `guatiao.dll` shipped beside it, in a directory of its
  own, failed to load: Windows looked beside the host and on `PATH` only.
  A load that fails is now tried again with the library's directory in
  place of the host's, `PATH` still searched.

- **CPython served by `guatiao-py` inside a Node host frees what a call
  drops, on Linux.** koffi runs each call on a stack of its own, which
  CPython 3.14 took for an overflowing one, deferring every deallocation
  for good: a closed Python conversation kept its listener forever. The
  bridge now tells CPython which stack the call runs on. macOS keeps the
  old behaviour.

- **R and MATLAB name a scan of a file as no directory.** A refused scan
  always suggested a bad rule; a path that is a file or missing now says
  so (`NotFound`), and a bad rule still names the rule (`BadValue`).
- **The Fortran binding built by CMake with LLVM flang finds serde and
  intake beside a backslash path, and quotes keys with one backslash.**
  flang's preprocessing doubled each backslash written as a literal.

- **On FreeBSD a C program needs no `-pthread`.** A program built
  without it bound libc's failing thread stubs, and the library aborted
  with `fatal runtime error: out of TLS keys`. Every library that links
  guatiao now names libthr itself (`NEEDED libthr.so.3`).
- **A `with` field's default is written through its module.** The schema
  wrote a `#[map(default)]` or `#[map(default = ..)]` beside `with`
  through the field type's own `ToValue`, so a type with no value impls
  could not take a default.
- **A tree a Rust provider answers reaches a caller in any language in
  the caller's allocator.** A slot `#[guatiao::kind]` generates called
  `<method>_in` with the carrier's allocator and wrote what it answered
  as it was, so a provider implementing only `<method>` (whose default
  `_in` calls it) answered a C, Python or Dart caller a tree in the
  library's own allocator, which freeing after the library was unloaded
  read from unmapped memory. The shim now moves the answer into the
  carrier's allocator first; only a Rust caller's proxy did before.
- **An instance built through the C surface is released by the provider
  that built it**, and holds its library open until then.
  `guatiao_registry_provider_destroy` found the provider by key, so after
  a retire it released nothing (and the library's own count then refused
  every unload and collect), and after a refresh loaded a new build under
  the key it called the new build's `destroy` with the old build's
  instance. The registry now keeps the building provider per instance:
  `unload` answers `GUATIAO_ERR_BUSY` while one lives, a retired library
  closes once the last is released, and Python's and Dart's
  `Instance.close()` work after a retire.

## [0.0.0-alpha.1] - 2026-10-19

### Changed

- **Breaking: every handle holds a lease on its library, and `Remote` is
  `Clone`, not `Copy`.** A registered library has a lease, an `Arc`
  holding the registry's own copies of its tables (each per-kind table
  and the shared `vtable`, copied under the size its descriptor declares)
  and, for a library the host built, its `LibraryParts`. `Remote`,
  `Offer`, `Instance`, `Object` and `ProviderView` hold a share, and
  their table pointers address the copies; the code those tables name
  and `ctx` are kept mapped by the lease. `Remote` stays `Send + Sync +
  'static`; an `Object` a method returns holds the lease of the `Remote`
  it came through, and `into_raw` lets it go. `Instance::remote()`
  answers `&Remote<K>`, borrowed from the instance it calls. *Migration:*
  a `Remote` used after a move is `.clone()`d; `let r = instance.remote()`
  that needs an owned proxy is `instance.remote().clone()`, valid while
  the instance lives.
- **Breaking: `Registry::unload` is safe, and refuses while a handle
  holds the library.** It answers the new `UnloadError::Busy { key,
  leases }` while any `Remote`, `Offer`, `Instance`, `Object` or
  `ProviderView` taken from the library, or an answer another library
  pinned through the host's services, is alive; then the library's own
  `unload` slot is asked (`Refused`, `NotSupported`), then the mapping is
  closed, and every refusal leaves the library registered. The unsafe
  `unload_unchecked` skips the lease count and a missing slot; a slot's
  refusal still stands. `guatiao_registry_unload` answers
  `GUATIAO_ERR_BUSY` for a held lease too, and Python and Dart raise
  `Busy`. What stays the caller's word is a raw pointer taken on purpose
  (`vtable()`, `table()`, `ctx()`) and kept past the handle it came
  from. *Migration:* drop the `unsafe { }` around
  `unload`; a match on `UnloadError` gains `Busy`; drop what you took
  from a library, or retire it and `collect`, before unloading.
- **A retired library is closed once nothing uses it.** The new
  `Registry::collect() -> Vec<String>` closes every retired library whose
  lease no handle holds and whose `unload` slot agrees, and answers their
  keys; the registry's next `load_file`, `register_local`,
  `register_entry`, `retire` or `unload` collects as well. One that
  refuses stays retired and mapped and is asked again; one with no slot
  stays mapped. Dropping a handle never closes anything, and dropping the
  registry never unmaps. C: `guatiao_registry_collect(reg, alloc, out)`
  writes the keys as a list; Python and Dart `Registry.collect()` return
  them. *Migration:* a raw table or `ctx` taken from a library is valid
  until it is closed, which after a retire may be the registry's next
  call: let it go first.
- **What a call through a kind returns lives in the host's allocator, and
  is built there when the provider can.** Before a call answering a
  `Value`, `Map` or `List`, or a type crossing through `ToValue`, the
  proxy writes into the out slot a carrier: an empty container recording
  the allocator the answer is wanted in, which allocates nothing (for a
  `Value`, ABSENT whose map arm records it). `#[guatiao::kind]` adds
  `<method>_in(.., alloc: Alloc)` to the trait beside every method
  answering a tree; the shim calls it with the carrier's allocator, and its
  default answers `<method>(..)`. A provider that implements `<method>_in`
  with the `_in` forms hands its tree over as it is, and a host can call
  `<method>_in` itself to have the answer built in an arena of its own.
  Any other tree, and a `ProviderError`'s message, is moved by the proxy
  into the carried allocator, the original freed through the allocator
  that made it while the call holds the library's lease. So nothing a host
  keeps from a call needs the library's allocator once the library is
  gone. A C provider builds into the carrier by reading its `alloc`
  (documented beside `guatiao_kind_header` and on each kind's table); a C
  caller of a table passes an initialised out slot, a carrier or zeroes,
  which a shim falls back from to its own allocator. *Migration:* a kind
  method named `<m>_in` beside a method `<m>` answering a tree is refused;
  rename it.
- **Breaking: a host's services answer descriptors the registry built,
  and can pin them.** `get` and `list` answer a descriptor per provider
  that the library's lease built over the registry's copies (its own
  text, values and tables, the library's code and `ctx`), valid until the
  library is retired, rather than the library's own. `HostServices` gains
  four appended slots: `get_pinned` and `list_pinned` answer the same with
  a hold on the library taken for the caller, `pin` takes another and
  `unpin` gives one back. `Host::get` and `Host::list` use them, so a view
  a library reads, and every `Offer` and `Remote` taken from it, keeps the
  provider's library loaded while it lives; from a host without them a
  view copies the tables. This also ends a library inside `get` or `list`
  reading a descriptor another thread had just retired and freed.
  `ProviderView::raw` is gone. *Migration:* a library that kept a
  descriptor pointer from a C `get`/`list` past a call, or that may race
  a retire, uses `get_pinned`/`list_pinned` and `unpin`.
- **Breaking: the descriptor doors take ordinary borrows.**
  `ProviderInfo::{view, as_kind, offer, instantiate}` take `&self` on any
  `ProviderInfo<'_>`, since nothing is kept of the descriptor but code,
  `ctx` and copied tables. `available()` on `ProviderView`, `Provider`
  and `Offer` answers `Result<(), &str>`, the reason borrowed while the
  handle holds its library. *Migration:* a caller that stored the reason
  as `&'static str` stores `reason.to_string()`.
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
- **Breaking: `ToValue::to_value_in(alloc)` and `Schema::schema_in(alloc)`
  take the allocator, and `to_value()` and `schema()` take none**, using
  the crate's own, as building does everywhere else. The derives write
  `to_value_in`. *Migration:* a hand-written `impl ToValue` renames its
  `to_value(&self, alloc)` to `to_value_in`; a call
  `x.to_value(Alloc::rust())` is `x.to_value()` and `T::schema(Alloc::rust())`
  is `T::schema()`; a call naming another allocator is `to_value_in(alloc)`
  or `schema_in(alloc)`.
- **Breaking: a descriptor is checked where it enters, and refused with
  a sentence.** Loading or registering a library is the trust decision, so
  every door that reads a descriptor stays safe (`load_file`,
  `register_local`, `register_entry`, `ProviderInfo::{view, as_kind, offer,
  instantiate}`, `ProviderPartsBuilder::build`) and its fields are its
  producer's contract. What can be checked now is, before anything is read
  through it: every non-null pointer aligned for what it points at, outside
  the low 64 KiB and canonical for the architecture; every length × stride
  addressable and capped (4096 providers; 256 kind names and 256 per-kind
  tables per provider; strides of at most 4096 bytes; text and tables of at
  most 64 KiB); each per-kind table's slots null or a plausible function
  pointer. For a library mapped from a file the loader places its image
  from the platform loader's own record of the mapping, without reading
  the file again (the PE headers at the module's base on Windows, the
  program headers `dl_iterate_phdr` reports on ELF platforms, the Mach-O
  header `dladdr` finds on Apple platforms; elsewhere the file's layout,
  placed where its entry symbol landed), and also refuses a data pointer
  that starts inside the image and runs past it, and a function pointer
  (`available`, `create`, `destroy`, `unload`, a table slot) outside the
  image's executable ranges: sections on Windows, segments on ELF and
  Mach-O. `LoadError::Malformed` gains
  `reason`, naming the part and the fault; `KindMismatch` gains
  `Misaligned` (the table's address, checked by every `Remote`) and
  `Unreadable(reason)` (`ProviderInfo::{as_kind, offer}` over a refused
  descriptor). *Migration:* a `LoadError::Malformed { path }` pattern adds
  `..`; a library a check now refuses fixes what `reason` names.
- **Breaking: `Registry::register_local` takes any `FnOnce(Host) ->
  Option<L>` with `L: Into<LocalLibrary<'a>>`, borrows a descriptor for
  the registration only, and owns a host's own library.** `LocalLibrary<'a>`
  is `Owned(Box<LibraryParts>)` or `Borrowed(&'a LibraryInfo<'a>)`, from
  `LibraryParts`, `Box<LibraryParts>`, `&'a LibraryInfo` or
  `&'a LibraryParts`. The registry copies what a descriptor says (text,
  values, tables) at the registration and keeps only its providers' code
  and `ctx`; owned parts belong to the library's lease and are freed once
  it is retired and its last handle dropped. `local_providers!` writes
  `library(Host) -> Option<LibraryParts>`, built on each call and kept
  nowhere. *Migration:* a describe answering `Option<&'static
  LibraryInfo<'static>>`, and `register_local(name, library)`, compile
  unchanged; a describe that leaked its parts into a `OnceLock` can return
  them by value or by a borrow; a caller of `library(host)` itself gets
  `LibraryParts` (`.info()` for the descriptor).
- **Breaking: `Object::from_cell` takes a cell whose table is paired with
  its value by type.** It is `from_cell(Box<ObjectCell<K::Vtable, T>>)`
  where `K::Vtable: TableFor<T, Kind = K>`; `ObjectCell`'s fields are
  private and `ObjectCell::new(value)` fills the table through the hidden
  `TableFor` the kind attribute implements, so a table built for another
  type or kind does not compile. `into_object` is unchanged. *Migration:*
  `ObjectCell { table: V::of::<T>(), value }` is `ObjectCell::new(value)`.
- **Breaking: `ProviderView`'s fields are private**, so a view can only
  come from reading a descriptor and its safe `available()` cannot be
  pointed at another `ctx`. Migration: `view.id` is `view.id()`, and so
  on for `kinds`, `display_name`, `config`, `meta`, `version`, `vtable`,
  `vtable_size`, `ctx`, `tables`, `create` and `destroy`
  (`version()` is `Option<&str>`, `config()`/`meta()` are
  `Option<&_>`).
- **Breaking: `Host::get` and `Host::list` answer `ProviderView`s**, read
  under the size each descriptor declares, rather than `&'static
  ProviderInfo`: a reference to an older library's shorter descriptor
  claimed bytes it never wrote. `Host::mismatches` pairs a view with its
  mismatch. Migration: drop the `.view()` call.
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
  `.build()`. Every slot defaults to absent, so a provider
  written without the derive keeps compiling when the envelope appends
  one. Migration: `ProviderParts::new(id, name, version, kinds, tables,
  config, ctx, available, create, destroy)` is
  `ProviderParts::builder(id).name(name).version(version).kinds(kinds)`,
  a `.table(..)` per table, `.ctx(ctx)`, a call per slot that is not
  `None`, then `.build()`.
- **An object kind is never served by a provider.** A registry or host
  answers the new `KindMismatch::ObjectKind` for one, and
  `#[provider(..)]` naming one is a compile error on the kind: its shims
  read `ctx` as an object one caller owns, and a provider's is shared.
- **Breaking: a typed value is checked as the type its kind names.**
  `validate_value` and `validate_map` required only that a scalar's text
  passed, so a boolean field took `yes`, a string field took `5900` and a
  number field took `inf` -- values the type's own `FromValue` refuses.
  Now a boolean field takes a boolean, an integer or number field a
  `Number`, a string field text, and `enum` is enforced by value on every
  type. A union tries each arm with the value itself, so a list is
  accepted by an array arm and text by no arm of `anyOf[integer,
  array<string>]`. Migration: build the value as the field's type (a
  `Number` for an integer, `Value::from(true)` for a boolean); a store of
  text keeps `validate_text`, which now reads a number as JSON writes one
  (`inf`, `+1`, `007` and `1e999` are refused).
- **Breaking: `schema::merge::merge_overrides` and `merge_options` return a
  `Result`**, and `merge_with_schema` can fail the same way: a schema
  nested past `MAX_DEPTH` is `MergeError::Build(ValueError::TooDeep)`
  instead of overflowing the stack. Migration: `?` or `.unwrap()`.
- **Breaking: merge paths are segments.** The top-level key `tls.ca` and
  the key `ca` inside `tls` were one leaf; `Provenance::source_of`,
  `Provenance::leaves`, `MergeOverrides::{set, with, get}` now take and
  return a list of segments. Migration: `source_of("tls.ca")` is
  `source_of(["tls", "ca"])`, `.with("tls.ciphers", m)` is
  `.with(["tls", "ciphers"], m)`, and `leaves()` yields `(&[String], &str)`
  (a list position is its decimal text). `MergeError::Kind::path` and the C
  override table's `path` use `guatiao-intake`'s path text (`tls.ca`,
  `peers[0].name`, `["a.b"]`); a plain dotted path is unchanged.
- **Breaking: the symbol `export_schema!` emits uses the crate's name**
  (`CARGO_CRATE_NAME`), as its documentation said. Migration: a crate
  `acme-net` exported `acme-net_conn_schema`, which C could not declare;
  it exports `acme_net_conn_schema`.
- **`SchemaBuilder::finish`, `KindBuilder::map` and arms refuse a field key
  declared twice** with `ValueError::DuplicateKey`, where the second
  silently replaced the first and `required` listed the name twice, and
  `KindBuilder::variant` refuses two arms stored as one discriminant the
  same way. The builders are `#[must_use]`. `ValueError` is
  `#[non_exhaustive]`, so a `match` already had a `_` arm.
- **Breaking: a schema that does not say accepts an undeclared key.** An
  object without `additionalProperties` accepts a key it does not
  declare, as JSON Schema reads it, and one whose `additionalProperties`
  is a schema checks the key's value against it; `false` refuses, and
  every object the builders and the derive seal writes `false`, so their
  documents refuse what they did. Migration: a hand-written schema that
  relied on the refusal writes `"additionalProperties": false`.
- **Breaking: numeric bounds are compared as the exact text that declared
  them, and every JSON Schema bound is enforced.** `exclusiveMinimum` and
  `exclusiveMaximum` were carried and ignored; a bound past `i64` (`1e18`
  on a long integer, `u64::MAX`) was read as no bound. An `enum` beside a
  `type` now applies both, a `type` admitting null beside an `enum` admits
  null only as a member, and below a minimum of zero an integer spelled
  with a minus sign (`-0`) is refused, as an unsigned type refuses it.
  Migration: none for a value the type it describes reads; a value one of
  these keywords refuses was accepted by mistake.
- **Breaking: a derived schema says what `FromValue` reads.**
  `Option<T>`'s kind is `T`'s accepting null as well -- `type: [T,
  "null"]`, or `anyOf: [T, {"type": "null"}]` for an enum, a union or a
  variant -- so an explicit null field, a `None` in a `Vec<Option<T>>` and
  a `None` value in a map of `Option<T>` validate. Integer widths write
  their bounds as exact digits (a `u64` field's `maximum` is
  `18446744073709551615`, `i128`'s `minimum` its own), and `f32` writes
  the exclusive bounds past which no finite `f32` exists, so `2^64` for a
  `u64` and `1e39` for an `f32` are refused by both. `Kind::Int` reads
  `None` for a bound past `i64`, as before. Migration: a consumer reading
  a derived document's `type` as one string reads it through
  `FieldRef::kind` and `is_nullable`, which take either shape.
- **`Kind` gains `Null`** (`#[non_exhaustive]`, so a `match` already had a
  `_` arm), and an integer `enum` reads as `Kind::Enum`, not `Kind::Int`.
  A non-integral integer bound rounds inward (`minimum: 1.5` is 2) instead
  of being dropped.
- **A `pattern` matches ECMA-262's ASCII `\d \D \w \W`**, where the `regex`
  crate's are Unicode (Arabic-Indic digits matched `\d`). `vocab::PATTERN`
  lists the differences that remain.
- **A kind's table hash reads standard and `guatiao` paths as their bare
  names.** `std::vec::Vec<T>` and `Vec<T>`, `guatiao::library::ProviderError`
  and `ProviderError` (and the same under a `crate =` root) now hash
  alike, so respelling a path in a kind crate no longer refuses every
  built provider. A type of the kind crate's own is still hashed by the
  name it is written with; the rule is in `guatiao-derive`'s `AGENTS.md`.
  *Migration:* breaking only for a kind whose required methods spelled
  one of those paths, or a reference's lifetime (`&'_ str` now hashes as
  `&str`): it gets a new `FLOOR_HASH` once, so rebuild its providers (a C
  table takes the new `*_FLOOR_HASH`). A kind spelled with bare names
  keeps its hash.
- **`#[guatiao::kind]` and `#[derive(Provider)]` refuse with a
  sentence what used to fail inside the expansion.** An argument named
  `ctx`, `out` or `err` now compiles (the shim's own parameters are
  `__ctx`, `__out`, `__err`, and the slot type renders it as `ctx_`).
  Types are read by path: guatiao's under `guatiao::`, `::guatiao::` or
  the `crate =` root, the standard ones under `std::`, `core::`,
  `alloc::`, both bare; a type ending in `Value`, `Map`, `List`,
  `String`, `Object` or `ProviderError` under another root is refused
  where it can only mean one of those -- borrowed, returned, as the error,
  or as `other::Object<dyn K>` -- naming the accepted spellings, and taken
  by value as an argument it crosses through `ToValue`/`FromValue` as any
  type of the caller's does (`proto::String`). Also refused: a method
  named `header`, one named `destroy` or `into_object` on an object kind,
  an argument whose name starts with `__`, and a kind named twice in
  `#[provider(..)]`. *Migration:* breaking only where such code compiled:
  an object kind's method named `destroy` or an argument starting with
  `__` is renamed, and `#[provider(K, K)]` drops the repeat.
- **Breaking: an `Object<dyn K>` is `Sync` only when `K`'s trait names
  `Sync`.** It was `Sync` for every kind, and an object's `&self` methods took
  the value as `&mut`, so a kind naming `Sync` called from two threads through
  `&dyn K` made two exclusive references at once. A `&self` slot of an object
  kind's table, and of its `&mut` lent table, now takes `&`; a `&mut self`
  slot still takes `&mut`. Slot signatures and table hashes are unchanged.
  *Migration:* share the handle of a kind naming `Send` alone through a
  `Mutex`, or name `Sync` on the kind.
- **A raw identifier is keyed without its `r#`.** A field `r#type` was
  written, read, described and given form hints under the key `r#type`;
  `ToValue`, `FromValue`, `Schema` and `Form` now key it `type`, as serde
  does. *Migration:* a stored value keyed `r#type` is read as `type`;
  rename the stored key, or keep it with `#[map(rename = "r#type")]`.
- **A `std::` path now marks something that needs the operating system.**
  Everything `core` or `alloc` also holds is spelled that way, enforced by
  the `std_instead_of_core`, `std_instead_of_alloc` and
  `alloc_instead_of_core` clippy lints for the whole workspace. Error types
  implement `core::error::Error`, and the derives and `providers!` reach
  `alloc` items through a hidden `guatiao::__private::alloc`, so the crate
  that expands them declares nothing.
- **`MergeMode::merge_layers` and `merge_layers_with` no longer declare an
  unused lifetime parameter**, `<'a, 'l>` is `<'l>`; no call spelled it, since
  the lifetime was never named by an argument.
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

### Added

- **`schema::redact(schema, &value, placeholder)`** (and `redact_in`):
  a copy of a value with every secret replaced -- each field whose schema
  sets `x-sensitive` or `writeOnly`, at any depth, through maps, lists and
  a variant's selected arm, and on the schema an optional wraps or any
  `allOf` member. A value a union accepts is hidden when any accepting arm
  is marked, and whole when the accepting arms would redact it
  differently. What the schema does not describe is hidden too, a
  discriminant that is not text included, and so is everything under a
  `$ref`: run `inline_refs` first.
- **`schema::inline_refs`** (and `inline_refs_in`): a copy of a schema with
  each local `$ref: "#/$defs/Name"` replaced by its definition, so the
  documents a serde-based generator writes are read as they are. A cycle,
  a remote reference or an undefined name stays as written and reads as a
  kind to skip. The copy is budgeted on what it copies, nodes plus bytes
  of text (32 times the document plus 256 Ki), so definitions that name
  each other twice stop growing; a reference naming a reference is
  followed in place, not as a level of nesting; and under `default`,
  `const`, `examples` and `enum` a `{"$ref": ..}` is data.
- **The reader gives generator shapes their meaning.** `type: [T, "null"]`
  and `anyOf: [X, null]` read as an optional `T`/`X`
  (`FieldRef::is_nullable()`); `enum` of any type, a lone `const` and a
  `oneOf` of `const`s read as `Kind::Enum`, `choices()` listing strings and
  numbers; `Kind::Null` is `type: "null"`; a lone `allOf` member -- the
  `{"description": .., "allOf": [{"$ref": ..}]}` written for a documented
  field -- reads as that member. A shape not read at all, and a schema
  that constrains nothing (`{}`, prose alone, an `allOf` of several, the
  schema `true`, which is now a field too), is `Kind::Unknown`;
  `Kind::Missing` is left for a value that is not a schema. Beside an
  `enum`, a `type` admitting null makes a field nullable only when null is
  a member.
- **`schema::PATTERNS_ENFORCED`**: whether this build checks a `pattern`
  (the `regex` feature). `vocab` gains `REF`, `DEFS`, `TYPE_NULL`,
  `ALL_OF`, `EXCLUSIVE_MINIMUM` and `EXCLUSIVE_MAXIMUM`, and the C header
  their `GUATIAO_KEY_*` macros.
- **`Text` and `Number` compare with `str`**, either way round
  (`text == "host"`, `"1.10" == number`); **`Text` is `Ord`** by its text,
  so it sorts and keys a `BTreeMap` as `String` does; **`Entry` is
  `Clone`, `PartialEq` and `Eq`**.
- **`Value::write_to(out, alloc)`**, the `unsafe` step an export of a
  library or host takes to hand its answer to a C caller: a null `out` is
  `GUATIAO_ERR_NULL`, a null `alloc` writes the tree as built, a named one
  gets a deep copy unless the root already records it, and a refused copy
  leaves `out` ABSENT with the status. The registry's and
  `guatiao-serde`'s exports write their answers through it.
- **`guatiao::Payload` and `guatiao::AllocError`** at the crate root,
  beside the `Value::from_raw_parts` that takes the one and the `try_`
  forms that answer the other.
- **`f32` is `ToValue`, `FromValue` and `TryFrom<&Value>`**, so the three
  derives take an `f32` field as they take an `f64` one. It writes its own
  shortest text (`1.1`, not the widened `1.100000023841858`), as
  `Number::try_from(f32)` now does too, and a number with no finite `f32`
  (`1e39`) is `MapError::BadValue`.
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
- **`#[map(rename_all = "...")]`** on a struct or an enum, as serde
  spells and applies it: `"lowercase"`, `"snake_case"`, `"kebab-case"`,
  `"SCREAMING_SNAKE_CASE"`. On a struct it names every field, on an enum
  every variant, before any `rename`; the schema's `enum`, the reader and
  `#[derive(Form)]`'s keys follow. A closed vocabulary in lower case is
  then a unit enum, validated by its schema, rather than a string with a
  `pattern`.
- **A provider can carry its form.** `#[provider(.., form = Ty)]`, for a
  `Ty` deriving `guatiao_intake::Form`, builds `Ty`'s screen into the
  descriptor's `meta` under `form` (`guatiao_intake::PROVIDER_FORM_KEY`),
  so a host shows a provider's configuration with its sections, labels
  and hints rather than bare section ids. `ProviderPartsBuilder::meta`
  sets a hand-built descriptor's `meta`. **`#[derive(Form)]` takes
  `#[form(crate = <path>)]`**, a re-export of `guatiao_intake`, and now
  reaches `guatiao` through `guatiao_intake` alone.
- **`include/kind_rename.toml`**, shipped beside `guatiao.h`: the C names
  of every guatiao type a kind table names, as a cbindgen configuration.
  A kind crate rendering its own header reads it from
  `DEP_GUATIAO_INCLUDE` and extends its `export.rename` with it, instead
  of copying the list; a test keeps it in step with `guatiao.h` and with
  what the kind attribute writes.
- **Borrowed object arguments.** A kind method may take `&mut dyn K`, or
  `&dyn K`, for an object kind `K`: the caller's value is lent for that
  call. It crosses as a `const guatiao_object *` on the caller's stack,
  whose `ctx` is the address of the caller's own pointer and whose table
  is the kind's lent table, so nothing is allocated and no `destroy` is
  owed. The callee's method gets a `&mut dyn K` over
  `guatiao::library::BorrowedMut` (or `Borrowed`), which has no `Drop`; a
  callee written in C calls its slots during the call, never keeps the
  pointer and never calls its `destroy`. `&dyn K` needs a `&self` method
  on `K`, and lent that way a `&mut self` slot answers
  `GUATIAO_ERR_UNSUPPORTED`; lending a provider kind is a compile error
  with a sentence. A method lending one returns `Result<_,
  ProviderError>`, as one passing an `Object` does. The lent tables are
  named by a hidden `unsafe` trait the attribute implements for every kind
  it declares, so a hand-written `impl Kind` compiles unchanged and cannot
  hand a callee a table of its own. A C lender fills `destroy` (every
  required slot is checked non-null) with a function that frees nothing;
  `guatiao.h` states the rest of the contract beside `guatiao_object`.
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
- **The Python and Dart bindings wrap the flat projection and the parse
  positions.** `guatiao.intake` / `package:guatiao/intake.dart` gain
  `resolve`, `flat_keys`, `flatten` and `unflatten` (Dart `resolve`,
  `flatKeys`, `flatten`, `unflatten`) over `guatiao_intake_resolve`,
  `_flat_keys`, `_flatten` and `_unflatten`, and `serde.loads` reads through
  the `_parse_ex` entry points, raising `ParseError` with `format`, `line`
  and `column`: in Python a `BadValue` and a `ValueError`, in Dart a
  `BadValue` and a `FormatException`, so an existing `except BadValue` /
  `on BadValue` still catches it. A kind's borrowed object arguments are
  not wrapped by either binding.

### Fixed

- **The Python binding frees what it builds when a conversion fails.** A
  value that failed half way (an element that cannot convert, a refused
  map or list move), and a list assignment or insert that raised after
  cloning its siblings, each left blocks allocated; the builder now frees
  its target on any failure and the cloned elements on every path.
- **The Dart package exports the struct types its public API names**
  (`guatiao_value`, `guatiao_alloc`) from `package:guatiao/guatiao.dart`,
  so a consumer can write `Pointer<guatiao_value>` without importing a
  `src/` file.
- **A `Simple` merge of two lists nested past `MAX_DEPTH` is
  `MergeError::Build(TooDeep)`** (`GUATIAO_ERR_BAD_VALUE` from
  `guatiao_merge`), where it overflowed the stack; and a stored absent map
  entry or list element keeps the earlier value under `Simple`, as it did
  under `Substitute` and `Deep`, instead of overwriting it with the absent
  marker.
- **`validate_map` checks a root that is not a declared object as the
  value it describes**, so a provider configured by a tagged enum accepts
  that enum's own value instead of refusing its discriminant as an
  unknown field.
- Merge provenance named the wrong layer when a later layer held an absent
  value (it claimed a leaf it did not set) and after a `Simple`-mode
  replace (the replaced subtree's leaves stayed attributed to the layer
  that no longer held them; the root could not be forgotten).
- **Every C function with an out-parameter writes it first.** The `guatiao`
  value constructors (`guatiao_value_null` to `_bytes`) and the registry's
  answers (`guatiao_registry_load_file`, `_register_entry`, `_scan_dir`,
  `_scan_dir_rules`, `_scan_path`, `_libraries`, `_providers`, `_provider`,
  `_available`, `_why_not`, `_best`), `guatiao-serde`'s functions, and
  `guatiao_intake_layout` and `_check` left `out` as the caller had it on
  failure, so a caller freeing it afterwards freed garbage; they now write it
  ABSENT on entry, as each header's preamble says of every such function, and
  `out` must not hold a tree the caller still owns.
  `guatiao_registry_provider_create` writes `out` null and `err` an empty
  error first. The `guatiao_intake_flatten`, `_unflatten` and `_flat_keys`
  docs describe lists, open maps and "nothing stored", and not only variants.
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
  unmapped only by `unload` or `collect`;
  `guatiao_registry_free` and a Python `Registry.close()` go through the
  same drop. A file passed over by its library key, or refused as a
  duplicate, after its entry point ran keeps its mapping too.
- **`unload` unmaps a library that a hard link to it was passed over
  for.** On Linux the loader answers a hard link to a loaded file with
  the image it already mapped and counts a second reference; the registry
  kept that handle, so `unload` reported success while the image stayed
  mapped. A skipped file whose handle names an image the registry already
  holds is now closed, which only gives that reference back.
- **A scan reads hostile files as data, and answers what it read.** A
  candidate is read on demand rather than whole: headers, export and dynamic
  symbol tables, and the 4 KiB declaration window. A file that only imports
  the entry symbol is not a library (an undefined dynamic symbol was taken for
  an export); an export table that cannot be read is `Skipped::NotExaminable`
  rather than `NoEntrySymbol`; and a universal Mach-O is probed through the
  slice for the running architecture instead of being refused, so a universal
  plugin loads on macOS.
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
- **An object kind with methods `x` and `mut_x` compiles.** Its lent
  tables' shims for the two had one name; they are `__guatiao_lentmut__<m>`
  and `__guatiao_lent__<m>` now, which no pair of method names can meet.
- **Python and Dart bindings: a borrowed view no longer outlives what it
  borrows from.** What `intake.resolve` and `intake.at` return holds the
  schema or value it views and refuses access once that is closed
  (`ValueError` in Python, `StateError` in Dart), where it read freed
  memory. Every write through a borrowed view is refused (`TypeError`,
  `StateError`), including deletes that allocate nothing.
- **Python and Dart bindings: `intake.unflatten` raises `TypeError`
  (Python) or `ArgumentError` (Dart) for any store that is not text to
  text**, whether it is a mapping or a value; `None`/`null` means only
  "nothing stored". The Python intake functions accept a `Ref` wherever
  they took a `Value`.
- **Python and Dart bindings: a parse refusal that is not a syntax error
  keeps the library's message** in the exception. Python `GuatiaoError` and
  `ParseError` pickle.

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

[Unreleased]: https://github.com/Boriken-Dev/guatiao/compare/v0.0.0-alpha.1...HEAD
[0.0.0-alpha.1]: https://github.com/Boriken-Dev/guatiao/compare/v0.0.0-alpha.0...v0.0.0-alpha.1
[0.0.0-alpha.0]: https://github.com/Boriken-Dev/guatiao/releases/tag/v0.0.0-alpha.0
