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

The package and its sub-packages each call their own library:
`guatiao.*` the `guatiao` library, `guatiao.serde.*` `guatiao_serde`,
`guatiao.intake.*` `guatiao_intake`, through the one gateway. Nothing
native is touched until the first call that needs a library; then:

1. `guatiao.use_library(path)` (`guatiao.serde.use_library`,
   `guatiao.intake.use_library` for theirs), called before that first
   call: a file exporting the library's functions, whatever it is called,
   or a folder. Naming another file once one is loaded raises
   `guatiao:Busy`.
2. For `guatiao_serde` and `guatiao_intake`, `GUATIAO_SERDE_LIBRARY` /
   `GUATIAO_INTAKE_LIBRARY`: a file, taken as it is, or a folder holding
   the library under its platform name.
3. `GUATIAO_LIBRARY`: a folder holding `guatiao.dll` / `libguatiao.so` /
   `libguatiao.dylib` (or `guatiao_serde.dll` and so on), or a file: taken
   as it is for `guatiao`, and for the others when it exports their
   functions (an application's own library carrying several surfaces),
   else their library beside it.
4. The platform's own search, by file name.

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
refuses), `malformed`, `noSchema`, `notASchema`, `invalid` (the wire
and its channels), `closed` (a closed value, registry, instance, remote
or receiver),
`library`, `symbol`, `floor` (a table not the kind's), `provider` (the
status a provider answered). Octave puts `guatiao_mex: ` before the words.
A provider method answers a status by raising one:
`error('guatiao:BadValue', 'nobody to greet')`; any other error answers
`Internal` with its message.

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

## Values on the wire, and channels

`b = v.to_wire()` (a value or a view) is the node in the wire encoding, a
`uint8` row: a tag byte per node, LEB128 lengths, a number as its own
text, bytes as bytes, a map in its order, the same bytes in every
language. `guatiao.Value.from_wire(b, 'Alloc', a)` reads it back; bytes
that are not exactly one value raise `guatiao:BadValue:malformed`.

A channel announces a schema before its values, and a receiver checks
every value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```matlab
r = guatiao.wire.Receiver();
e = r.accept(guatiao.wire.announce(1, schema));   % e.kind 'schema', e.value its copy
e.value.close();
e = r.accept(guatiao.wire.send(1, struct('greeting', 'hi')));
x = e.value;                                       % the checked value
x.data()
x.close();
r.accept(guatiao.wire.close(1));                   % e.kind 'close', e.value []
r.close();
```

- `guatiao.wire.announce(channel, schema)`, `guatiao.wire.send(channel,
  value)` (a value, a view or MATLAB data) and `guatiao.wire.close(channel)`
  answer the frame, a `uint8` row. A channel is a whole number from 0, a
  `uint64` past 2^53.
- `r = guatiao.wire.Receiver('Alloc', a)`; `r.accept(frame)` answers
  `struct(kind, channel, value)`: `kind` `'schema'`, `'value'` or `'close'`,
  `channel` a `uint64`, `value` a `guatiao.Value` (the schema's copy or the
  checked value; close it) or `[]`. `r.close()` (a second does nothing),
  `r.is_open()`; the last reference going frees it too.
- A refusal raises `guatiao:BadValue:malformed` (bytes that are not a
  frame), `guatiao:NotFound:noSchema`, `guatiao:WrongKind:notASchema` or
  `guatiao:BadValue:invalid` (a value its schema refuses), its message the
  library's words; with two outputs, `[e, problem] = r.accept(frame)`
  answers `e = []` and `problem` = struct(`refusal`, `status`, `channel`,
  `kind` (the frame's kind byte), `at` (the byte of the frame, for
  `malformed`), `key`, `expected`, `known` (for `invalid`), `message`).
  A refusal changes nothing the receiver keeps and quotes nothing it was
  given.

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
guatiao.intake.provider_form_key()   % 'form': a provider's form in reg.provider_meta(key)
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

## Registry

```matlab
reg = guatiao.Registry('my-host', '1.0');
reg.scan_dir('/path/to/plugins', 'kind=greeter');
reg.providers('greeter')                         % a cell of structs, best first
g = reg.bind('acme_hello', 'greeterkind.Greeter');   % a remote (see Kinds)
inst = reg.create('acme_shouter', struct('prefix', 'hey'));
s = inst.bind('greeterkind.Greeter');
inst.close(); reg.close();
```

