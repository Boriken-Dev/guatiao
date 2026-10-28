# guatiao (R) — API header

R bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `guatiao`, R 4.2 or later, 64-bit platforms and 32-bit ARM. A
source package: its C glue is compiled when it installs (Rtools on Windows) and needs
libffi (`libffi-dev` / `libffi-devel` on Linux, the SDK's on macOS,
Rtools' own on Windows). Imports only `methods` and `utils`; suggests
`bit64`, used only when installed and asked for. It carries no guatiao
library: one is opened at run time.

## Exports

| area | names |
| --- | --- |
| library | `use_library`, `has_export` |
| values | `value`, `absent`, `number`, `is_open`, `tag`, `is_absent`, `is_null`, `value_data`, `node_path`, `push`, `insert`, `delete`, `clear`, `merge_from`, `lower_value` |
| typed | `schema_of`, `config_class`, `field`, `from_value`, `validate`, `redact` |
| time | `time_parse`, `time_format`, `time_normalize`, `read_length`, `add_span`, `civil_time`, `civil_datetime`, `instant`, `span`, `exact_length` |
| registry | `registry` (its methods with `$`) |
| kinds | `kind`, `method`, `lent`, `taken`, `bind_table`, `has_slot`, `call_slot`, `byte_buffer`, `live_objects`, the S4 class `guatiao_remote` |
| providers | `provider`, `guatiao_library`, `library_of`, `live_instances` |
| errors | `guatiao_error`, `statuses` |
| wire | `from_wire`, `to_wire`, `wire_announce`, `wire_send`, `wire_close`, `wire_receiver` (its methods with `$`) |

JSON, TOML and YAML are the package `guatiao.serde`, forms
`guatiao.intake`: each over its own library, built on the C interface
below.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `use_library(path, library = "guatiao")`, called before that first
   call: a file exporting the library's functions, whatever it is called,
   or a directory. Naming another file once one is loaded is an error;
   naming the same one is not.
2. For `guatiao_serde` and `guatiao_intake` (the packages
   `guatiao.serde` and `guatiao.intake` call them),
   `GUATIAO_SERDE_LIBRARY` / `GUATIAO_INTAKE_LIBRARY`: a file, taken as
   it is, or a directory holding the library under its platform name.
3. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib` (or `guatiao_serde.dll` and so
   on), or a file: taken as it is for `guatiao`, and for the others when
   it exports their functions, else their library beside it.
4. The platform's own search, by file name.

Not found signals `guatiao_library_not_found` (`basename`, `looked`). An
export the loaded build lacks signals `guatiao_missing_symbol` (`path`,
`symbol`, `feature`) when it is used. `has_export(symbol, library =
"guatiao")` asks. A library stays loaded for the session, and so does
this package's own compiled code. The library must be built for the R
process's architecture (an x64 R on Windows ARM64 loads x64 DLLs).

## Values

`value(x = absent, alloc = NULL)` builds a `guatiao_value`, which owns a
root node; `close(v)` frees it (a second `close` does nothing),
`is_open(v)`. A value never closed is freed by the garbage collector.
`v$key`, `v[["key"]]` and `v[[i]]` (from 1; on a map, the i-th entry)
answer a view (`guatiao_node`) addressing a node by a path of keys and
positions, walked again on every access, never a cached address: a write
beside a node can move the array it sits in. A missing key answers
`NULL`; a position past the end, or a key on a list, signals. A view of a
closed value signals `guatiao_closed`; a view whose node is gone signals
`NotFound`.

```r
v <- value(list(host = "10.0.0.1", port = 5900L, tags = c("a", "b")))
v$port <- 5901L
push(v$tags, "c")
value_data(v$port)        # 5901L
names(v)                  # "host" "port" "tags"
close(v)
```

**Reading** (a value or a view): `value_data(x, numbers = c("auto",
"text", "double", "integer64"))` is the node's plain R data; `as.vector`, `as.list`,
`as.character` (a number's exact text), `as.numeric`, `as.integer`,
`as.logical`, `as.raw` go through it. `length` (a map's entries, a list's
elements, 1 for any other value, 0 for null and absent), `names` (a map's
keys, in insertion order), `tag(x)` (`"absent"`, `"null"`, `"bool"`,
`"number"`, `"string"`, `"bytes"`, `"list"`, `"map"`), `is_absent(x)`,
`is_null(x)`, `node_path(x)` (the intake grammar, positions from 0:
`"agent[1].name"`), `value(x)` (a deep copy, a new `guatiao_value`).

**Writing**: `v$key <- x` / `v[["key"]] <- x` set an entry in place (a
new key goes last); `v[[i]] <- x` replaces element `i` of a list, or
appends at `length + 1`; `push(x, ...)`, `insert(x, at, value)`,
`delete(x, at)` (a key or a position; nothing to delete is no error),
`clear(x)`, `merge_from(x, src)` (a map's entries, replacing collisions).
Each answers `x` invisibly. What is added is built through the allocator
the container records. A failed write leaves the tree as it was.

## R data and values

| R | value |
| --- | --- |
| `NULL`, any `NA` | null |
| `absent` | absent |
| logical, integer, double, string of length 1 | boolean, number, string |
| a double | the shortest text that reads back as it; a whole number below 1e21 as the integer; NaN and infinities refused |
| `number("1.10")` | a number, its exact text (JSON's grammar) |
| raw vector | bytes |
| unnamed list, atomic vector of another length, `I(x)` | list |
| named list, named atomic vector (named in full, no name twice) | map, in order |
| a value or view | a deep copy |
| bit64's `integer64` | a number, its exact text; `NA` null |
| factor | its labels |
| `Date`, `POSIXct`, `difftime`, the time classes below | their text forms; a `difftime` an exact length |
| an S4 object | a map of its slots |
| anything else | `lower_value(x)`, an S3 generic: define `lower_value.myclass` |

Reading back: null is `NULL`, absent `absent`, a boolean `TRUE`/`FALSE`,
a string UTF-8 text (a NUL inside signals), bytes a raw vector, a list an
unnamed list, a map a named list. A number, with `numbers = "auto"`: an
integer spelling that fits an R integer is an integer; otherwise a double
when the double holds it exactly (any fraction or exponent reads as the
nearest double); otherwise its exact text as a `number`. `"text"` answers
every number as a `number`, `"double"` as the nearest double,
`"integer64"` an integer spelling a bit64 `integer64` holds as one (not
`INT64_MIN`, its `NA`) and the rest as `"auto"` does. Both directions stop
past 128 nested containers.

**bit64.** `integer64` is the `bit64` package's class, which this package
suggests and never needs: an `integer64` is taken wherever a 64-bit
integer is (exactly; its `NA` is no number), and answered only where
`numbers = "integer64"` or `int64 = "integer64"` asks, so no answer's
class depends on what happens to be installed. Asking without `bit64`
installed is an error naming it.

## Typed reading and configurations

`schema_of(type)` answers the JSON Schema (2020-12) of an S4 class, as a
list: an object whose properties are its slots in order, keys it does
not declare refused, a slot required unless its prototype holds a value
(basic types) or `config_class` gave it a default. Slot types:
`character` (string), `integer` (integer, R's bounds), `numeric`
(number), `logical`, `raw` (bytes), `list` and `ANY` (anything), `Date`
(`format: date`), `POSIXct` (`format: date-time`), `difftime` (an exact
length), another S4 class (nested), a class union of a type with `NULL`
(may be null, optional).

`config_class(Class, name = field(type, default, doc, ...), ..., where)`
is `setClass` that also records each field's default (then optional,
stated in the schema), `doc` (its `description`) and further keywords
(`minLength = 1`); it answers the class generator.

`from_value(x, type)` reads a value or R data back as a type above: an
integer exactly (a fraction, an exponent or a value past an R integer
refused), a missing slot its default or prototype, `NULL` where a union
allows it, else refused; keys a class does not declare are ignored. A
refusal is a `guatiao_error` `BadValue` naming the path (`at agent.port:
want an integer, got string`).

`validate(schema, value)` answers `NULL` when the schema accepts the
value, else the refusal as a list (`key`, `message`, ...), which never
quotes the value.

## Dates, times, instants, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>, and every text is read
and written by the library's own functions.

```r
time_parse("date", "2026-10-21")                    # Date
time_parse("time", "13:45:00.5")                    # civil_time, to the nanosecond
time_parse("datetime", "2026-10-21T13:45:00.5")     # civil_datetime
ts <- time_parse("timestamp", "2026-10-21T19:15:00+05:30")   # instant, UTC
time_format(ts)                                     # "2026-10-21T13:45:00Z"
time_normalize("span", "1h 30m")                    # "PT1H30M"
add_span(as.Date("2026-01-31"), span(months = 1))   # 2026-02-28
read_length(list(90, 500000000))                    # exact_length
```

- `time_parse(form, text)` for `"date"`, `"time"`, `"datetime"`,
  `"timestamp"`, `"span"`; `time_format(x)`; `time_normalize(form,
  text)`. Text a grammar refuses signals `guatiao_unparsed` (`form`, `at`
  the byte offset in the UTF-8 text, `expected`), which never quotes it.
- A date is R's `Date`, years 0000 to 9999. `civil_time(hour, minute,
  second, nanosecond)` and `civil_datetime(date, time)` hold a time of day
  and a civil date and time, which R lacks. `instant(seconds,
  nanoseconds)` is an instant (or `instant(posixct)`), read in UTC;
  `as.POSIXct` converts. A `POSIXct` written as a value is rounded to the
  microsecond.
- `span(years, months, weeks, days, hours, minutes, seconds,
  nanoseconds)`: calendar units, one sign for the whole, checked by the
  library; `==` unit by unit (`P1D` is not `PT24H`), unary `-`.
  `add_span(x, span)` for a `Date` (time units refused) or a
  `civil_datetime`.
- `exact_length(seconds, nanoseconds)` (or `exact_length(difftime)`):
  never negative, seconds up to 2^64 - 1; written as `[seconds,
  nanoseconds]`. `read_length(x)` reads any of its three spellings.
- Past 2^53, where an R number stops holding whole numbers exactly: a
  span's units (within plus or minus 2^63 - 1) and an exact length's
  seconds are taken as a number, an `integer64`, a `number` or a string
  spelling an integer, read exactly. While every unit is within 2^53 the object holds
  numbers; past that it holds each unit's exact text (`unclass(x)` is a
  character vector), so equal spans are identical however they were made.
  `time_parse("span", ...)` and `read_length()` answer the same way;
  `as.list(x, int64 = "auto")` answers each unit as a 64-bit slot's answer
  is (see Kinds).

## Secrets

`redact(schema, value, placeholder = "[redacted]", alloc = NULL)`
answers a new value with every field the schema marks (`x-sensitive` or
`writeOnly`) replaced at any depth, and whatever the schema does not
describe hidden too. Local `$ref`s are followed. `WrongKind` for a
schema that is not a map.

## Values as bytes, and channels

`to_wire(x)` (a value, a view or R data) answers a raw vector in the wire
encoding: a tag byte per node, LEB128 lengths, a number as its own text,
bytes as bytes, a map in its order, the same bytes in every language.
`from_wire(bytes, alloc = NULL)` answers a `guatiao_value`; `BadValue`
for bytes that are not exactly one value.

A channel announces a schema before its values; a receiver checks every
value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```r
rcv <- wire_receiver()
for (frame in list(wire_announce(1, schema), wire_send(1, list(greeting = "hi")), wire_close(1))) {
  ev <- rcv$accept(frame)                 # list(kind, channel, value), class guatiao_wire_event
  if (!is.null(ev$value)) close(ev$value) # the schema's copy, or the checked value
}
rcv$close()
```

- `wire_announce(channel, schema)`, `wire_send(channel, value)`,
  `wire_close(channel)`: raw vectors. A channel is 0 to 2^64 - 1, as
  every 64-bit integer argument is read (an integer, a whole double, an
  `integer64`, a `number` or a string spelling one).
- `wire_receiver()`: a Reference Class object. `$accept(frame)` answers an
  event: `kind` `"schema"`, `"value"` or `"close"`, `channel` (an R
  integer when it fits, a double up to 2^53, else a `number`), `value` a
  `guatiao_value` to close, or `NULL`. `$close()` (idempotent; the garbage
  collector frees one never closed), `$is_open()`. It builds in the
  package's allocator; one receiver serves one conversation.
- A refusal signals `guatiao_wire_malformed` (`at`, the byte of the
  frame), `guatiao_wire_no_schema`, `guatiao_wire_not_a_schema` or
  `guatiao_wire_invalid` (`key` the field, `expected` or `known`), each
  also a `guatiao_wire_refused` and a `guatiao_error` with its status
  (`BadValue`, `NotFound`, `WrongKind`, `BadValue`), carrying `refusal`,
  `channel` and `kind`. It changes nothing the receiver keeps and quotes
  nothing it was given.
- Text holding a NUL decodes and re-encodes, but cannot be read into R
  (`BadValue` from `value_data`).

## The C interface, for a package over another guatiao library

The package registers a C table, `guatiao_r_api`, which `guatiao.serde`
and `guatiao.intake` build on and any package binding a C library that
takes and answers guatiao values may use. Its header is
`include/guatiao_r_api.h` in the installed package: list `LinkingTo:
guatiao` and import from `guatiao`, include a copy of `guatiao.h` before
it, and fetch the table in the package's init:

```c
const guatiao_r_api *api =
  ((const guatiao_r_api *(*)(void))R_GetCCallable("guatiao", "guatiao_r_api"))();
```

- `size` first: refuse a table whose `size` is not your
  `sizeof(guatiao_r_api)`, as `guatiao.serde` and `guatiao.intake` do.
- `export_of(surface, name)`: an export of the library carrying
  `"guatiao"`, `"guatiao_serde"` or `"guatiao_intake"`, opened on first
  use by the order above; NULL when the build lacks it. `missing(surface,
  name, feature)` signals `guatiao_missing_symbol`; `raise(status,
  message)` the status's `guatiao_error`. Neither returns.
- `binding_alloc`, this package's allocator; `alloc_arg(alloc)`;
  `str_arg(s, what)`, a one-string argument as UTF-8; `mkchar(text)`.
- `tree_arg(x, &tmp)` / `tree_args(...)`: the node a value, view or R data
  names (R data built into `tmp`; `free(&tmp)` after the call).
- `value_new(&root, alloc, keep)`: a value taking over a root a call
  wrote; `read_and_free(&v, 0)`: a tree as R data, freed;
  `take_text(&v)`: a string node's text, freed; `view_at(arg, root,
  found, &tmp)`: a view of what a lookup found inside a value or view
  argument, a copy for R data.
- `wire_of(x, &bytes, msg, n)`: R data as wire bytes, with no tree, in
  memory R frees when the `.Call` returns (a status; why in `msg`), for a
  `*_from_wire` export; `read_wire_and_free(&v, numbers)`: the wire bytes a
  `*_to_wire` export answered as a BYTES value, read as R data as
  `read_and_free` reads a tree (`numbers` 0 `"auto"`, 1 `"text"`, 2
  `"double"`, 3 `"integer64"`), the value freed either way.

A tree crosses libraries as its root node: built by one library, it is
read and freed by any other, since a value records its own allocator;
the library that built it stays loaded for the session.

## Registry

```r
reg <- registry("my-host", "1.0")
reg$scan_dir("/path/to/plugins", rules = "kind=greeter")
reg$providers("greeter")              # list of lists, best first
reg$best("greeter")                   # key, id, version, library, display_name, from, kinds, has_config, vtable_size
reg$why_not("codec")                  # list(available = FALSE, why = "nothing-claims-it", ...)
greeter <- reg$bind("acme_hello", Greeter)       # a remote (see Kinds)
inst <- reg$create("acme_shouter", list(prefix = "hey"))
shouter <- inst$bind(Greeter)
inst$close(); reg$close()
```

`registry(id, version, alloc = NULL)` answers a Reference Class object:
its methods are called with `$`. `alloc` names the allocator it offers
libraries. Every answer that is data is plain R data, already freed.
`reg$close()` (the libraries it loaded stay mapped), `reg$is_open()`,
`is_open(reg)`. Methods: `load_file(path)`, `register_entry(name,
entry)` (an external pointer to a `guatiao_library_entry`),
`register_library(lib)` (see Providers), `scan_dir(dir, rules = "",
descending = FALSE)` and `scan_path(spec, rules = "", descending =
FALSE)` (`rules`: one `[!]KEY=VALUE` per line, matched against what a
library declares before it is mapped), `libraries()`, `providers(kind =
"")`, `available(kind = "")`, `why_not(kind)`, `provider(key)`,
`best(kind)`, `provider_available(key)` (`list(available, reason)`),
`keyed_by(template)` / `libraries_keyed_by(template)` (`%id` by
default; `%name`, `%library`, `%version`, `%source`), `set_priority(id, p)` /
`priority(id)`, `set_import_search("path")` / `import_search()` (where
Windows finds what a file the registry opens imports: `"library"`, the
default, looks beside the file, beside the host's executable and in the
system directory, `"path"` also in the current directory and on `PATH`,
as an R host adding a Python bridge needs; unix ignores it),
`provider_table(key, kind = NULL)` (`list(table, size,
ctx)`, `table` NULL for none; no `kind` for a single table),
`provider_config(key)` (a copy of the schema, a value, or NULL),
`provider_meta(key)` (what the provider declared beyond its schema, its
notices or its form under `guatiao.intake`'s `intake_provider_form_key`: a
copy, outliving the library, or NULL; `NotFound` for an unknown key),
`kind_descriptions(key)` (the document of the kinds the provider serves
and the object kinds they reach, as R data `kinds_of()` loads, or NULL when
it carries none; `NotFound` for an unknown key, `BadValue` for one that is
not a description),
`host()` (the `guatiao_host_info *` as an external pointer).

`create(key, config)` checks the configuration against the provider's
schema first (`BadValue` naming the field; an empty `list()` is an empty
map), then answers an instance: `inst$close()` reaches the provider that
built it, even after a retire or a refresh loaded a new build under the
key; `inst$bind(kind)`, `inst$is_open()`. An instance never closed is
closed by the garbage collector.

`retire(key)` takes a library out and leaves it mapped until nothing
uses it; `collect()` closes every retired library whose own `unload`
agrees and answers their keys, as does the next load, retire or unload.
`unload(key, unchecked = FALSE)` unmaps when the library agrees: `Busy`
while something it handed out lives, `Unsupported` with no `unload`
function, `WrongKind` for a library linked rather than mapped, `NotFound`
for none. **Both take the LIBRARY key** (`%id` by default). No table,
ctx or remote taken from it is used again.

**Sources**: `add_source_file(path, config)` maps a file exporting
`guatiao_source` (a bridge to another language) and adds the source it
answers, refusing with its own words; `watch_dir(dir, rules = "")`
watches a directory; `refresh()` makes the registry agree with every
source, answering `list(loaded, retired, skipped, refused)`. Nothing
refreshes on its own.

## Kinds: calling a table, serving one

A kind is declared once with `kind()`, from its C header: a virtual S4
class, a generic per slot, and a remote class that calls the slots.

```r
Listener <- kind("listener", floor_hash = 1867921573, object = TRUE,
  heard = method(what = "string", required = TRUE))
Conversation <- kind("conversation", floor_hash = 2769624503, object = TRUE,
  say = method(what = "string", required = TRUE, fallible = TRUE),
  read = method(dst = lent("bytes"), returns = "int64", required = TRUE),
  turns = method(returns = "int64", required = TRUE))
Greeter <- kind("greeter", floor_hash = 2032737757,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"),
  start = method(listener = taken(Listener), returns = Conversation, fallible = TRUE),
  greet_into = method(name = "string", listener = lent(Listener), fallible = TRUE),
  greet_from = method(config = taken("map"), returns = "map", fallible = TRUE),
  check = method(config = lent("map"), returns = "bool", fallible = TRUE))
```

`kind(name, floor_hash, ..., object = FALSE, class, int64 = "auto",
where = topenv(parent.frame()))` defines in `where` (a package's namespace as it
installs, or the global environment): `setClass(class,
representation("VIRTUAL"))` (the C name in CamelCase unless `class`
says it); a generic per slot, `function(self, <the slot's arguments>)`,
dispatching on `self` (a name two kinds of one package share must take
the same arguments; a name such as `start` then masks another package's
when attached); and `<class>Remote`, extending the kind's class and
`guatiao_remote`, with a method of each generic that calls the slot
(`call_slot(self, i, args, int64 = NULL)`). It answers the kind's
description, which `bind` takes. `method(..., returns = NULL, required = FALSE, fallible =
FALSE)` lists a slot's arguments, named, in order; `required` marks a
slot every table fills, wherever it is, `fallible` a slot ending in
`guatiao_provider_error *err`. An object kind's `destroy` is `close()`
on its remote. Refused, as the kind attribute and the library's reader
refuse them: a slot named `header`, a `"list"` argument (a list is lent
or taken), `lent("bytes")` on a kind that is not an object kind, and an
object passed or answered on a slot that is not `fallible`.

`kinds_of(document, known = list(), class = .gt_camel, where = new.env())`
declares every kind of a description document as `kind()` would (each
object kind before the kinds naming it) and answers them in a list named
by C name: the `<crate>.kinds.json` a kind crate commits, as its JSON text
(read by the library; no JSON package needed), or the R data
`reg$kind_descriptions(key)` answers. `known` are kinds a slot may name
that the document lacks; `class` names each kind's S4 class from its C
name (give another when the same kinds are already declared, as class
names are global); a slot's generic is `get("greet", envir = where)`. An
unknown type, a borrowed list or a cycle is an error naming the kind and
slot; text that is not a description is `BadValue` naming where.

| shape | C | the caller passes | the provider receives / returns |
| --- | --- | --- | --- |
| `"string"` | `guatiao_str` | a string | a string (UTF-8) |
| `"bytes"` | `guatiao_bytes` | a raw vector | a raw vector (a copy) |
| `lent("bytes")` | `guatiao_bytes_mut` | a `byte_buffer(n)`, written in place | a buffer (`length`, `[`, `[<-`, `as.raw`), copied back when the method returns |
| `"bool"`, `"int8"`..`"uint32"`, `"float"`, `"double"` | the C scalar | a number or logical that fits | a number (an integer for 32 bits and less) or logical |
| `"int64"`, `"uint64"`, `"size"`, `"intptr"` | the C scalar | an R integer, a whole double, an `integer64`, a `number` or a string spelling an integer, read exactly | as `int64` says (below) |
| `"value"`, `"map"` | `const guatiao_value *` / `map *` | a value, a view or R data | a read-only view, valid for the call |
| `lent("value")`, `lent("map")`, `lent("list")` | the same without `const`, "Lent for the call" | a value or a view, changed in place | a writable view, valid for the call; what is added is built in the allocator the tree records, and what a callee built elsewhere, or wrote over it, is moved, once the call returns, into the allocator the tree recorded when it was lent; a refused move is the call's error (the slot's own refusal first), and so is a key the callee wrote into a lent map that is not UTF-8, removed and freed (bad value), what it answered freed |
| `taken("value")`, `taken("map")`, `taken("list")` | the same, handed over | a value (moved: it holds absent afterwards), a view (copied) or R data | a value the method owns (and closes, or the collector frees), moved into this binding's allocator on arrival (copied only where the caller built it elsewhere), so it may be kept past the caller's library; a null one refuses the call, and each is taken, and freed, even when another argument refuses it |
| answer `"value"`, `"map"`, `"list"` | `guatiao_value/map/list *out` | a value, moved into this binding's allocator on arrival when the slot built it in its own | any R data or a value, built in the allocator the carrier names (`list()` is an empty map too) |
| answer `"string"` | `guatiao_string *out` | a string | a string |
| answer scalar | `T *out` | a number or logical | a number or logical |
| answer `NULL` | no answer | `NULL` | ignored |
| `taken(K)` (object kind) | `guatiao_object` | an S4 or Reference Class object with methods of `K`'s generics, or a remote this side owns (handed over) | `KRemote`, owned: `close()` destroys it |
| `lent(K)` | `const guatiao_object *` | the same, or any remote of `K` | `KRemote`, valid for the call |
| answer `K` | `guatiao_object *out` | `KRemote`, owned | an object implementing `K`, or a remote it owns |

**64-bit integers.** An R number holds whole numbers exactly only to
2^53, and base R has no 64-bit integer. An integer shape takes an R
integer or logical, a whole double, a bit64 `integer64`, a `number`, or a
string spelling an integer (`"-9223372036854775808"`), read exactly and
range-checked; anything else is refused before the call, `BadValue`
naming the kind, the slot and the argument (`wide.echo_i64(x): out of
range for int64`). A 64-bit answer, and a 64-bit argument an R method is
handed, is what `int64` says: `"auto"` an R integer when it fits, a
double up to 2^53 (where a double holds every whole number), else a
`number` with its exact text; `"number"` always a `number`; `"double"`
always the nearest double; `"integer64"` an `integer64` when one holds it
(not `INT64_MIN`, its `NA`, nor an unsigned value past `INT64_MAX`), else
a `number`. `"auto"` never answers an `integer64`. `kind(..., int64)`
sets it for the kind's remotes and the R methods serving it,
`call_slot(..., int64)` for one call. An R method answers a 64-bit slot
in any spelling an argument takes; an answer out of range is refused
naming the slot (`wide.echo_u64 answered: out of range for uint64`).

**A remote** (`GreeterRemote`) has a method of each of the kind's
generics; `has_slot(r, "shout")` says whether its table fills a slot
(a provider may leave an optional one null, and calling it is
`Unsupported`); a refusal is a `guatiao_error`
with the provider's status and words. `bind_table(K, table, size, ctx,
keep = NULL)` makes one over any table; `reg$bind` refuses a table
smaller than the kind's required slots or with another floor hash
(`guatiao_floor_mismatch`). A remote from `reg$bind` is valid while the
registry is open and the library loaded (`guatiao_closed` once the
registry is closed); one from `inst$bind` until the instance is closed.
An object remote this side owns and never closes is destroyed by the
garbage collector. Arguments a call refuses before it reaches the table
(a wrong shape) leave everything as it was: nothing is handed over.

**An R object as an object of a kind**: an S4 or Reference Class object
whose class has methods of the object kind's generics. Its table is
built once per kind and class (a slot for each generic the class has a
method of, null for the rest) and kept for the session. What C holds of
it is a ctx shaped like an address (`0x10000 + 16 * id`, never reused),
so a call naming one released answers `Gone`. It is held until its
holder destroys it (handed over or answered), when `close()` is called
on it if its class has a method, or until the call returns (lent).
`live_objects()` counts what is held.

**A refusal**: a method signals; a `guatiao_error` crosses as its status
and words, anything else as `Internal` with its message. A slot that
cannot fail and fails prints the message on stderr and its status
crosses alone. No R error ever unwinds into C.

## Providers written in R

```r
Hello <- setRefClass("Hello", contains = c("Greeter", "Counter"), fields = list(asked = "numeric"),
  methods = list(initialize = function(...) { asked <<- 0; callSuper(...) }))
