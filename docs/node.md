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

## A kind, once

A kind is the table its C header declares. TypeScript describes it once,
as a typed value: its methods in table order, their shapes from `t`, the
header's `FLOOR_HASH` copied from its `#define`. The types a provider
implements and a caller holds are inferred from it.

```ts
// greeter_kind.ts
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
  `t.usize`, `t.isize`), as C does; the 64-bit ones cross as `bigint`.
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
a source such as the guatiao-py bridge, and `reg.refresh()` loads what it
serves. A Node host calls greeters written in Python that way.

## As a provider

A provider is a class extending `Provider`, implementing its kinds' types
and listing them in `static kinds`:

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
  type. `static kinds` is what serves it, since `implements` is erased.
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
guatiao-py is for Python) is not offered.

## Threads

Everything runs on Node's main thread. A library may call a provider or
an object written here from a thread of its own: koffi queues the call,
it runs on Node's main thread when the event loop next turns, and the
calling thread waits for the answer. So a native call made synchronously
from Node's thread that waits for such a thread never returns.

## What Node does not do that Python does

- **Serve another process.** guatiao-py serves Python modules to any host;
  nothing serves Node modules to a host that is not Node.
- **Answer from more than one thread at a time.** Python's calls are
  serialised by the GIL but run on the calling thread; Node's run on its
  main thread, so a library's threads wait for its event loop.
- **Register a library synchronously from a module object.** A Python host
  passes a module; an ES module namespace carries no name, so
  `libraryOf` takes the module's location and imports it.
- **Make kinds anywhere.** `kind()` and `library()` register koffi
  callbacks, which koffi cannot do inside a call a library is making:
  describe kinds and build libraries at module level.

## The examples

- `examples/node_host/` scans for greeters in any language, registers
  the TypeScript library, optionally adds the guatiao-py bridge, and calls
  every slot of the kind; its README has the commands.
- `examples/node_provider/node_greeter/` is the library above.
- `examples/greeter_kind/node/greeter_kind.ts` describes the greeter kinds.