`guatiao.Registry(id, version, 'Alloc', a)`; every answer that is data is
plain MATLAB data, already freed. `close()` (the libraries it loaded stay
mapped; the last reference going closes it too), `is_open()`. Methods:
`load_file(path)`, `register_entry(name, entry)` (an entry function's
address, `uint64`), `register_library(lib)` (see Providers), `scan_dir(dir,
rules, descending)` and `scan_path(spec, rules, descending)` (`rules`: one
`[!]KEY=VALUE` per line, matched against what a library declares before
it is mapped), `libraries()`, `providers(kind)`, `available(kind)`,
`why_not(kind)`, `provider(key)`, `best(kind)`, `provider_available(key)`
(struct(available, reason)), `keyed_by(template)` /
`libraries_keyed_by(template)` (`%id` by default; `%name`, `%library`,
`%version`, `%source`), `set_priority(id, p)` / `priority(id)`, `provider_table(key,
kind)` (struct(table, size, ctx), addresses as `uint64`, `table` `[]` for
none; no `kind` for a single table), `provider_config(key)` (a copy of the
schema, a `guatiao.Value`, or `[]`), `provider_meta(key)` (what the
provider declared beyond its schema, its notices or its form under
`guatiao.intake.provider_form_key()`: a copy outliving the library, or
`[]`; `guatiao:NotFound` for an unknown key), `host()` (the
`guatiao_host_info *`).

`create(key, config)` checks the configuration against the provider's
schema first (`BadValue` naming the field), then answers a
`guatiao.Instance`: `close()` reaches the provider that built it even
after a retire or a refresh loaded a new build under the key; `bind(cls)`,
`is_open()`, `Key`. The last reference going closes it.

`retire(key)` takes a library out and leaves it mapped until nothing uses
it; `collect()` closes every retired library whose own `unload` agrees and
answers their keys, as does the next load, retire or unload.
`unload(key, unchecked)` unmaps when the library agrees: `Busy` while
something it handed out lives, `Unsupported` with no `unload` function,
`WrongKind` for a library linked rather than mapped, `NotFound` for none.
Both take the LIBRARY key (`%id` by default).

**Sources**: `add_source_file(path, config)` maps a file exporting
`guatiao_source` (a bridge to another language) and adds the source it
answers; `watch_dir(dir, rules)` watches a folder; `refresh()` makes the
registry agree with every source: struct(loaded, retired, skipped,
refused). Nothing refreshes on its own.

## Kinds: calling a table, serving one

A kind is a `classdef` in the kind author's package, declared once from
its C header:

```matlab
classdef Greeter < guatiao.Kind
  properties (Constant, Hidden)
    GreeterKind = guatiao.Kind.describe('greeter', 2695856491, ...
      guatiao.slot('greet', {'name', 'string'}, 'Returns', 'map', 'Required', true, 'Fallible', true), ...
      guatiao.slot('shout', {'name', 'string'}, 'Returns', 'string'), ...
      guatiao.slot('start', {'listener', guatiao.taken('greeterkind.Listener')}, ...
                   'Returns', 'greeterkind.Conversation', 'Fallible', true))
  end
  methods
    function out = greet(self, name), out = self.call('greet', name); end
    function out = shout(self, name), out = self.call('shout', name); end
    function out = start(self, listener), out = self.call('start', listener); end
  end
end
```

