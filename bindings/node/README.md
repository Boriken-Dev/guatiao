# guatiao for Node.js

[![npm version](https://img.shields.io/npm/v/guatiao.svg)](https://www.npmjs.com/package/guatiao)
[![Node versions](https://img.shields.io/node/v/guatiao.svg)](https://www.npmjs.com/package/guatiao)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/node/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Node.js and TypeScript bindings for the
[guatiao](https://github.com/Boriken-Dev/guatiao/) ABI: **one value model
for passing data between languages**, a JSON Schema that describes a
value, and a registry that loads plugin libraries. Typed ESM over koffi,
with nothing to compile. It works with any shared library that exports the
ABI, whether that is `guatiao` itself or an application that carries it.

> **Status: alpha.** The package carries no native library of its own:
> you point it at one.

## Features

- **Value trees as JavaScript objects** — build, read and change a tree in
  place, or convert it to and from plain objects, arrays and `Map`s.
- **Numbers keep their exact text** — `1.10` stays `1.10` and an integer
  of any size survives as a `bigint`.
- **Dates, times, spans and exact lengths** — `PlainDate`, `PlainTime`,
  `PlainDateTime` and `Span` to the nanosecond, instants as `Date`, read
  and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.
- **Importing never fails** — the native library is looked for on first
  use, and a missing one names every place that was searched.

## Installation

```bash
npm install guatiao
```

The package drives a shared library that exports the guatiao ABI. Say
where it is:

```bash
export GUATIAO_LIBRARY=/opt/myapp/lib          # a directory holding guatiao.dll / libguatiao.so / libguatiao.dylib
export GUATIAO_LIBRARY=/opt/myapp/myapp.dll    # or the file itself, whatever it is called
```

or call `useLibrary(path)` before the first call. Without either, the
platform's library search is tried by file name. A library that exports
only part of the ABI is fine: calling a function it lacks throws
`MissingSymbol` naming it.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, time, redaction |
| `guatiao_serde` | `guatiao/serde`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | `guatiao/intake` |

Node 22 or later, on a 64-bit platform koffi ships a build for (Linux,
macOS, Windows, FreeBSD, OpenBSD, Android; x64 and arm64).

## Quick start

### Values

```ts
import { Value } from 'guatiao';

using v = new Value({ host: '10.0.0.1', port: 5900, tags: ['a', 'b'] });
const m = v.asMap();
m.set('port', 5901);
m.get('tags')!.asList().push('c');

m.get('port')!.toJS();      // 5901
m.keys();                   // ['host', 'port', 'tags'], insertion order
m.get('port')!.intOr(22n);  // 5901n, or 22n if it were not an integer
v.toJS();                   // back to a plain object
```

A `Value` owns its tree and frees it on `close()`, or at the end of the
scope with `using`. `m.get('tags')` is a `Ref`: a path into the tree that
is walked again on every access, so it stays valid while the tree grows
around it.

**Numbers keep their exact text.** Choose what comes back when reading:

```ts
using n = Value.number('1.10');
n.toJS();                       // 1.1
n.toJS({ numbers: 'text' });    // '1.10'
using big = new Value(2n ** 64n);
big.toJS();                     // 18446744073709551616n
```

### JSON, TOML and YAML

```ts
import * as serde from 'guatiao/serde';

using doc = serde.loads('{"name": "ana", "n": 1.10}');
serde.dumps(doc, { format: 'yaml' });
```

### Dates and spans

```ts
import { PlainDate, Span } from 'guatiao/time';

PlainDate.parse('2026-01-31').add(Span.parse('1mo')).toString();   // '2026-02-28'
```

## Known limits

- An instant reads into a `Date`, which counts milliseconds: what lies
  below one is dropped, toward the past. `PlainTime`, `PlainDateTime` and
  `Span` keep every nanosecond.
- A map read with `toJS()` becomes a plain object, whose integer-like keys
  (`"2"`) JavaScript lists before the others; `toJS({ maps: 'map' })`
  keeps the map's order.
- `ListRef.set` and `insert` rebuild the list, because the C ABI only
  appends and removes.
- 64-bit platforms only, and not in a browser.

## API overview

| Module | Purpose |
| --- | --- |
| `guatiao` | `Value`, `Ref`, `MapRef`, `ListRef`, `redact`, `useLibrary`, the errors |
| `guatiao/serde` | `loads` / `dumps` for JSON, TOML, YAML |
| `guatiao/intake` | `check`, `layout`, `isVisible`, `at`, `forSchema`, `formFor`, `resolve`, `flatKeys`, `flatten`, `unflatten` |
| `guatiao/time` | `PlainDate`, `PlainTime`, `PlainDateTime`, `Span`, instants, exact lengths, `normalize` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/node/AGENTS.md)
is the full API reference and ships in the package. The C ABI is described
by [`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/node
npm ci
npm run typecheck     # tsc --strict over the sources, the tests and the examples
npm test              # vitest
npm run build         # dist/
```

The tests need the three libraries and the repository's example plugins
in one directory; `test/setup.ts` points `GUATIAO_LIBRARY` at the
repository's build output when the variable is unset, and the repository's
own `AGENTS.md` says how to produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
