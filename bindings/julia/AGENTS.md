# Guatiao (Julia) — API header

Julia bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `Guatiao`, Julia 1.10 or later, 64-bit platforms. No dependency
outside the standard library (`Libdl`, `Dates`, `TOML`), no native
library of its own and nothing compiled: guatiao is loaded at run time
and called with `ccall`.

## Modules

| name | what |
| --- | --- |
| `Guatiao` | `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`, `absent`, `decode`, `schema`, `@config`, `lower`, `redact`, `use_library`, the errors; dates and spans (`Timestamp`, `Span`, `Length`, `fromtext`, `totext`, `normalize`, `readlength`) |
| `Guatiao.Serde` | `parse`, `emit`: JSON, TOML, YAML |
| `Guatiao.Intake` | forms, paths, the flat projection |

Exported: `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`,
`absent`, `decode`, `schema`, `redact`, `GuatiaoError`, `ParseError`,
`Unparsed`, `LibraryNotFound`, `MissingSymbol`, `FloorMismatch`,
`Timestamp`, `Span`, `Length`, `Serde`, `Intake`. The rest is reached as
`Guatiao.name`.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `Guatiao.use_library(path)`, called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded is an error; naming the same one is
   not.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   `guatiao`; `guatiao_serde` and `guatiao_intake` are looked for beside
   it under their own names).
3. `Libdl.find_library`, by base name.

Not found is a `LibraryNotFound` (`basename`, `looked`). An export the
loaded build lacks is a `MissingSymbol` (`path`, `symbol`, `feature`)
when it is used. A library stays loaded for the process.
`Guatiao.exports(basename, symbol)` asks whether the loaded build has
one. The library must be built for the Julia process's architecture.

## Values

`Value` owns a root node; `close(v)` frees it (a second `close` does
nothing), `isopen(v)`. A Value dropped unclosed is freed by its
finalizer once nothing reaches it, a view into it included.
`Value(x) do v ... end` closes after the block. A `ValueRef` addresses a
node by a path of keys and positions, walked again on every access,
never a cached address: a write beside a node can move the array it sits
in. `MapRef <: AbstractDict{String,ValueRef}` and
`ListRef <: AbstractVector{ValueRef}` are views onto a map and a list.

```julia
v = Value(Dict("host" => "10.0.0.1", "port" => 5900, "tags" => ["a", "b"]))
v["port"] = 5901
push!(v["tags"], "c")
v["port"][]                       # 5901: `n[]` is the node's plain Julia data
m = MapRef(v); keys(m)            # ["host", "port", "tags"]
decode(Vector{String}, v["tags"])
close(v)
```

**Building**: `Value(x = absent; alloc)` converts Julia data (below);
`Value(nothing)` is null, `Value()` absent. `alloc` is a
`Ptr{Guatiao.GAlloc}` the caller keeps alive while every tree built
through it lives; the default is the library's own. A failed conversion
frees what it built. `Value(root::Ptr{GValue})` takes over a root node a
C function answered (C `malloc` memory).

**Reading** (any `Value`, `ValueRef`, `MapRef`, `ListRef`): `n[]` (plain
data: `nothing`, `Bool`, `Int64` or `BigInt` for an integer, `Float64`
for a fraction or exponent, `String`, `Vector{UInt8}`, `Vector{Any}`,
`Dict{String,Any}`, `Guatiao.absent`); `n["key"]` and `n[i]` (from 1)
extend the path, a `KeyError` / `BoundsError` when nothing is there and a
`GuatiaoError` `WrongKind` for another kind; `Guatiao.tag(n)` (`:absent`,
`:null`, `:bool`, `:number`, `:string`, `:bytes`, `:list`, `:map`),
`Guatiao.isabsent`, `Guatiao.isnull`, `Guatiao.path(n)` (the intake
grammar, positions from 0); `decode(T, n)`; `copy(n)` (a deep copy, a
`Value`); `MapRef(n)`, `ListRef(n)` (`WrongKind` for another kind).