The description is a `Constant` property named `<Class>Kind` (two kinds
a provider inherits must not share a property name).
`guatiao.Kind.describe(name, floor_hash, slot, ..., 'Object', tf)`: the C
name, the floor hash the header defines, a `guatiao.slot` per slot in the
header's order (required ones first); an object kind's `destroy` is its
remote's `close()`. `guatiao.slot(name, {argname, shape, ...}, 'Returns',
shape, 'Required', tf, 'Fallible', tf)`. Each slot has a method calling
`self.call(slotname, args...)`.

| shape | C | the caller passes | the provider receives / returns |
| --- | --- | --- | --- |
| `'string'` | `guatiao_str` | `char` | `char` (UTF-8) |
| `'bytes'` | `guatiao_bytes` | `uint8` | `uint8` (a copy) |
| `guatiao.lent('bytes')` | `guatiao_bytes_mut` | a `guatiao.Buffer(n)`, written in place | a `guatiao.Buffer` (`Data`), copied back when the method returns |
| `'bool'`, `'int8'`..`'uint64'`, `'float'`, `'double'`, `'size'`, `'intptr'` | the C scalar | a number or logical that fits (`int64`/`uint64` exactly) | a double, or `int64`/`uint64` past 2^53; logical for `bool` |
| `'value'`, `'map'`, `'list'` | `const guatiao_value *` / `map *` / `list *` | a value, a view or MATLAB data | a read-only `guatiao.Value`, valid for the call |
| `guatiao.lent('value'/'map'/'list')` | the same without `const`, lent | a value or view, changed in place | a writable `guatiao.Value`, valid for the call; what is added is built in the allocator the tree records, and what a callee built elsewhere, or wrote over it, is moved, once the call returns, into the allocator the tree recorded when it was lent |
| `guatiao.taken('value'/'map'/'list')` | the same, handed over | a value (moved: it holds absent afterwards), a view (copied) or MATLAB data | a `guatiao.Value` the method owns |
| answer `'value'`, `'map'`, `'list'` | `guatiao_value/map/list *out` | a `guatiao.Value`, moved into this binding's allocator on arrival when the slot built it in its own | MATLAB data or a value, built in the allocator the carrier names (`[]`, `{}` and `struct()` the empty container) |
| answer `'string'` | `guatiao_string *out` | `char` | `char` |
| answer scalar | `T *out` | a number or logical | a number or logical |
| no answer | | `[]` | ignored |
| `guatiao.taken(K)` (object kind class) | `guatiao_object` | an object of class `K` (a subclass implementing it) or a remote of `K` this side owns (handed over) | a remote of `K`, owned: `close()` destroys it |
| `guatiao.lent(K)` | `const guatiao_object *` | the same, or any remote of `K` | a remote of `K`, valid for the call |
| answer `K` | `guatiao_object *out` | a remote of `K`, owned | an object of class `K`, or a remote it owns |

**A remote** is an instance of the kind's class bound to a table
(`Registry.bind`, `Instance.bind`, an object a slot answers): its methods
call the slots. `has_slot(name)` says whether its table carries a slot (a
slot appended after a library was built is missing from an older table,
and calling it raises `Unsupported`); `is_bound()`; `close()`. A refusal
raises `guatiao:<Status>:provider` with the provider's words. A table
smaller than the kind's required slots or with another floor hash is
refused (`guatiao:WrongKind:floor`). A remote from `Registry.bind` is valid
while the registry is open and the library loaded (`guatiao:Gone:closed`
once the registry is closed); one from `Instance.bind` until the instance
is closed. An object remote this side owns is destroyed when closed or
when its last reference goes. Arguments a call refuses before it reaches
the table (a wrong shape) leave everything as it was.

**An M object as an object of a kind**: an instance of a class inheriting
the object kind's class and overriding its methods. Its table is built
once per kind and class (a slot for each method the class overrides, null
for the rest) and kept for the session. What C holds of it is a ctx
shaped like an address (`65536 + 16 * id`, never reused), so a call naming
one released answers `Gone`. It is held until its holder destroys it, when
its class's own `close()` runs if it defines one, or until the call
returns (lent).

`guatiao.serving(fn)` runs `fn` inside a call into guatiao: C code of your
own (a MEX file, a library called through `calllib`) that calls a slot an
M object serves needs it (see Threads).

## Providers written in M

```matlab
classdef Hello < greeterkind.Greeter & greeterkind.Counter
  properties (Constant, Hidden)
    Provider = guatiao.provider('DisplayName', 'Hello, in M')
  end
  properties
    asked = 0
  end
  methods
    function out = greet(self, name)
      if isempty(name), error('guatiao:BadValue', 'nobody to greet'); end
      out = struct('greeting', ['hello, ' name]);
    end
    function n = count(self)
      self.asked = self.asked + 1; n = self.asked;
    end
  end
end

