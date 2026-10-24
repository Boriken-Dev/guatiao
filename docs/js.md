# JavaScript without Node: a source of libraries

`guatiao-js` is a [source](language-bridges.md) whose libraries are
JavaScript modules, run by [QuickJS](https://github.com/quickjs-ng/quickjs)
built into one file. A host in any language adds the file once and calls
providers written in JavaScript as it calls the ones written in Rust, C
or Python. The host needs no Node, no browser and no JavaScript engine of
its own, which suits a program that wants JavaScript scripts and cannot
ship Node: a mobile app, an embedded device, a small daemon.

A library is written the way the [Node binding](node.md#as-a-provider)
writes one: a class extending `Provider` that implements the kinds it
lists, loaded from their description. **The same file runs under Node and
here.**

## Building

The file is built from this repository, with a C compiler (QuickJS is C):

```bash
cargo build --release --manifest-path crates/guatiao-js/Cargo.toml
```

It is `libguatiao_js.so`, `libguatiao_js.dylib` or `guatiao_js.dll`, a
workspace of its own and not published.

## A library

A package: a `package.json` that names it and gives its version, the
module, and a manifest the source reads before it runs anything.

```js
// greeters.js
import { BadValue } from 'guatiao';
import { Provider } from 'guatiao/provide';
import { Counter, Greeter } from './greeter_kind.js';   // written by guatiao-kind

export class Hello extends Provider {
  static kinds = [Greeter, Counter];
  static displayName = 'Hello, in JavaScript';
  #asked = 0n;

  greet(name) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello, ${name}` };
  }
  count() {
    return ++this.#asked;
  }
}
```

```toml
# greeters.guatiao-js.toml
name = "greeters"                 # the library's id: the package's name
kinds = ["greeter", "counter"]    # every kind its providers serve
paths = ["."]                     # the files it may import, relative to this file
# version and main are the package.json beside it, unless said here
```

`guatiao-kind` writes `greeter_kind.js` from the kinds'
[description](encodings.md#a-kind-described), as it does for Node; the
source is given the same description by the host, which is the one that
counts: a module that describes a kind differently is refused. A
provider is every exported class extending `Provider`; its id is the
library's followed by its name in snake case (`greeters_hello`).

What a library imports is `guatiao` (the errors, `ExactNumber`),
`guatiao/kinds`, `guatiao/provide` and its own files by relative path. It
has `console`, `TextEncoder` and `TextDecoder`, and no files, network or
processes: QuickJS is given none.

[`examples/js_provider`](https://github.com/Boriken-Dev/guatiao/tree/main/examples/js_provider)
holds a greeter serving every kind of the example kinds that runs under
Node and here, and one taking trees.

## Serving it to a host

The host adds the file and says where the manifests and the kinds are:

```rust,ignore
let mut config = Map::new();
config.set("dirs", List::from_iter(["/opt/myapp/js"]));
config.set("kinds", List::from_iter(["/opt/myapp/greeter_kind.kinds.json"]));
config.set("limits", limits);        // optional
registry.add_source_file(Path::new("/opt/myapp/libguatiao_js.so"), &config)?;
let changed = registry.refresh();
```

```python
reg.add_source_file("libguatiao_js.so", {"dirs": ["js"], "kinds": ["greeter_kind.kinds.json"]})
reg.refresh()
reg.bind("greeters_hello", Greeter).greet("ana")
```

`refresh()` lists the manifests without running anything and imports a
new or changed library in a context of its own; an old build closes once
nothing holds it, so an edit to the module, to a file it imports or to
its `package.json` is served at the next refresh while instances of the
old build still answer. Two libraries share nothing but the runtime's
thread and memory limit. The manifests, stamps and refresh are the
[bridge's](language-bridges.md).

## Limits

The configuration's `limits` are kept per call, opening a library
included, and a stop is an error the host reads while the runtime goes on:

| limit | kept by |
| --- | --- |
| `operations`, `time_ms` | QuickJS's interrupt handler, polled once per 10 000 calls and loop iterations; `try`, `catch` and `finally` cannot hold it back |
| `depth` | the native stack QuickJS may use, about 740 bytes a level |
| `memory` | QuickJS's allocator, for the whole runtime |

## From Node, what differs

The shapes are the Node binding's, with these exceptions:

- **A tree argument is plain data**, an object or an array (a lent one
  changed in place and read back); Node hands a `Value` or a `MapRef`. A
  provider that takes trees is written for one engine.
- **A decimal that a double would change** is an `ExactNumber`
  (`toString()` gives its text), where Node reads it as a `number`.
- **A provider is one instance**: `static config` and `s` (providers built
  from a configuration) are refused, and a kind cannot be defined in
  JavaScript: the host's descriptions are the kinds.
- **A library's meta** (the package's `description`, licence and authors)
  is not carried.
- **A script's own object is never passed to a host as an object**, only
  answered; a handle the host gave it can be passed on.

## What JavaScript cannot do here

- **Wait.** There is no event loop and no timer. Promise reactions run
  after each call, under the call's limits (a call whose reactions run
  past them is stopped, and the rest are killed), and a slot answers
  before it returns: an `async` slot is refused.
- **Stop native work mid-call.** A limit is looked at once per poll: one
  long `repeat` or sort runs to its end, and the memory limit bounds what
  it can build.
- **Run calls in parallel.** One runtime, one thread, calls queued to it;
  a call a host makes back into the source from inside a call runs at
  once on that thread.
- **Run on a target QuickJS does not build for.** Linux x86-64 is what the
  tests here ran on.