**`MapRef`**: `length`, `haskey`, `m[k]`, `get(m, k, default)` (a
`ValueRef` or `default`), `keys` (insertion order; keys compare as UTF-8
bytes), `values`, iteration as `k => ValueRef`, `m[k] = x` (replaces in
place), `delete!` (nothing to delete is no error), `empty!`,
`merge!(m, src)` (copies `src`'s entries, replacing collisions). The
same writes work on a `Value` or `ValueRef` holding a map.

**`ListRef`**: `length`, `l[i]`, iteration, `push!(l, xs...)`,
`deleteat!(l, i)`, `empty!`, and `l[i] = x` and `insert!(l, i, x)`, which
are O(n): the C surface only appends and removes, so they copy every
element, clear and push back. `push!`, `insert!` and `deleteat!` work on
a `Value` or `ValueRef` holding a list.

A closed Value, or a view whose source is closed or whose call has
returned, throws `InvalidStateException`. A borrowed view throws
`ArgumentError` on every write.

## Julia data and trees

`Value(x)`, `m[k] = x`, `push!` and every kind call write Julia data:

| Julia | value |
| --- | --- |
| `nothing` | null (a field of a `Union{Nothing,T}` type holding `nothing` is left out) |
| `Guatiao.absent` | absent |
| `Bool` | boolean |
| `Integer`, `AbstractFloat`, `NumberText` | number, its exact text (a float's shortest; `-0.0` kept; NaN and infinities refused) |
| `AbstractString`, `Symbol`, `AbstractChar` | string (UTF-8 or refused) |
| `Vector{UInt8}`, `codeunits(s)` | bytes |
| `AbstractVector`, `Tuple`, `AbstractSet` | list |
| `AbstractDict` (text, `Symbol`, integer, `Bool` or `Char` keys), `NamedTuple` | map, in the collection's iteration order |
| a struct | map of its fields, in order; a struct with no fields is null |
| an `@enum` value | string, its name |
| `Dates.Date`, `Dates.Time`, `Dates.DateTime`, `Timestamp`, `Span` | their text forms (below) |
| `Length`, a `Dates.FixedPeriod` | an exact length `[seconds, nanoseconds]` |
| `Dates.CompoundPeriod`, `Year`, `Month` | a span |
| `Value`, a view | a deep copy |
| anything else | `Guatiao.lower(x)`, when a method answers other data; else `ArgumentError` |

Define `Guatiao.lower(x::MyType)` to write a type as other data.

**`decode(T, n)`** reads back: `Any` (plain data), `Bool`, every integer
type (exactly: a fraction, an exponent or a value out of range is
refused, never rounded), floats (nearest), `NumberText`, `String`,
`Symbol`, `Char`, `Vector{UInt8}`, vectors, tuples (exact length), sets
(a repeated element refused), `Dict`s (keys as above; two keys reading as
one refused), `NamedTuple`s, `Union{Nothing,T}` (null or absent is
`nothing`), enums by name, the dates and times, `Length` (any of its
three spellings) and fixed periods (whole units), `Value` (a copy), and
concrete structs field by field: a missing key is `nothing` for a field
whose type allows it, else the keyword constructor's default for a
`@kwdef` struct, else refused. Keys a struct does not declare are
ignored. Both directions stop past 128 nested containers. A refusal is a
`GuatiaoError` `BadValue` naming the path (`at agent[1].port: want an
integer, got string`).

**`Guatiao.schema(T) -> Value`**: the JSON Schema (2020-12) of what `T`
writes: a struct is an object, properties in field order, keys it does
not declare refused, a field required unless its type allows `nothing`
(the schema then also allows null) or `@config` gave it a default;
integer types state their bounds; dates, times, spans and lengths their
forms; `Any` and `Value` are `{}`. `Guatiao.keywords(::Type{T})`
answering a `NamedTuple` of per-field keywords
(`(host = (minLength = 1,),)`) adds them; `Guatiao.schema_of(::Type{T})`
answering plain data (a `NamedTuple` keeps key order) states a type's
own. An unknown type is an `ArgumentError`.

**`Guatiao.@config struct ... end`** is `Base.@kwdef` that also records
each field's default and docstring: a defaulted field is optional and
its schema states the default; a docstring is its `description`.

## Dates, times, instants, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>, and every text is read
and written by the library's own functions.

```julia
using Dates
Guatiao.fromtext(Date, "2026-10-21")
Guatiao.fromtext(Time, "13:45:00.5")                        # to the nanosecond
Guatiao.fromtext(DateTime, "2026-10-21T13:45:00.5")         # a civil date and time
ts = Guatiao.fromtext(Timestamp, "2026-10-21T19:15:00+05:30")  # an instant, UTC
Guatiao.totext(ts)                                          # "2026-10-21T13:45:00Z"
Guatiao.normalize(:span, "1h 30m")                          # "PT1H30M"
Date(2026, 1, 31) + Span(months = 1)                        # 2026-02-28
Guatiao.readlength(value)                                   # Length: [s, ns], seconds, {secs, nanos}
```

- `fromtext(T, text)` for `Date`, `Time`, `DateTime`, `Timestamp`,
  `Span`; `totext(x)` the canonical text; `normalize(form, text)` for
  `:date`, `:time`, `:datetime`, `:timestamp`, `:span`. Text the grammar
  refuses is an `Unparsed` (`form`, `at` the byte offset in the UTF-8
  text, `expected`), which never quotes the text.
- `Dates.DateTime` counts milliseconds: a civil date and time read into
  one drops what is below a millisecond, toward the past; writing one
  back is exact. `Dates.Time` keeps nanoseconds. Years 0000 to 9999.
- **`Timestamp(seconds, nanoseconds)`**: an instant, read in UTC (the
  offset it was written with is not kept); `Timestamp(dt::DateTime)`
  takes `dt` as UTC, `DateTime(ts)` answers UTC.
- **`Span(; years, months, weeks, days, hours, minutes, seconds,
  nanoseconds)`**: calendar units, one sign for the whole, checked by
  the library (`BadValue` for mixed signs or nanoseconds of a second or
  more); `==` unit by unit (`P1D` is not `PT24H`); `string`, `-`,
  `iszero`, `Guatiao.isnegative`; `d + s` for a `Date` (time units
  refused) or `DateTime`; `Span(::Dates.CompoundPeriod)` and
  `Dates.CompoundPeriod(s)`.
- **`Length(seconds, nanoseconds)`**: an exact length, never negative,
  to 18446744073709551615 seconds; `Length(::Dates.FixedPeriod)`,
  `Dates.Nanosecond(l)` (`InexactError` past about 292 years).

## Secrets

```julia
shown = redact(schema, value; placeholder = "[redacted]")
```

Every field a schema marks (`x-sensitive` or `writeOnly`) is replaced at
any depth, and whatever the schema does not describe is hidden too.
Local `$ref`s are followed. `schema` and `value` are Values, views or
plain data; the answer is a new `Value`. `WrongKind` for a schema that is
not a map.

## Serde: JSON, TOML, YAML

```julia
v = Serde.parse("""{"port": 5900}""")          # format = :json
text = Serde.emit(v; format = :yaml)
```

`Serde.parse(text; format = :json, how = 0, alloc) -> Value`;
`Serde.emit(x; format = :json, pretty = false, how = 0) -> String` takes a
Value, a view or plain data. `how`: `Serde.BYTES_DATA_URI`,
`BYTES_BASE64`, `BYTES_ARRAY`, `BYTES_REFUSE`, or'd with
`Serde.READ_DATA_URIS`. A document that does not parse is a `ParseError`
(`format`, `line` and `column` from 1, the column in characters, 0 when
the parser did not say; `message`, which never quotes the document). A
TOML document must be a map (`WrongKind`). TOML and YAML need a
`guatiao_serde` built with them (`MissingSymbol` naming the feature). A
number keeps its exact text.

## Intake

```julia
Intake.check(schema, form)                  # nothing, or the error map
Intake.layout(schema, form)                 # [Dict("section" => ..., "fields" => [...]), ...]
Intake.isvisible(schema, form, "key", values)
Intake.at(value, "agent[1].name")           # a borrowed ValueRef, or nothing
Intake.for_schema(schema)                   # a Value
Intake.form_for(form, schema, "path")       # a Value, or nothing
Intake.resolve(schema, "auth.password")     # a borrowed ValueRef, or nothing
Intake.flat_keys(schema, "auth")            # Vector{String}; WrongKind for an unknown key
Intake.flatten(schema, "auth", value)       # Dict{String,String}
Intake.unflatten(schema, "auth", store)     # a Value, or nothing
```

Every schema, form, value and values argument is a Value, a view or
plain data. `at` and `resolve` answer a view borrowed from their
argument: read-only, found again on every access,
`InvalidStateException` once the source is closed; `copy` it to keep it.
`unflatten` answers `nothing` for "nothing stored" or a key that does not
apply; a store that is not text to text is an `ArgumentError`.

## Errors

A failing `guatiao_status` is a `GuatiaoError` (`status::Guatiao.Status`,
`message`: the other side's own words, empty when it gave none; `code`,
the raw number). `Guatiao.Status` is an enum: `OK`, `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy`, `Internal`, reached as `Guatiao.BadValue` and so
on. `GuatiaoError(status, message)` builds one. `ParseError` and
`Unparsed` have `Guatiao.status(e) == BadValue`; all three are
`Guatiao.GuatiaoException`s. `LibraryNotFound`, `MissingSymbol` and
`FloorMismatch` are this binding's own.

## Threads

A Value is not safe to change from several tasks at once; reading one
from several at once is.