reg.register_library(guatiao.library_of('mypackage'));
```

**A provider** is a handle class inheriting the kinds it serves and
overriding their methods, declared by a `Constant` `Provider` property:
`guatiao.provider('Serves', classes, 'Name', n, 'Id', id, 'DisplayName',
d, 'Version', v, 'Meta', struct, 'Config', guatiao.Config, 'ConfigSchema',
data, 'Make', fn, 'Default', tf, 'Available', fn)`. `Serves` defaults to
the kinds it inherits; `Name` to its class name in snake case; `Id` to
`<library id>_<name>`. With `Config`, each configuration the host checked
against `Config.schema()` builds an instance through the class's
constructor with `Config.read(config)` (a struct); with `ConfigSchema`,
with the configuration as plain data; `Make(config)` replaces the
constructor. With neither, its constructor with no arguments builds its
one instance (`Default`), when the library's descriptor is built.
`Available` answers `[]` when the provider can run here and why not
otherwise, asked every time a host asks. A required slot the class does not
override (its defining class is still the kind's), an object kind among
`Serves`, or two providers sharing an id is refused when the library is
made. A method raising `guatiao:<Status>` answers that status and its
words; any other error answers `Internal`. An instance (or its one
instance) whose class defines `close()` is closed when its host destroys
it.

**A library**: `guatiao.library_of(where, 'Id', id, 'Version', v, 'Meta',
struct, 'Providers', classes)`, `where` a folder, an installed Octave
package's name, or a namespace (`+folder`) on the path. Id, version and
meta from the folder's `DESCRIPTION` (Octave's package format;
`packinfo/DESCRIPTION` once installed): `Name`, `Version`, `Title` as
`summary`, `Description`, `Author` as `authors`, `Maintainer`, `License`,
`Url` as `urls.homepage`; without one, `Contents.m`'s `% Version x.y.z`
line or an installed MATLAB toolbox's version. `COPYING`, `LICENSE`,
`LICENCE`, `NOTICE` (and `.md`, `.txt`) become `meta.notices`
(component, text, format). Providers: the classes in its namespaces
carrying a `Provider` constant, in name order. Built once per folder and
session. `guatiao.Library(id, version, providers, 'Meta', m)` is the
primitive (providers: class names, or `guatiao.provider` specs with
`Class` set). A library has `Id`, `Version`, `Providers`, `ProviderIds`,
`Meta`, `entry()`, `live_instances()`, `close()` (lets go of each
provider's one instance; a host asking for the descriptor afterwards is
declined). Its descriptor is built on first use and kept for the
session, its `unload` answering `Busy` while an instance lives.
`Registry.register_library(lib)` files it under its id: linked, not
mapped, so `unload` answers `WrongKind` and `retire` takes it out.

A host in this session, written in any language, calls an M provider
through the same table it calls one written in Rust or C. A library file
written in M that a foreign process's scan loads would need MATLAB or
Octave embedded in it; the package offers none.

## Threads

M code runs only on the interpreter's thread, and so does everything in
this package.

**A slot an M object serves runs only on the interpreter's thread, while a
call into guatiao is in progress there**: a host's own call (a remote's
method, a registry call) or a library calling back during one, or code run
by `guatiao.serving`. The method runs under `mexCallMATLABWithTrap`, so an
error never unwinds through the C frames between the library and the
interpreter. Called from any other thread, or with no call into guatiao in
progress, it touches no interpreter object and answers at once:
`Unsupported`, with "M code runs only on the interpreter's thread, inside
a call into guatiao" in the `guatiao_provider_error` when the slot has one;
a provider's `available` answers false with that reason; `create` answers
`Unsupported`; a `destroy` is queued and done at the gateway's next call;
a library's `unload` and entry answer from C alone. Nothing waits, so
nothing deadlocks.

An interrupt (Ctrl-C) or running out of memory while an M provider serves
a call reaches the interpreter's own error handling, not the trap: avoid
interrupting a host while a foreign caller is calling M.

## Octave and MATLAB

- Octave 11 warns "method delete: conflicting definitions" when an object
  of a class with two handle superclasses (a provider serving two kinds) is
  deleted; a `delete` method in that class silences it.
- Octave's `mexCallMATLABWithTrap` drops an error's message; the package
  catches errors itself first, so a provider's words cross in both.
- Typed property declarations (`x char = ''`) are a syntax error in
  Octave: configurations are `guatiao.Config` field lists.
