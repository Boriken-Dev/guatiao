# guatiao-intake for Python

[![PyPI version](https://img.shields.io/pypi/v/guatiao-intake.svg)](https://pypi.org/project/guatiao-intake/)
[![Python versions](https://img.shields.io/pypi/pyversions/guatiao-intake.svg)](https://pypi.org/project/guatiao-intake/)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

Taking a value in from a person, for
[guatiao](https://github.com/Boriken-Dev/guatiao/) values: the path
grammar (`agent[1].name`), whether a form fits a schema and how it is
laid out, which fields are shown, and the flat `key -> text` store a front
end keeps. Pure Python over `ctypes`, calling the `guatiao_intake`
library, and built on the [`guatiao`](https://pypi.org/project/guatiao/)
package for the value model and its errors.

> **Status: alpha.** A pure Python package: it carries no native
> library, you point it at one.

## Features

- **Forms over a schema** — check a form document against the schema it
  shows, lay its fields out in sections, decide which are visible given
  what was entered, or take the form a schema implies when nobody wrote
  one.
- **A path names a place in a value** — `agent[1].name`, `env[PATH]`,
  read as a borrowed view of the tree you passed.
- **The flat store** — a value as `key -> text` and back, the way a front
  end keeps what a person typed, with tagged fields, lists and open maps.
- **A provider's form** — read from its `meta` under `PROVIDER_FORM_KEY`
  and checked against its configuration schema.
- **Any guatiao value** — every argument is a `guatiao.Value` or `Ref`,
  whichever library built it, and every answer an ordinary `Value`.

## Installation

```bash
pip install guatiao-intake
```

The package drives the `guatiao_intake` shared library. Say where it is:

```bash
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/lib       # a directory holding guatiao_intake.dll / libguatiao_intake.so / .dylib
export GUATIAO_INTAKE_LIBRARY=/opt/myapp/myapp.dll # or the file itself, whatever it is called
```

Without it, `GUATIAO_LIBRARY` is tried: a directory holding the library,
or a file that exports its functions (an application's own library
carrying several guatiao surfaces), or a file it sits beside. Then the
system's library search. Nothing is loaded at import time; the first call
raises `guatiao.LibraryNotFound`, naming every place it looked.
`guatiao_intake.use_library(path)` names the library from code instead.

## Quick start

```python
import guatiao_intake as intake

problem = intake.check(schema, form_doc)               # None when the form fits the schema
sections = intake.layout(schema, form_doc)             # [{"section": ..., "fields": [...]}, ...]
shown = intake.is_visible(schema, form_doc, "tls.verify", values)
name = intake.at(value, "agent[1].name")               # a borrowed Ref, or None

# A front end that keeps text under flat keys (`agent[0].name`, `env[PATH]`):
field = intake.resolve(schema, "auth.password")        # the schema field, or None
keys = intake.flat_keys(schema, "auth")                # ["auth", "auth.username", ...]
store = intake.flatten(schema, "auth", auth_value)     # {"auth": "userpass", ...}
back = intake.unflatten(schema, "auth", store)         # a Value, or None

with registry.provider_meta("acme_dialer") as meta:   # a provider's form
    form = meta.as_map()[intake.PROVIDER_FORM_KEY]
    assert intake.check(registry.provider_config("acme_dialer"), form) is None
```

## Known limits

- What `at` and `resolve` answer borrows from the tree it was found in:
  read-only, and refused once that tree is closed. `.clone()` copies it
  out.
- 64-bit targets and 32-bit ARM, as `guatiao`.

## API overview

| Name | Purpose |
| --- | --- |
| `check`, `layout`, `is_visible`, `for_schema`, `form_for` | a form over a schema |
| `at` | a path's place in a value |
| `resolve`, `flat_keys`, `flatten`, `unflatten` | the flat `key -> text` store |
| `PROVIDER_FORM_KEY` | where a provider's `meta` carries its form |
| `use_library(path)`, `library()` | naming and loading `guatiao_intake` |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/python/guatiao-intake/src/guatiao_intake/AGENTS.md)
is the full API reference and ships inside the wheel. The C ABI is
described by [`guatiao_intake.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao-intake/include/guatiao_intake.h).

## Development

```bash
pip install -e "bindings/python/guatiao[dev]" -e "bindings/python/guatiao-intake[dev]"   # from the repository root
pytest bindings/python/guatiao-intake/tests -rs
```

`conftest.py` points `GUATIAO_LIBRARY` at the repository's build output
when the variable is unset; the repository's own `AGENTS.md` says how to
produce it. A test needing a library that is missing skips and says
which.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
