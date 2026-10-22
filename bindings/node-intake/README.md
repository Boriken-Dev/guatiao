# guatiao-intake for Node.js

[![npm](https://img.shields.io/npm/v/guatiao-intake.svg)](https://www.npmjs.com/package/guatiao-intake)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is laid
out, which fields are shown, and the flat `key -> text` store a front end
keeps. TypeScript over the `guatiao_intake` library, built on the
[`guatiao`](https://www.npmjs.com/package/guatiao) package for the value
model, its errors and its one koffi.

> **Status: alpha.** It ships no native code; you point it at the
> library.

## Features

- **Forms over a schema** — check a form document against the schema it
  shows, lay its fields out in sections, decide which are visible given
  what was entered, or take the form a schema implies when nobody wrote
  one.
- **A path names a place in a value** — `agent[1].name`, `env[PATH]`,
  read as a borrowed view of the tree you passed.
- **The flat store** — a value as `key -> text` and back, the way a front
  end keeps what a person typed, with tagged fields, lists and open maps.
- **A provider's form** — read from its `meta` under `providerFormKey`
  and checked against its configuration schema.
- **Any guatiao value** — every argument is a `Ref` of `guatiao` or plain
  data, and every answer an ordinary `Value`.

## Installation

```bash
npm install guatiao-intake
```

The package drives the `guatiao_intake` shared library. Say where it is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib         # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll   # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
platform's search. `useLibrary(path)` names it from code instead. Nothing
is loaded at import time; the first call throws `LibraryNotFound`, naming
every place it looked.

Node 22 or later, on a 64-bit platform koffi ships a build for.

## Quick start

```ts
import * as intake from 'guatiao-intake';

intake.check(schema, formDoc);                      // null when the form fits the schema
intake.layout(schema, formDoc);                     // [{ section, fields: [...] }, ...]
intake.isVisible(schema, formDoc, 'tls.verify', values);
intake.at(value, 'agent[1].name');                  // a borrowed Ref, or undefined

// A front end that keeps text under flat keys (`agent[0].name`, `env[PATH]`):
intake.resolve(schema, 'auth.password');            // the schema field, or undefined
intake.flatKeys(schema, 'auth');                    // ['auth', 'auth.username', ...]
const store = intake.flatten(schema, 'auth', auth); // { auth: 'userpass', ... }
using back = intake.unflatten(schema, 'auth', store)!;

using meta = registry.providerMeta('acme_dialer')!; // a provider's form
intake.check(registry.providerConfig('acme_dialer')!, meta.asMap().get(intake.providerFormKey)!);
```

## Known limits

- What `at` and `resolve` answer borrows from the tree it was found in:
  read-only, and refused once that tree is closed. `.clone()` copies it
  out.
- 64-bit platforms only, and not in a browser.

## API overview

| Name | Purpose |
| --- | --- |
| `check`, `layout`, `isVisible`, `forSchema`, `formFor` | a form over a schema |
| `at` | a path's place in a value |
| `resolve`, `flatKeys`, `flatten`, `unflatten` | the flat `key -> text` store |
| `providerFormKey` | where a provider's `meta` carries its form |
| `useLibrary(path)`, `library()` | naming and loading `guatiao_intake` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/node-intake/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

## Development

```bash
cd bindings
npm ci                # the workspace: guatiao, guatiao-serde and this package
cd node-intake
npm run typecheck
npm test              # vitest, against the core's sources beside it
npm run build         # dist/, after `npm run build` in ../node
```

`test/setup.ts` points `GUATIAO_LIBRARY` at the repository's build output
when the variable is unset; the repository's own `AGENTS.md` says how to
produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
