# guatiao for MATLAB and GNU Octave

[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/guatiao/matlab/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/guatiao/test.yaml)](https://github.com/Boriken-Dev/guatiao/actions/workflows/test.yaml)

MATLAB and GNU Octave bindings for the
[guatiao](https://github.com/Boriken-Dev/guatiao/) ABI: **one value model
for passing data between languages**, a JSON Schema that describes a
value, and a registry that loads plugin libraries. A `+guatiao` package of
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
- **Be a plugin host** — scan a folder, filter on what a library declares
  before loading it, list providers best first, build an instance from a
  configuration, call a provider through its table with MATLAB values in
  and out, and keep sources of libraries that come and go
  (`reg.refresh()`).
- **A kind is a classdef** — declared once from its C header, with a
  method per slot; a remote is an instance of that class bound to a table,
  so code written against the kind takes a local provider or a remote
  alike.
- **Write a provider in M** — a handle class inheriting the kinds it
  serves and overriding their methods, in a library a host calls the way it
  calls one written in Rust, C, Python, Go, Julia or R; the folder is the
  library, named and versioned by its `DESCRIPTION`.
- **Hand a library an M object** — any object of a class implementing an
  object kind (a listener, a callback) becomes an object a library calls,
  or holds until it is done with it.
- **A configuration is a field list** — `guatiao.Config` writes the JSON
  Schema it describes and reads a configuration back as a struct with its
  defaults.
- **Dates, times, spans and exact lengths** — dates, times of day and
  civil dates and times to the nanosecond, instants, spans and exact
  lengths, read and written by the library itself.
- **Values on the wire** — `v.to_wire()` and `guatiao.Value.from_wire(b)`
  carry a tree exactly as bytes, and `guatiao.wire` announces a schema on
  a channel before its values, with a `Receiver` that checks each one.
- **Redaction** — a copy of a value with every field its schema marks
  secret hidden.
- **JSON, TOML and YAML**, and **form checks against a schema**, through
  the `guatiao-serde` and `guatiao-intake` libraries when they are present.

## Installation

GNU Octave, from a release's `guatiao-<version>.tar.gz`:

```matlab
pkg install guatiao-VERSION.tar.gz   % the release's file; compiles the MEX file
pkg load guatiao
```

MATLAB, from a release's `guatiao.mltbx` (or a checkout's
`bindings/matlab/guatiao/inst` on the path), then once:

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
| `guatiao` | values, the registry, kinds and providers, schemas, time, redaction, the wire and channels |
| `guatiao_serde` | `guatiao.serde.parse`, `guatiao.serde.data`, `guatiao.serde.emit`: JSON, and TOML and YAML when the library has them; found through `GUATIAO_SERDE_LIBRARY` first, or named with `guatiao.serde.use_library` |
| `guatiao_intake` | the `guatiao.intake` functions; found through `GUATIAO_INTAKE_LIBRARY` first, or named with `guatiao.intake.use_library` |

MATLAB R2024a or later, or GNU Octave 8.4 or later, on a 64-bit platform
(Octave also on 32-bit ARM Linux), with a library built for the same
architecture as the process.

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

### Hosting plugins

```matlab
reg = guatiao.Registry('my-host', '1.0');
reg.scan_dir('/opt/plugins', 'kind=greeter');
providers = reg.providers('greeter');

greeter = reg.bind('acme_hello', 'greeterkind.Greeter');   % the kind's class
answer = greeter.greet('ana');      % a guatiao.Value: {greeting: "hello, ana"}
answer.close();
reg.close();
```

### A kind, and a provider of it

A kind is a class declared once, from its C header:

```matlab
classdef Greeter < guatiao.Kind
  properties (Constant, Hidden)
    GreeterKind = guatiao.Kind.describe('greeter', 2695856491, ...
      guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true), ...
      guatiao.slot('shout', {'name', 'string'}, 'Returns', 'string'))
  end
  methods
    function out = greet(self, name), out = self.call('greet', name); end
    function out = shout(self, name), out = self.call('shout', name); end
  end
end
```

A provider is a class inheriting it and overriding its methods:

```matlab
classdef Hello < greeterkind.Greeter
  properties (Constant, Hidden)
    Provider = guatiao.provider('DisplayName', 'Hello, in M')
  end
  methods
    function out = greet(self, name)
      if isempty(name), error('guatiao:BadValue', 'nobody to greet'); end
      out = struct('greeting', ['hello, ' name]);
    end
  end
end
```

```matlab
reg.register_library(guatiao.library_of('mypackage'))   % the folder declaring it is the library
```

A host in this session, written in any language, calls `Hello` through
the same table it would call a Rust or C greeter through.

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
x = guatiao.serde.data('{"name": "ana", "n": 1.10}');   % the same as doc.data(), no value built
guatiao.serde.emit(x)                                   % MATLAB data: no value built either
```

### Dates and spans

```matlab
char(guatiao.Date(2026, 1, 31).add(guatiao.Span('P1M')))            % '2026-02-28'
char(guatiao.time.parse('timestamp', '2026-10-21T19:15:00.5+05:30'))  % '2026-10-21T13:45:00.5Z'
```

## Known limits

- **M code runs only on the interpreter's thread, inside a call into
  guatiao.** A library that calls a provider written in M from a thread of
  its own, or with no call into guatiao in progress, is answered
  `Unsupported` at once, the provider never running; a destroy it asks for
  there waits for the interpreter's thread. Hosts that call providers from
  worker threads need providers written in another language. C code of
  your own that calls M providers runs inside `guatiao.serving`.
- A 1x1 array is a scalar: wrap it in a cell (`{5}`) for a list of one. A
  matrix is a list of rows; arrays of more than two dimensions are
  refused.
- A map whose keys are not field names reads back as a `containers.Map`,
  which orders its keys itself: read `keys()` for the tree's order.
- Octave cannot index the result of a package function call in place:
  `x = guatiao.intake.at(v, 'a'); x.data()`, not
  `guatiao.intake.at(v, 'a').data()`.
- A remote cannot run a kind's default body for a slot a provider leaves
  out, as a Rust caller does: `has_slot` asks, and the call raises
  `Unsupported`.
- An M library is registered in a host in the same session; there is no
  library file a foreign host's scan loads, since that would need MATLAB
  or Octave embedded in it.
- 64-bit platforms, and Octave on 32-bit ARM Linux; not 32-bit x86, where
  the guatiao library does not build. On Windows ARM64, Octave and MATLAB
  are x64 builds and need an x64 build of the library.

## API overview

| Name | Purpose |
| --- | --- |
| `guatiao.Value`, `guatiao.Node`, `guatiao.Number`, `guatiao.Absent`, `guatiao.Allocator` | the value model |
| `guatiao.Config`, `guatiao.field`, `guatiao.validate` | configurations and their schemas |
| `guatiao.Date`, `TimeOfDay`, `DateTime`, `Timestamp`, `Span`, `Duration`, `guatiao.time.*` | dates, times, spans, exact lengths |
| `guatiao.redact` | secrets hidden |
| `guatiao.wire.announce`, `send`, `close`, `guatiao.wire.Receiver` | values on the wire and channels |
| `guatiao.serde.parse`, `guatiao.serde.data`, `guatiao.serde.emit` | JSON, TOML, YAML, as a value or as MATLAB data |
| `guatiao.intake.*` | forms and the flat projection |
| `guatiao.Registry`, `guatiao.Instance` | hosting plugins: `scan_dir`, `providers`, `bind`, `create`, `refresh`, ... |
| `guatiao.Kind`, `guatiao.slot`, `guatiao.lent`, `guatiao.taken`, `guatiao.Buffer` | a kind, declared from its C header |
| `guatiao.provider`, `guatiao.Library`, `guatiao.library_of`, `guatiao.serving` | providers and libraries written in M |
| `guatiao.use_library`, `guatiao.has_export`, `guatiao.statuses`, `guatiao.build` | the library, errors, the MEX file |

[`AGENTS.md`](https://github.com/Boriken-Dev/guatiao/blob/main/bindings/matlab/guatiao/inst/AGENTS.md)
is the full API reference and ships in the package. The C ABI is
described by
[`guatiao.h`](https://github.com/Boriken-Dev/guatiao/blob/main/crates/guatiao/include/guatiao.h).

## Development

```bash
cd bindings/matlab/guatiao/tests
GUATIAO_LIBRARY=../../../../target/debug octave-cli --eval "run_tests('Exit', true)"
matlab -batch "cd bindings/matlab/guatiao/tests; assert(run_tests())"
```

`run_tests` builds the MEX file when it is not on the path, then runs
every `t_*.m`. The tests need the three libraries in one folder, named by
`GUATIAO_LIBRARY`, and a checkout for the shared vectors and the examples;
the repository's own `AGENTS.md` says how to produce the build.
`GUATIAO_C_GREETER`, `GUATIAO_GO_LIBRARY` and `GUATIAO_PY_BRIDGE` add the
greeters written in C, Go and Python; `GUATIAO_REQUIRE_ALL=1` makes a
skipped case a failure. The examples (`examples/greeter_kind/matlab`,
`examples/matlab_provider/mlgreeter`, `examples/matlab_host`) hold the
greeter kinds, a library of providers and a host written in M.

## License

MPL 2.0 — see [LICENSE](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE).
