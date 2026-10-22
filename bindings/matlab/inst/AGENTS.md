# guatiao (MATLAB and GNU Octave) — API header

MATLAB and GNU Octave bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

The `+guatiao` package of `classdef` and functions, in the subset both
MATLAB (R2024a or later) and GNU Octave (8.4 or later) run, over one C MEX
gateway, `guatiao_mex`, which the package calls and nothing else should.
64-bit platforms. It carries no guatiao library: one is opened at run time.

## Building the gateway

The package ships the gateway's C sources (`src/`). An Octave package
installed with `pkg install` builds it at install (its `src/Makefile`);
from a folder or a MATLAB toolbox, run once:

```matlab
guatiao.build()                         % mex (MATLAB) or mkoctfile --mex (Octave)
guatiao.build('FfiDir', 'C:\vcpkg\installed\x64-windows-static-md')
```

`guatiao.build(...)` options: `'Output'` (folder for the MEX file, default
the package's), `'FfiDir'` (a libffi prefix with `include/` and `lib/`,
default `GUATIAO_FFI_DIR`, else the compiler's own search), `'Strict'`
(warnings as errors, gcc and clang), `'Flags'` (a cell of further
arguments), `'Verbose'`. It needs a C compiler the host supports and
libffi: `libffi-dev` / `libffi-devel` on Linux, the SDK's on macOS,
Octave's Windows bundle carries its own, MATLAB with Visual C++ takes
vcpkg's `libffi:x64-windows-static-md`. Answers the file written.

## Finding the native library

Nothing native is touched until the first call that needs it; then:

1. `guatiao.use_library(path)`, called before that first call: a file
   exporting the ABI, whatever it is called, or a folder (`guatiao_serde`
   and `guatiao_intake` are looked for beside it). Naming another file once
   one is loaded raises `guatiao:Busy`.
2. `GUATIAO_LIBRARY`: a folder holding `guatiao.dll` / `libguatiao.so` /
   `libguatiao.dylib`, or a file.
3. The platform's own search, by file name.

Not found raises `guatiao:NotFound:library`; an export the loaded build
lacks raises `guatiao:Unsupported:symbol` naming its feature when it is
used. `guatiao.has_export(symbol, library)` asks (`library`: `'guatiao'`,
`'guatiao_serde'`, `'guatiao_intake'`). A library stays loaded for the
session, and so does the gateway (it locks itself; `clear mex` does not
unload it). The library must be built for the process's architecture (an
x64 MATLAB or Octave on Windows ARM64 loads x64 DLLs).

## Errors

Every failure is an `MException` whose identifier is
`guatiao:<Status>[:<detail>]`: `OK` 0, `BadValue` 7, `AllocFailed` 8,
`WrongKind` 9, `NotFound` 10, `NullArgument` 11, `Gone` 12, `Unsupported`
13, `Busy` 14, `Internal` 100 (`guatiao.statuses()` is the table). Details:
`parse` (a document that does not parse), `unparsed` (text a time grammar
refuses), `closed` (a closed value, registry, instance or remote),
`library`, `symbol`. Octave puts `guatiao_mex: ` before the words.

## Values

`v = guatiao.Value(x)` builds a value from MATLAB data and owns its root;
`guatiao.Value()` holds absent; `guatiao.Value(x, 'Alloc', a)` builds in
allocator `a`. `v.close()` frees it (a second close does nothing); the
last reference going frees it too. `v.is_open()`.

```matlab
v = guatiao.Value(struct('host', '10.0.0.1', 'port', 5900, 'tags', {{'a', 'b'}}));
v.set('port', 5901);
tags = v.at('tags');
tags.push('c');
p = v.at('port');
p.data()               % 5901
v.keys()               % {'host', 'port', 'tags'}
v.close();
```

Methods of a value and of a view (`guatiao.Tree`):

