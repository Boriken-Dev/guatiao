# guatiao-serde for Node.js

[![npm](https://img.shields.io/npm/v/guatiao-serde.svg)](https://www.npmjs.com/package/guatiao-serde)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a `Value`, write one back out. TypeScript
over the `guatiao_serde` library, built on the
[`guatiao`](https://www.npmjs.com/package/guatiao) package for the value
model, its errors and its one koffi.

> **Status: alpha.** It ships no native code; you point it at the
> library.

## Features

- **Three formats, one call each** — `loads` and `dumps` for JSON, TOML
  and YAML, parsed and written by the library itself, so every language
  bound to guatiao reads a document the same way.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives the trip.
- **Bytes have a spelling** — a data URI, base64 or a list of numbers
  where the format has none, and data URIs read back as bytes when asked.
- **A document that does not parse says where** — `ParseError` carries
  the format, line and column, and never quotes the document.
- **Any guatiao value** — what `loads` returns is an ordinary `Value` of
  `guatiao`, and `dumps` takes any `Ref` or plain data.

## Installation

```bash
npm install guatiao-serde
```

The package drives the `guatiao_serde` shared library. Say where it is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib         # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll   # or the file itself, whatever it is called
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
import * as serde from 'guatiao-serde';

using doc = serde.loads('{"name": "ana", "n": 1.10}');   // format 'json' by default
doc.asMap().get('n')!.toJS({ numbers: 'text' });          // '1.10'
serde.dumps(doc, { format: 'yaml' });
serde.dumps({ port: 5900 }, { pretty: true });            // plain data too

try {
  serde.loads('{"port": }');
} catch (e) {
  if (e instanceof serde.ParseError) console.log(e.format, e.line, e.column);
}
```

A format the loaded library was built without throws `MissingSymbol`
naming its feature. A TOML document must be a map.

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
  Everything else about a number's text survives a round trip.
- 64-bit platforms only, and not in a browser.

## API overview

| Name | Purpose |
| --- | --- |
| `loads(text, { format, how, alloc })` | a document as a `Value` |
| `dumps(value, { format, pretty, how })` | a `Ref` or plain data as text |
| `ParseError` | a document that does not parse: `format`, `line`, `column`, `reason` |
| `BYTES_DATA_URI`, `BYTES_BASE64`, `BYTES_ARRAY`, `BYTES_REFUSE`, `READ_DATA_URIS`, `PRETTY` | the `how` flags |
| `useLibrary(path)`, `library()` | naming and loading `guatiao_serde` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/node/guatiao-serde/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
cd bindings/node
npm ci                # the workspace: guatiao, this package and guatiao-intake
cd guatiao-serde
npm run typecheck
npm test              # vitest, against the core's sources beside it
npm run build         # dist/, after `npm run build` in ../guatiao
```

`test/setup.ts` points `GUATIAO_LIBRARY` at the repository's build output
when the variable is unset; the repository's own `AGENTS.md` says how to
produce it.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
