# guatiao (R) — API header

R bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `guatiao`, R 4.2 or later, 64-bit platforms. A source package:
its C glue is compiled when it installs (Rtools on Windows) and needs
libffi (`libffi-dev` / `libffi-devel` on Linux, the SDK's on macOS,
Rtools' own on Windows). Imports only `methods` and `utils`. It carries no
guatiao library: one is opened at run time.

## Exports

| area | names |
| --- | --- |
| library | `use_library`, `has_export` |
| values | `value`, `absent`, `number`, `is_open`, `tag`, `is_absent`, `is_null`, `value_data`, `node_path`, `push`, `insert`, `delete`, `clear`, `merge_from`, `lower_value` |
| typed | `schema_of`, `config_class`, `field`, `from_value`, `validate`, `redact` |
| time | `time_parse`, `time_format`, `time_normalize`, `read_length`, `add_span`, `civil_time`, `civil_datetime`, `instant`, `span`, `exact_length` |
| serde, intake | `serde_parse`, `serde_emit`, `intake_check`, `intake_layout`, `intake_is_visible`, `intake_at`, `intake_for_schema`, `intake_form_for`, `intake_resolve`, `intake_flat_keys`, `intake_flatten`, `intake_unflatten` |
| registry | `registry` (its methods with `$`) |
| kinds | `kind`, `method`, `lent`, `taken`, `bind_table`, `has_slot`, `call_slot`, `byte_buffer`, `live_objects`, the S4 class `guatiao_remote` |
| providers | `provider`, `guatiao_library`, `library_of`, `live_instances` |
| errors | `guatiao_error`, `statuses` |

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `use_library(path)`, called before that first call: a file exporting
   the ABI, whatever it is called, or a directory. Naming another file
   once one is loaded is an error; naming the same one is not.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   `guatiao`; `guatiao_serde` and `guatiao_intake` are looked for beside
   it under their own names).
3. The platform's own search, by file name.

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
"text", "double"))` is the node's plain R data; `as.vector`, `as.list`,
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
every number as a `number`, `"double"` as the nearest double. Both
directions stop past 128 nested containers.

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
  never negative; written as `[seconds, nanoseconds]`. `read_length(x)`
  reads any of its three spellings.
- Units and seconds past 2^53 are refused: an R number holds whole
  numbers exactly only that far.

## Secrets

`redact(schema, value, placeholder = "[redacted]", alloc = NULL)`
answers a new value with every field the schema marks (`x-sensitive` or
`writeOnly`) replaced at any depth, and whatever the schema does not
describe hidden too. Local `$ref`s are followed. `WrongKind` for a
schema that is not a map.

## Serde: JSON, TOML, YAML

`serde_parse(text, format = "json", read_data_uris = FALSE, alloc =
NULL)` answers a value; `serde_emit(x, format = "json", pretty = FALSE,
bytes = c("data_uri", "base64", "array", "refuse"))` a string, from a
value, a view or R data. A document that does not parse signals
`guatiao_parse_error` (`format`, `line` and `column` from 1, the column in
characters, 0 when the parser did not say; `reason`, which never quotes
the document). TOML writes only a map, and no null. TOML and YAML need a
`guatiao_serde` built with them. A number keeps its exact text.

## Intake

```r
intake_check(schema, form)                  # NULL, or the error as a list
intake_layout(schema, form)                 # list(list(section, fields), ...)
intake_is_visible(schema, form, "key", values)
intake_at(value, "agent[1].name")           # a view, or NULL
intake_for_schema(schema)                   # a value
intake_form_for(form, schema, "path")       # a value, or NULL
intake_resolve(schema, "auth.password")     # a view, or NULL
intake_flat_keys(schema, "auth")            # character vector
intake_flatten(schema, "auth", value)       # named character vector
intake_unflatten(schema, "auth", store)     # a value, or NULL
```

Every schema, form, value and values argument is a value, a view or R
data. `intake_at` and `intake_resolve` answer a view of the argument's
own node when it is a value or view (found again on every access), and a
copy when it is R data. `intake_unflatten` answers `NULL` for "nothing
stored" or a key that does not apply; its store is a named character
vector or a named list of strings.

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
default; `%name`, `%library`, `%version`), `set_priority(id, p)` /
`priority(id)`, `provider_table(key, kind = NULL)` (`list(table, size,
ctx)`, `table` NULL for none; no `kind` for a single table),
`provider_config(key)` (a copy of the schema, a value, or NULL),
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
Greeter <- kind("greeter", floor_hash = 2695856491,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"),
  start = method(listener = taken(Listener), returns = Conversation, fallible = TRUE),
  greet_into = method(name = "string", listener = lent(Listener), fallible = TRUE),
  greet_from = method(config = taken("map"), returns = "map", fallible = TRUE),
  check = method(config = lent("map"), returns = "bool", fallible = TRUE))
```