| method | does |
| --- | --- |
| `at(sel)` | a view (`guatiao.Node`) of a map's entry (a key, or a position from 1) or a list's element (from 1); `[]` for a missing key; past the end raises `NotFound` |
| `data(...)` | the node as MATLAB data; `'Numbers'`: `'auto'`, `'text'`, `'double'`; `'Maps'`: `'auto'`, `'map'` |
| `tag()` | `'absent'`, `'null'`, `'bool'`, `'number'`, `'string'`, `'bytes'`, `'list'`, `'map'` |
| `length()` | entries of a map, elements of a list, 1 for any other value, 0 for null and absent |
| `keys()`, `has(key)` | a map's keys in insertion order (a cell row), whether it holds one |
| `isabsent()`, `isnull()` | |
| `path()` | the node's path, intake grammar, positions from 0: `'agent[1].name'` |
| `set(sel, x)` | sets a map's entry (a new key goes last) or replaces a list's element, appending at length + 1 |
| `push(x, ...)`, `insert(at, x)` | appends to a list; inserts before position `at` (from 1) |
| `remove(sel)`, `clear()` | removes a key or element (nothing there is no error); empties a map or list |
| `merge(src)` | copies a map's entries in, replacing collisions |
| `copy('Alloc', a)` | a deep copy, a new `guatiao.Value` |

A view is a path walked again on every use, never a cached address; it
keeps its value alive. A view of a closed value raises `guatiao:Gone:closed`,
one whose node is gone `guatiao:NotFound`. What is added is built in the
allocator the container records. A failed write leaves the tree as it
was.

Octave cannot index the result of a package function call in place
(`guatiao.intake.at(v, 'a').data()` fails there): name the result first.
A method's result on a variable (`v.at('a').data()`) works in both.

## MATLAB data and values

