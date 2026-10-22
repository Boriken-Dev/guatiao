# guatiao for MATLAB and GNU Octave

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/matlab/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

MATLAB and GNU Octave bindings for the
[guatiao](https://github.com/Boriken-Dev/guatiao/) ABI: **one value model
for passing data between languages**, a JSON Schema that describes a
value, its JSON, TOML and YAML text, and forms. A `+guatiao` package of
`classdef` and functions that both MATLAB and Octave run, over one small
MEX file built from C on your machine, which opens the library at run
time. It works with any shared library that exports the ABI, whether that
is `guatiao` itself or an application that carries it.

> **Status: alpha.** The package carries no native library of its own:
> you point it at one.

## Features

- **Value trees read the way MATLAB reads data** — build a tree from
  structs, cell arrays, `char`, numbers, `uint8` and `containers.Map`,
  change it in place through views (`v.at('tags').push('c')`), read it
  back with `data()`.
- **Numbers keep their exact text** — `1.10` stays `1.10`, an integer
  past 2^53 reads back as `int64`/`uint64`, and anything wider survives as
  `guatiao.Number('18446744073709551615.5')`.
- **A configuration is a field list** — `guatiao.Config` writes the JSON
  Schema it describes and reads a configuration back as a struct with its
  defaults.
- **Dates, times, spans and exact lengths** — dates, times of day and
  civil dates and times to the nanosecond, instants, spans and exact
  lengths, read and written by the library itself.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.

## Installation

GNU Octave, from a release's `guatiao-<version>.tar.gz`:

```matlab
pkg install guatiao-0.0.0-alpha.1.tar.gz   % compiles the MEX file
pkg load guatiao
```

MATLAB, from a release's `guatiao.mltbx` (or a checkout's
`bindings/matlab/inst` on the path), then once:

```matlab
guatiao.build()
```

Both need a C compiler their `mex` / `mkoctfile` supports and libffi
(`libffi-dev` on Debian and Ubuntu, `libffi-devel` on Fedora; macOS and
Octave's Windows installer carry their own; MATLAB with Visual C++ on
Windows takes vcpkg's `libffi:x64-windows-static-md`, named with
`guatiao.build('FfiDir', ...)`). The package drives a shared library that
exports the guatiao ABI. Say where it is:

```matlab
setenv('GUATIAO_LIBRARY', '/opt/myapp/lib')          % a folder holding guatiao.dll / libguatiao.so / libguatiao.dylib
setenv('GUATIAO_LIBRARY', '/opt/myapp/myapp.dll')    % or the file itself, whatever it is called
```

or call `guatiao.use_library(path)` before the first call. Without either,
the platform's library search is tried by file name.

| Exports from | Needed for |
| --- | --- |
| `guatiao` | values, schemas, time, redaction |
| `guatiao_serde` | `guatiao.serde.parse`, `guatiao.serde.emit`: JSON, and TOML and YAML when the library has them |
| `guatiao_intake` | the `guatiao.intake` functions |

MATLAB R2024a or later, or GNU Octave 8.4 or later, on a 64-bit platform,
with a library built for the same architecture as the process.

## Quick start

### Values

```matlab
v = guatiao.Value(struct('host', '10.0.0.1', 'port', 5900, 'tags', {{'a', 'b'}}));
v.set('port', 5901);
tags = v.at('tags');
tags.push('c');

port = v.at('port');
port.data()        % 5901
v.keys()           % {'host', 'port', 'tags'}, insertion order
tags.data()        % {'a', 'b', 'c'}
v.close();
```

A value owns its tree and frees it on `close` (or when its last reference
goes). `v.at('tags')` is a view: a path into the tree that is walked again
on every use, so it stays valid while the tree grows around it.

**Numbers keep their exact text**:

```matlab
n = guatiao.Value(guatiao.Number('1.10'));
n.data('Numbers', 'text')   % guatiao.Number 1.10
n.data()                    % 1.1
```

### A configuration's schema

```matlab
Listen = guatiao.Config('host', guatiao.field('string', 'Default', '0.0.0.0', 'Doc', 'Where to listen.'), ...
                        'port', guatiao.field('integer'));
Listen.schema()                       % the JSON Schema, as MATLAB data
s = Listen.read(struct('port', 5900));
s.host                                % '0.0.0.0'
```

### JSON, TOML and YAML

```matlab
doc = guatiao.serde.parse('{"name": "ana", "n": 1.10}');
guatiao.serde.emit(doc, 'yaml')
```

### Dates and spans

```matlab
char(guatiao.Date(2026, 1, 31).add(guatiao.Span('P1M')))            % '2026-02-28'
char(guatiao.time.parse('timestamp', '2026-10-21T19:15:00.5+05:30'))  % '2026-10-21T13:45:00.5Z'
```

## Known limits

- A 1x1 array is a scalar: wrap it in a cell (`{5}`) for a list of one. A
  matrix is a list of rows; arrays of more than two dimensions are
  refused.
- A map whose keys are not field names reads back as a `containers.Map`,
  which orders its keys itself: read `keys()` for the tree's order.
- Octave cannot index the result of a package function call in place:
  `x = guatiao.intake.at(v, 'a'); x.data()`, not
  `guatiao.intake.at(v, 'a').data()`.
- 64-bit platforms only. On Windows ARM64, Octave and MATLAB are x64
  builds and need an x64 build of the library.

## API overview

| Name | Purpose |
| --- | --- |
| `guatiao.Value`, `guatiao.Node`, `guatiao.Number`, `guatiao.Absent`, `guatiao.Allocator` | the value model |
| `guatiao.Config`, `guatiao.field`, `guatiao.validate` | configurations and their schemas |
| `guatiao.Date`, `TimeOfDay`, `DateTime`, `Timestamp`, `Span`, `Duration`, `guatiao.time.*` | dates, times, spans, exact lengths |
| `guatiao.redact` | secrets hidden |
| `guatiao.serde.parse`, `guatiao.serde.emit` | JSON, TOML, YAML |
| `guatiao.intake.*` | forms and the flat projection |
| `guatiao.use_library`, `guatiao.has_export`, `guatiao.statuses`, `guatiao.build` | the library, errors, the MEX file |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/matlab/inst/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/matlab/tests
GUATIAO_LIBRARY=../../../target/debug octave-cli --eval "run_tests('Exit', true)"
matlab -batch "cd bindings/matlab/tests; assert(run_tests())"
```

`run_tests` builds the MEX file when it is not on the path, then runs
every `t_*.m`. The tests need the three libraries in one folder, named by
`GUATIAO_LIBRARY`, and a checkout for the shared vectors; the
repository's own `AGENTS.md` says how to produce the build.
`GUATIAO_REQUIRE_ALL=1` makes a skipped case a failure.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
