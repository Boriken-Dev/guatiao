# A plugin host in Dart

`bin/host.dart` loads greeter libraries written in any language and calls
each one through the classes `dart run guatiao:kind` wrote for the
`greeter` kind ([`../greeter_kind/dart`](../greeter_kind/dart/)): a map
back, a map lent and changed in place, a map handed over, an object
handed in and one handed back (a conversation), an object lent for one
call (a listener). It registers the greeters written in Dart
([`../dart_provider`](../dart_provider/)) the same way, and with
`--caller` has the Rust caller (`greeter_caller`) call the Dart greeter
through the kind's Rust proxy. It closes everything it opened.

## Run it

From the repository root, after building the workspace:

```bash
cargo build --workspace --all-features
export GUATIAO_LIBRARY=$PWD/target/debug
cd examples/dart_host && dart pub get
dart run bin/host.dart ../../target/debug --caller ../../target/debug/libgreeter_caller.so
```

On Windows the caller is `greeter_caller.dll`, on macOS
`libgreeter_caller.dylib`. To add the greeter written in C, build it into
a directory of its own and name that directory too (as
[`../node_host/README.md`](../node_host/README.md) shows); to add those
written in Python, name the guatiao-source-py bridge with `--python-source`,
where its manifests are with `--python-dir`, and what they import with
`--python-path`.

An AOT executable is the same program:

```bash
dart build cli -t bin/host.dart -o build/host
build/host/bundle/bin/host ../../target/debug --caller ../../target/debug/libgreeter_caller.so
```

## What it shows

- **One set of classes serves every language.** The Rust, C, Python and
  Dart greeters are called through `GreeterRemote`; a table whose floor
  hash is not the kind's is refused before anything in it runs.
- **A table may be older than the kind.** The C greeter fills four of six
  slots; `has('shout')` asks before calling, and calling a missing slot
  throws `Unsupported`.
- **Dart objects cross both ways**: the listener is a plain class
  implementing `Listener`; the conversation a greeter hands back is a
  `ConversationRemote` whose `close()` destroys it.
- **Rust calls Dart**: the Rust caller reaches the Dart greeter through
  the registry's host services and calls every slot of it.
