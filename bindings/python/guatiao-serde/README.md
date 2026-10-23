# guatiao-serde for Python

[![PyPI version](https://img.shields.io/pypi/v/guatiao-serde.svg)](https://pypi.org/project/guatiao-serde/)
[![Python versions](https://img.shields.io/pypi/pyversions/guatiao-serde.svg)](https://pypi.org/project/guatiao-serde/)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

JSON, TOML and YAML for [guatiao](https://github.com/Boriken-Dev/guatiao/)
values: read a document into a `guatiao.Value`, write one back out. Pure
Python over `ctypes`, calling the `guatiao_serde` library, and built on
the [`guatiao`](https://pypi.org/project/guatiao/) package for the value
model and its errors.

> **Status: alpha.** A pure Python package: it carries no native
> library, you point it at one.

## Features

- **Three formats, one call each** — `loads` and `dumps` for JSON, TOML
  and YAML, parsed and written by the library itself, so every language
  bound to guatiao reads a document the same way.
- **Numbers keep their exact text** — `1.10` stays `1.10`, and an integer
  of any size survives the trip.
- **Bytes have a spelling** — a data URI, base64 or a list of numbers
  where the format has no bytes, and data URIs read back as bytes when
  asked.
- **A document that does not parse says where** — `ParseError` carries
  the format, line and column, and never quotes the document.
- **Any guatiao value** — what `loads` returns is an ordinary
  `guatiao.Value`, and `dumps` takes any `Value` or `Ref`, whichever
  library built it.
- **Or plain Python, with no value in between** — `parse` answers dicts
  and lists, `render` writes them; the document crosses once.

## Installation

```bash
pip install guatiao-serde
```

The package drives the `guatiao_serde` shared library. Say where it is:

```bash
export GUATIAO_SERDE_LIBRARY=/opt/myapp/lib       # a directory holding guatiao_serde.dll / libguatiao_serde.so / .dylib
export GUATIAO_SERDE_LIBRARY=/opt/myapp/myapp.dll # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
system's library search. Nothing is loaded at import time; the first call
raises `guatiao.LibraryNotFound`, naming every place it looked.
`guatiao_serde.use_library(path)` names the library from code instead.

## Quick start

```python
import guatiao_serde as serde

value = serde.loads('{"port": 5900, "tags": ["a"]}')     # format="json" by default
value.as_map()["port"].to_python()                       # 5900
text = serde.dumps(value, format="yaml")
pretty = serde.dumps(value, pretty=True)
value.close()

config = serde.parse('{"port": 5900}')                   # {'port': 5900}
serde.render(config, format="toml")                      # 'port = 5900\n'

try:
    serde.loads('{"port": }')
except serde.ParseError as error:
    error.format, error.line, error.column               # ("json", 1, 10)
```

A format the loaded library was built without raises
`NotImplementedError` naming the feature. A TOML document must be a map.

## Known limits

- Reading JSON writes an exponent's sign: `1e400` parses as `1e+400`.
  Everything else about a number's text survives a round trip.
- 64-bit targets only, as `guatiao`.

## API overview

| Name | Purpose |
| --- | --- |
| `loads(text, *, format="json", how=0, alloc=None)` | a document as a `guatiao.Value` |
| `dumps(value, *, format="json", pretty=False, how=0, alloc=None)` | a `Value` or `Ref` as text |
| `parse(text, *, format="json", how=0, numbers="auto")` | a document as plain Python, no `Value` built |
| `render(obj, *, format="json", pretty=False, how=0)` | plain Python as text, no `Value` built |
| `ParseError` | a document that does not parse: `format`, `line`, `column`, `message` |
| `BYTES_DATA_URI`, `BYTES_BASE64`, `BYTES_ARRAY`, `BYTES_REFUSE`, `READ_DATA_URIS`, `PRETTY` | the `how` flags |
| `use_library(path)`, `library()` | naming and loading `guatiao_serde` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/guatiao-serde/src/guatiao_serde/AGENTS.md)
is the full API reference and ships inside the wheel. The C ABI is
described by [`guatiao_serde.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-serde/include/guatiao_serde.h).

## Development

```bash
pip install -e "bindings/python/guatiao[dev]" -e "bindings/python/guatiao-serde[dev]"   # from the repository root
pytest bindings/python/guatiao-serde/tests -rs
```

`conftest.py` points `GUATIAO_LIBRARY` at the repository's build output
when the variable is unset; the repository's own `AGENTS.md` says how to
produce it. A test needing a library that is missing skips and says
which.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