setMethod("greet", "Hello", function(self, name) {
  if (!nzchar(name)) stop(guatiao_error("BadValue", "nobody to greet"))
  list(greeting = paste0("hello, ", name))
})
setMethod("count", "Hello", function(self) { self$asked <- self$asked + 1; self$asked })
provider(Hello, display_name = "Hello, in R")

ShouterConfig <- config_class("ShouterConfig",
  prefix = field("character", doc = "What every greeting starts with.", minLength = 1))
Shouter <- setClass("Shouter", contains = "Greeter", slots = c(prefix = "character"))
setMethod("greet", "Shouter", function(self, name) list(greeting = paste(self@prefix, toupper(name))))
provider(Shouter, config = ShouterConfig, make = function(config) Shouter(prefix = config@prefix))

reg$register_library(library_of("mypackage"))   # the package is the library
```

**A provider** is an S4 or Reference class that contains the kinds it
serves, with methods of their generics (`setMethod` refuses a method
whose arguments are not the generic's). `provider(class, serves, name,
id, display_name, version, meta, config, config_schema, make, default,
available, where)` declares it where it is written (`where = NULL`
declares nothing, for `guatiao_library`): `serves` (default the kinds
the class contains), `name` (default the class's name in snake case),
`id` (default `{library id}_{name}`), `display_name`, `version` (empty:
the library's), `meta` (a named list), `config` (a configuration class:
an instance per configuration, `make(config)` after the host checked it
against `schema_of(config)`), `config_schema` (R data: `make(plain)` with
the configuration as plain data), `make` (default `new(class)`, or
`new(class, config = config)` for a configured provider), `default`
(also offer `make()`; the default without a configuration), `available`
(a function answering `NULL` when the provider can run here and why not
otherwise, asked every time a host asks). With neither `config` nor
`config_schema`, `make()` is the provider's one instance, built when the
library's descriptor is. A required slot with no method, an object kind
among `serves`, or two providers sharing an id is refused when the
library is made. An instance (or one instance) whose class has a
`close()` method is closed when its host destroys it (or when the
library is closed).

**A library**: `library_of(pkg, id = NULL, version = NULL, meta =
NULL)` is the package: id `Package`, version `Version`, providers its
`provider()` declarations in order. Meta, lowest first: `Title` as
`summary`, `Description` as `description`, `Authors@R` (or `Author`) as
`authors`, `Maintainer`, `License`, `URL` (the first as
`urls$homepage`) and `BugReports` (`urls$issues`), the installed
package's `LICENSE`, `LICENCE`, `COPYING` and `NOTICE` files (and their
`.md`/`.txt` forms) as `notices` (`component`, `text`, `format`), then
`meta`. An environment that is no package's namespace needs `id` and
`version`. Built once per package. `guatiao_library(id, version,
providers, meta = NULL)` is the same with both given (providers as
`provider()` specs or class names). A library has `$id`, `$version`,
`$providers`, `live_instances(lib)` and `close(lib)` (lets go of each
provider's one instance; a host asking for the descriptor afterwards is
declined). Its descriptor is built on first use and kept for the
session, with its `unload` answering `Busy` while an instance lives.
`reg$register_library(lib)` files it under its id and keeps it: linked,
not mapped, so `unload` answers `WrongKind` and `retire` takes it out.

A host in this session, written in any language, calls an R provider
through the same table it calls one written in Rust or C. A library
file written in R that any host's scan loads would need R embedded in
it; the package offers none.

## Errors

A failing status signals a `guatiao_error` condition (`status`, the name;
`code`, the number; `message`, the other side's own words, empty when it
gave none). `statuses` is the named table: `OK` 0, `BadValue` 7,
`AllocFailed` 8, `WrongKind` 9, `NotFound` 10, `NullArgument` 11, `Gone`
12, `Unsupported` 13, `Busy` 14, `Internal` 100. `guatiao_error(status,
message)` builds one. `guatiao_parse_error` and `guatiao_unparsed` are
`guatiao_error`s with status `BadValue` (the first is signalled by
`guatiao.serde`). The `guatiao_wire_*` refusals are `guatiao_error`s with
their status (see Values as bytes, and channels). `guatiao_closed`,
`guatiao_library_not_found` and `guatiao_missing_symbol` are this
package's own.

## Threads

R code runs only on R's main thread, and so does everything in this
package: a value, a registry and a remote are used from R's thread.

**A slot an R object serves runs on R's main thread only.** A host
calling it on that thread (an R host's own call, or a library calling
back during one) runs the method there. Called from any other thread —
a library's own worker, a thread pool — it touches no R object and
answers at once: `Unsupported`, with "R code runs only on R's main
thread" in the `guatiao_provider_error` when the slot has one; a
provider's `available` answers false with that reason; `create` answers
`Unsupported`; a `destroy` (of an object or an instance) is queued and
done on R's thread at its next call into the package; a library's
`unload` and entry answer from C alone. Nothing waits for R's thread, so
nothing can deadlock, and a library that calls an R provider from its
own threads gets refusals it must handle.

A method runs inside `R_ToplevelExec`: an error, an interrupt or a
condition nobody handles ends there, never unwinding through the C (or
Rust) frames between the library and R.