`kind(name, floor_hash, ..., object = FALSE, class, where =
topenv(parent.frame()))` defines in `where` (a package's namespace as it
installs, or the global environment): `setClass(class,
representation("VIRTUAL"))` (the C name in CamelCase unless `class`
says it); a generic per slot, `function(self, <the slot's arguments>)`,
dispatching on `self` (a name two kinds of one package share must take
the same arguments; a name such as `start` then masks another package's
when attached); and `<class>Remote`, extending the kind's class and
`guatiao_remote`, with a method of each generic that calls the slot
(`call_slot(self, i, args)`). It answers the kind's description, which
`bind` takes. `method(..., returns = NULL, required = FALSE, fallible =
FALSE)` lists a slot's arguments, named, in order; `required` marks the
kind's original slots (they come first), `fallible` a slot ending in
`guatiao_provider_error *err`. An object kind's `destroy` is `close()`
on its remote.

| shape | C | the caller passes | the provider receives / returns |
| --- | --- | --- | --- |
| `"string"` | `guatiao_str` | a string | a string (UTF-8) |
| `"bytes"` | `guatiao_bytes` | a raw vector | a raw vector (a copy) |
| `lent("bytes")` | `guatiao_bytes_mut` | a `byte_buffer(n)`, written in place | a buffer (`length`, `[`, `[<-`, `as.raw`), copied back when the method returns |
| `"bool"`, `"int8"`..`"uint64"`, `"float"`, `"double"`, `"size"`, `"intptr"` | the C scalar | a number or logical that fits | a number (an integer for 32 bits and less) or logical |
| `"value"`, `"map"`, `"list"` | `const guatiao_value *` / `map *` / `list *` | a value, a view or R data | a read-only view, valid for the call |
| `lent("value")`, `lent("map")`, `lent("list")` | the same without `const`, "Lent for the call" | a value or a view, changed in place | a writable view, valid for the call; what is added is built in the allocator the tree records |
| `taken("value")`, `taken("map")`, `taken("list")` | the same, handed over | a value (moved: it holds absent afterwards), a view (copied) or R data | a value the method owns (and closes, or the collector frees) |
| answer `"value"`, `"map"`, `"list"` | `guatiao_value/map/list *out` | a value | any R data or a value, built in the allocator the carrier names (`list()` is an empty map too) |
| answer `"string"` | `guatiao_string *out` | a string | a string |
| answer scalar | `T *out` | a number or logical | a number or logical |
| answer `NULL` | no answer | `NULL` | ignored |
| `taken(K)` (object kind) | `guatiao_object` | an S4 or Reference Class object with methods of `K`'s generics, or a remote this side owns (handed over) | `KRemote`, owned: `close()` destroys it |
| `lent(K)` | `const guatiao_object *` | the same, or any remote of `K` | `KRemote`, valid for the call |
| answer `K` | `guatiao_object *out` | `KRemote`, owned | an object implementing `K`, or a remote it owns |

Integers past 2^53 are not held exactly by an R number.

**A remote** (`GreeterRemote`) has a method of each of the kind's
generics; `has_slot(r, "shout")` says whether its table carries a slot
(a slot appended after the library was built is missing from an older
table, and calling it is `Unsupported`); a refusal is a `guatiao_error`
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
`guatiao_error`s with status `BadValue`. `guatiao_closed`,
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