| MATLAB | value |
| --- | --- |
| `[]` (an empty numeric or logical array) | null |
| `guatiao.Absent()` | absent; as a struct field or map entry, no entry |
| logical, double, single, `int8`..`uint64` scalar | boolean, number |
| a double | the shortest text that reads back as it; a whole number below 1e21 as the integer; NaN and infinities refused |
| `guatiao.Number('1.10')` | a number, its exact text (JSON's grammar) |
| `char` row (`''` included) | string; a character matrix is refused |
| `uint8` array | bytes |
| a numeric or logical vector, a cell vector | list; a matrix a list of rows |
| struct (1x1) | map, fields in order; a struct array a list of maps |
| `containers.Map` with `char` keys | map |
| a value or view | a deep copy |
| `guatiao.Date`, `TimeOfDay`, `DateTime`, `Timestamp`, `Span`, `Duration`; MATLAB's `datetime` and `duration`, `string` | their text forms; an exact length `[seconds, nanoseconds]` |
| an object with a `guatiao_value()` method | what it answers |
| any other object | a map of its public properties |

Function handles, sparse and complex arrays and arrays of more than two
dimensions are refused. Reading back: null is `[]`, absent
`guatiao.Absent`, a boolean `true`/`false`, a string a `char` row, bytes a
`uint8` row, a list a cell row, a map a struct when every key is a field
name (letters, digits, `_`, a letter first, at most 63) else a
`containers.Map` (which orders keys itself). A number, with `'Numbers'`
`'auto'`: an integer spelling a double, up to 2^53, past it an `int64` or
`uint64` that holds it, else a `guatiao.Number`; a fraction or exponent the
nearest double (a `guatiao.Number` when it overflows). `'text'` answers
every number as a `guatiao.Number`, `'double'` as the nearest double. Both
directions stop past 128 nested containers. Text crosses as UTF-8.

`guatiao.Number(text)`: `Text`, `double(n)`, `char(n)`, `==`.
`guatiao.Allocator.counting()`: an allocator counting live blocks
(`live()`, `records(node)`), kept for the session.

## Configurations and their schemas

```matlab
C = guatiao.Config('host', guatiao.field('string', 'Doc', 'Where to listen.', 'minLength', 1), ...
                   'port', guatiao.field('integer', 'Default', 5900));
C.schema()                               % the JSON Schema, as MATLAB data
s = C.read(struct('host', 'h'))          % s.port == 5900
```

`guatiao.Config(name, spec, ..., 'Doc', text)`: names are MATLAB
identifiers; a spec is `guatiao.field(type, ...)` or a type alone.
`guatiao.field(type, 'Default', v, 'Doc', text, 'Nullable', tf, keyword,
value, ...)`: types `'string'`, `'integer'` (a whole number a double holds
exactly), `'number'`, `'boolean'`, `'bytes'`, `'list'`, `'map'`, `'any'`,
`'date'`, `'time'`, `'datetime'`, `'timestamp'`, `'span'`, `'duration'`,
or a nested `guatiao.Config`; any other option is a JSON Schema keyword
added as it is. `schema()`: an object, properties in order, undeclared keys
refused, a field required unless it has a default or is nullable.
`read(x)` (a value, a view or MATLAB data): a struct with every field,
defaults filled in, each read as its type (a date a `guatiao.Date`, a
duration a `guatiao.Duration`); keys it does not declare are ignored; a
refusal raises `guatiao:BadValue` naming the path (`at port: want an
integer, got string`).

`guatiao.validate(schema, value)`: `[]` when the JSON Schema accepts the
value, else the refusal as data (key, message, ...), never quoting the
value.

## Dates, times, instants, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>; every text is read and
written by the library's own functions.

| class | form | built from |
| --- | --- | --- |
| `guatiao.Date` | `'2026-10-21'` | `(year, month, day)` or text |
| `guatiao.TimeOfDay` | `'13:45:00.5'`, to the nanosecond | `(hour, minute, second, nanosecond)` or text |
| `guatiao.DateTime` | `'2026-10-21T13:45:00.5'` | `(date, time)`, seven fields or text |
| `guatiao.Timestamp` | `'2026-10-21T13:45:00.5Z'`, read with any offset | `(seconds, nanoseconds)` since 1970 UTC, or text; `posix()` |
| `guatiao.Span` | `'P1DT2H'`, ISO 8601 or `'1h 30m'` | text, or `'Years'`..`'Nanoseconds'` name-value; `-s`, `==` unit by unit |
| `guatiao.Duration` | `[seconds, nanoseconds]`, never negative | `(seconds, nanoseconds)` or seconds; `seconds(d)` |

`char(x)` is the canonical text. `guatiao.time.parse(form, text)` (forms
`'date'`, `'time'`, `'datetime'`, `'timestamp'`, `'span'`),
`guatiao.time.normalize(form, text)`, `guatiao.time.add_span(x, span)` (a
`Date`, time units refused, or a `DateTime`; `x.add(span)` is the same),
`guatiao.time.read_length(x)` (an exact length in any of its three
spellings). Text a grammar refuses raises `guatiao:BadValue:unparsed`
naming the byte offset and what was expected, never quoting the text;
with two outputs `parse` and `normalize` answer `[]` and a problem struct
(`form`, `at`, `expected`) instead. Units and seconds past 2^53 are
refused.

## Secrets

`guatiao.redact(schema, value, placeholder, 'Alloc', a)`: a new
`guatiao.Value` with every field the schema marks (`x-sensitive` or
`writeOnly`) replaced by `placeholder` (default `'[redacted]'`) at any
depth, and whatever the schema does not describe hidden too. Local
`$ref`s are followed. `WrongKind` for a schema that is not a map.

## Serde: JSON, TOML, YAML

`[v, problem] = guatiao.serde.parse(text, format, 'ReadDataUris', tf,
'Alloc', a)`: a `guatiao.Value`; the format `'json'` (default), `'toml'`
or `'yaml'` (TOML and YAML need a `guatiao_serde` built with them). A
document that does not parse raises `guatiao:BadValue:parse`, its message
naming the line and column (from 1, the column in characters), never
quoting the text; with two outputs it answers `[]` and `problem` =
struct(format, line, column, reason). `guatiao.serde.emit(x, format,
'Pretty', tf, 'Bytes', how)`: text from a value, a view or MATLAB data;
`how` is `'data_uri'` (default), `'base64'`, `'array'` or `'refuse'`.
TOML writes only a map, and no null. A number keeps its exact text.

## Intake

```matlab
guatiao.intake.check(schema, form)              % [] or the error as data
guatiao.intake.layout(schema, form)             % a cell of sections
guatiao.intake.is_visible(schema, form, 'key', values)
guatiao.intake.at(value, 'agent[1].name')       % a view, a copy, or []
guatiao.intake.for_schema(schema)               % a guatiao.Value
guatiao.intake.form_for(form, schema, 'path')   % a guatiao.Value, or []
guatiao.intake.resolve(schema, 'auth.password') % a view, a copy, or []
guatiao.intake.flat_keys(schema, 'auth')        % a cell of text
guatiao.intake.flatten(schema, 'auth', value)   % a containers.Map of flat key to text
guatiao.intake.unflatten(schema, 'auth', store) % a guatiao.Value, or []
```

Every argument is a value, a view or MATLAB data. `at` and `resolve`
answer a view of the argument's own node when it is a value or view, and
a copy when it is MATLAB data. `unflatten`'s store is a `containers.Map`
or a struct of text; `[]` means nothing stored, or a key that does not
apply.

## Octave and MATLAB

- Typed property declarations (`x char = ''`) are a syntax error in
  Octave: configurations are `guatiao.Config` field lists.
