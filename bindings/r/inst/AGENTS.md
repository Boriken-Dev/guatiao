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
package.
