# Node.js: host and provider

Node plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a library,
classes in TypeScript or JavaScript, that any host in the same process
calls through the same tables. Both use the `guatiao` package: typed ESM
over [koffi](https://koffi.dev/), for Node 22 and later, shipping no native
code of its own.

```bash
npm install guatiao
```

The package loads the guatiao library on its first native call: the one
`useLibrary(path)` named, else `GUATIAO_LIBRARY` (a file or a directory),
else the platform's library search.

## A kind, never declared twice

A kind is the table its C header declares, and its crate commits the
same kinds as data beside the header: a [description](encodings.md#a-kind-described)
(`greeter_kind.kinds.json`) naming each slot in table order, what it
takes and answers, and whether it is required. Node loads that; nothing
here says again what a slot takes.

`guatiao-kind`, a command the package installs, writes a module from the
description, which you commit:

```bash
npx guatiao-kind greeter_kind.kinds.json --out greeter_kind   # greeter_kind.js and greeter_kind.d.ts
```

In JavaScript each kind is a value, `Greeter`. In TypeScript the same
name is also an interface: the kind's required slots as methods, its
appended ones optional, typed from the description, so a provider class
`implements` the kind it serves:

```ts
// greeter_kind.d.ts, as guatiao-kind writes it (abridged)
export interface Greeter {
  greet(name: string): k.MapAnswer;
  shout?(name: string): string;
  greetInto?(name: string, listener: ListenerRemote): void;
}
export interface GreeterRemote extends k.RemoteBase<typeof Greeter> {
  greet(name: string): g.Value;
  shout(name: string): string;
  greetInto(name: string, listener: Listener | k.ObjectHandle<typeof Listener>): void;
}
export declare const Greeter: k.DescribedKind<Greeter, GreeterRemote, false>;
```

`kindsOf(document)` loads the same description at run time, from the
parsed object or its JSON text, and answers each kind by name:

```js
import { readFileSync } from 'node:fs';
import { kindsOf } from 'guatiao/kinds';

const { greeter: Greeter } = kindsOf(readFileSync('greeter_kind.kinds.json', 'utf8'));
```

A library built with `#[derive(Provider)]` carries the same document, so
a host can bind its kinds with no file at all:

```js
const { greeter: Greeter } = kindsOf(reg.kindDescriptions('derived_greeter_hello'));
const greeter = reg.bind('derived_greeter_hello', Greeter);
```

### A kind defined in JavaScript

A kind with no header behind it, defined here for other code to serve or
call, is a class extending `Kind`; its static fields say its name, floor
hash and slots, which plain JavaScript runs as they are:

```js
import { Kind, slot, t } from 'guatiao/kinds';

export class Page extends Kind {
  static kind = 'page';
  static floorHash = 0x70616765;
  static object = true;
  static slots = { write: slot(t.str).required() };
}

export class Echo extends Kind {
  static kind = 'echo';
  static floorHash = 0x6563686f;
  static slots = {
    echo: slot(t.str).returns(t.text).required().fallible(),
    copyOnto: slot(t.str, t.lent(Page)).fallible(),
  };
}
```

A class's description is data too: `describeKinds([Echo])` answers the
document `kindsOf` loads and the other bindings' generators write code
from, with the object kinds the slots reach. `slot(...).named('text')`
names the arguments (one name each), `.doc('...')` and `static doc` say
what a slot and the kind do; both are in the description and nowhere
else. Plain JavaScript writes the file with no build step:

```js
import { writeFileSync } from 'node:fs';
import { describeKinds } from 'guatiao/kinds';
import { Echo } from './echo_kind.js';

writeFileSync('echo_kind.kinds.json', `${JSON.stringify(describeKinds([Echo]), null, 2)}\n`);
```

and Dart's `dart run guatiao:kind echo_kind.kinds.json` writes the same
kinds as Dart classes.

### A kind defined in TypeScript

A kind that exists only here can be written as interfaces instead, the way
a Rust trait declares one, and `guatiao-kind` reads them into the same
description. The types are `guatiao/kinds`': a scalar says its width
(`u32`, `i64`, `f64`; a bare `number` is refused), a tree says how it
crosses (`MapRef` is read for the call, `Taken<MapRef>` is handed over,
`Lent<MapRef>` is changed where it stands), an object kind extends
`ObjectKind`.

```ts
// tally_kind.ts
import type { MapRef } from 'guatiao';
import type { Lent, MapAnswer, ObjectKind, Taken, i64, u32 } from 'guatiao/kinds';

/** What a tally tells whoever is watching it. Implemented by the host. */
export interface Watcher extends ObjectKind {
  /** `name` now stands at `total`. */
  changed(name: string, total: i64): void;
}

/** Counts by name. */
export interface Tally {
  /**
   * Adds `by` to the count under `name`: the new count, or why not.
   *
   * @throws
   */
  add(name: string, by: u32): i64;
  /** The count under `name`: 0 when there is none. */
  total(name: string): i64;
  /** Appended: every count, as `{name: count}`. */
  all?(): MapAnswer;
  /**
   * Appended: tells `watcher` each count, for this call alone.
   *
   * @throws
   */
  report?(watcher: Lent<Watcher>): void;
  /**
   * Appended: adds the counts of `counts`, which the tally owns from the
   * call on; answers how many names it held.
   *
   * @throws
   */
  load?(counts: Taken<MapRef>): u32;
}
```

```bash
npx guatiao-kind tally_kind.ts --out node/tally_kind --document include/tally_kind.kinds.json
```

writes the description (`tally_kind.kinds.json`, the file Dart's `dart
run guatiao:kind` and every other generator reads) and the module, as for
any kind: `Tally` is then a value and an interface, and a provider
`implements` it. `describeInterfaces(files)` from `guatiao/describe` does
the first step from code and answers the document.

- **An exported interface is a kind**, named in snake_case (`@kind name`
  says otherwise); one that `extends ObjectKind` is an object kind. An
  object kind a slot reaches is described too, whichever file it is in.
- **A method is a slot**, in the interface's order; a method with `?` is
  an appended slot, and the required ones come first, as in a trait. A
  slot's name is the method's in snake_case, so a method that would not
  come back the same (`parseHTTP`) is refused. `@throws` marks a slot
  that can refuse.
- **Doc comments** are the descriptions' `doc`.
- **The floor hash** is the FNV-1a of the shape and the required slots,
  spelled as Rust spells them, so an interface repeating a trait's
  required slots hashes as the trait does; `@floorHash 123` says it where
  a kind must keep a hash it already has.
- **Reading is done by the compiler**, the `typescript` package, version
  7.0 or newer, which the project already has if it checks its types: the
  files are read through the nearest `tsconfig.json`, or `--project`. A
  type that is not one of these is refused with its file, line and column.

The compiler API in TypeScript 7.0 is marked unstable, so reading
interfaces is the one part of this package that follows TypeScript's
releases; a kind defined as a class needs no compiler.

### Underneath: `kind()`

Both are built on one primitive, which TypeScript can type by inference:
its methods in table order, their shapes from `t`, the header's
`FLOOR_HASH`.

```ts
import { type Implementation, kind, t } from 'guatiao/kinds';

export const Listener = kind('listener', {
  floorHash: 1867921573,
  object: true,
  methods: { heard: { args: [t.str], required: true } },
});

export const Greeter = kind('greeter', {
  floorHash: 2695856491,
  methods: {
    greet: { args: [t.str], answer: t.map, required: true, fallible: true },
    shout: { args: [t.str], answer: t.text },
    greetInto: { args: [t.str, t.lent(Listener)], fallible: true },
    check: { args: [t.lent(t.map)], answer: t.bool, fallible: true },
  },
});
export type Greeter = Implementation<typeof Greeter>;

export const Counter = kind('counter', {
  floorHash: 1227528937,
  methods: { count: { answer: t.i64, required: true } },
});
export type Counter = Implementation<typeof Counter>;
```

- `required` marks the slots every table fills; a table shorter than them
  is refused. `fallible` marks a slot that can refuse, with a
  `guatiao_provider_error *err` last.
- An integer says its width (`t.i8` to `t.i64`, `t.u8` to `t.u64`,
  `t.usize`, `t.isize`), as C does; the 64-bit ones and the word-wide
  `usize` and `isize` cross as `bigint`.
- `t.lent(...)` is a tree or object lent for the call and changed where it
  stands; `t.taken(...)` one handed over; an `answer` of `t.map`, `t.list`
  or `t.value` a tree answered, and an object kind an object answered.
- The `methods` object's order is the table's: a slot is found by
  position, so the TypeScript name is free (`greetInto` for `greet_into`).

The binding's
[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/node/guatiao/AGENTS.md)
lists every shape, both ways.

## As a host

```ts
import { Registry } from 'guatiao';
import { Greeter } from './greeter_kind.js';

using reg = new Registry('my-host', '1.0');
reg.scanDir('/opt/plugins', { rules: 'kind=greeter' });

for (const provider of reg.providers('greeter')) {
  const greeter = reg.bind(String(provider.key), Greeter);
  using answer = greeter.greet('ana');
  console.log(answer.toJS());                 // { greeting: 'hello, ana' }
  if (greeter.has('greetInto')) {
    greeter.greetInto('bo', { heard: (what: string) => console.log(what) });
  }
}

using instance = reg.create('acme_shouter', { prefix: 'hey' });
instance.bind(Greeter).greet('cy').close();
```

`bind` checks the table against the kind (its size and its floor hash)
before anything in it runs. A slot an older table lacks throws
`Unsupported`, and `has(name)` asks first. A refusal throws the error its
status names (`BadValue`, `NotFound`, ...) with the provider's own words.
A tree answered is a `Value` the caller closes; an object answered is an
`ObjectHandle` whose `close()` destroys it.

Sources work as in every binding: `reg.addSourceFile(path, config)` adds
a source such as the guatiao-source-py bridge, and `reg.refresh()` loads what it
serves. A Node host calls greeters written in Python that way.

## As a provider

A provider is a class extending `Provider`, implementing its kinds' types
and listing them in `static kinds`. The same provider file also runs
without Node, in any host, through [the JavaScript source](js.md):

```ts
// greeters.ts, in a package named node_greeter
import { BadValue } from 'guatiao';
import { Provider, s } from 'guatiao/provide';
import { Counter, Greeter } from './greeter_kind.js';

export class Hello extends Provider implements Greeter, Counter {
  static override readonly kinds = [Greeter, Counter];
  static override readonly displayName = 'Hello, in Node';
  #n = 0n;

  greet(name: string) {
    if (!name) throw new BadValue('nobody to greet');
    return { greeting: `hello, ${name}` };
  }

  count() {
    return ++this.#n;
  }
}

export const ShouterConfig = s.object({ prefix: s.string() });

export class Shouter extends Provider implements Greeter {
  static override readonly kinds = [Greeter];
  static override readonly config = ShouterConfig;
  readonly #prefix: string;

  constructor(config: s.Infer<typeof ShouterConfig>) {
    super();
    this.#prefix = config.prefix;
  }

  greet(name: string) {
    return { greeting: `${this.#prefix} ${name.toUpperCase()}` };
  }
}
```

- **`tsc --strict` checks it**: `implements Greeter` refuses a class that
  misspells `greet`, takes a number for the name or answers the wrong
  type, and an appended method it does implement with the wrong types.
  `static kinds` is what serves it, since `implements` is erased. A
  misspelled appended method is not caught: it is one more method.
- **Plain JavaScript runs the same class as it is**, with no build step:
  `static kinds = [Greeter, Counter];` and no types. A class missing a
  required method is refused when its library is built, with a
  `TypeError` naming the method and the kind. With `// @ts-check` at the
  top of the file, `/** @implements {Greeter} */` above the class has an
  editor check its methods against the generated declarations.
- **A configuration is a schema value.** `s.object({ prefix: s.string()
  })` writes the JSON Schema the host checks a configuration against, and
  its type is inferred; the constructor receives the built object. With no
  `config` the provider is its one instance.
- **An exception is a refusal**: a `GuatiaoError` crosses with its status
  and words, anything else as `GUATIAO_ERR_INTERNAL`.

**The module is the library.** `libraryOf` imports an ES module and reads
its package: the `package.json` `name` is the library's id, its `version`
the version, the `Provider` classes the module exports its providers, its
`description`, `license`, `author` and licence files its meta.

```ts
import { libraryOf } from 'guatiao/provide';

const greeters = await libraryOf(new URL('./greeters.js', import.meta.url));
reg.registerLibrary(greeters);       // node_greeter_hello, node_greeter_shouter
```

`library(id, version, { providers, meta })` builds one without a module.
A host in the same process, written in any language, then reaches these
providers as it reaches any other: through the registry's tables, or the
host's services from inside a library. The repository's
`examples/greeter_caller` is such a caller written in Rust, and the
binding's tests have it call every slot of a TypeScript greeter.

Serving Node modules to a host that is not a Node process (a bridge, as
guatiao-source-py is for Python) is not offered.

## Threads

Everything runs on Node's main thread. A library may call a provider or
an object written here from a thread of its own: koffi queues the call,
it runs on Node's main thread when the event loop next turns, and the
calling thread waits for the answer. So a native call made synchronously
from Node's thread that waits for such a thread never returns.

## What Node does not do that Python does

- **Serve another process.** guatiao-source-py serves Python modules to any host;
  nothing serves Node modules to a host that is not Node.
- **Answer from more than one thread at a time.** Python's calls are
  serialised by the GIL but run on the calling thread; Node's run on its
  main thread, so a library's threads wait for its event loop.
- **Register a library synchronously from a module object.** A Python host
  passes a module; an ES module namespace carries no name, so
  `libraryOf` takes the module's location and imports it.
- **Make kinds anywhere.** `kind()`, `kindsOf()` and `library()`
  register koffi callbacks, as does a kind class when first used, which
  koffi cannot do inside a call a library is making: describe kinds and
  build libraries at module level.

## A page, or Node, as a client of a server

Calling the providers a `guatiao-remote-server` serves over a WebSocket
needs none of this package: `guatiao-remote` is guatiao's remote protocol
in plain TypeScript, for a browser page or Node, with no native library
(see [remote](remote.md), "A page").

## The examples

- `examples/node_host/` scans for greeters in any language, registers
  the TypeScript library, optionally adds the guatiao-source-py bridge, and calls
  every slot of the kind; its README has the commands.
- `examples/node_provider/node_greeter/` is the library above.
- `examples/node_js/` is a library and a host in plain JavaScript, with a
  kind of its own defined as a class.
- `examples/greeter_kind/node/` is what `guatiao-kind` wrote from
  `examples/greeter_kind/include/greeter_kind.kinds.json`.
- `examples/tally_kind/` is a kind written as TypeScript interfaces,
  `tally_kind.ts`, with the description and the Node and Dart modules
  written from it.
