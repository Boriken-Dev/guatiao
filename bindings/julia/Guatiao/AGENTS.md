# Guatiao (Julia) — API header

Julia bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships in the package, so it names no
repository path.

Package `Guatiao`, Julia 1.10 or later, 64-bit platforms (no supported
Julia runs on 32-bit ARM). No dependency
outside the standard library (`Libdl`, `Dates`, `TOML`), no native
library of its own and nothing compiled: guatiao is loaded at run time
and called with `ccall`.

## Modules

| name | what |
| --- | --- |
| `Guatiao` | `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`, `absent`, `decode`, `schema`, `@config`, `lower`, `redact`, `from_wire`, `to_wire`, `use_library`, the errors; dates and spans (`Timestamp`, `Span`, `Length`, `fromtext`, `totext`, `normalize`, `readlength`); `Registry`, `Instance`; `@kind`, `Lent`, `Taken`, `bind`; `@provider`, `Provider`, `Library`, `library_of`, `register_library` |
| `Guatiao.Wire` | channels: `announce`, `send`, `close` (a frame), `Receiver`, `accept`, `Event`, `Refused` |
| `Guatiao.FFI` | for a package over another guatiao library: `@cdef`, `surface!`, `find_library`, trees as root nodes |

JSON, TOML and YAML are the package `GuatiaoSerde`, forms
`GuatiaoIntake`: each over its own library, built on `Guatiao.FFI`.

Exported: `Value`, `ValueRef`, `MapRef`, `ListRef`, `NumberText`,
`absent`, `decode`, `schema`, `redact`, `from_wire`, `to_wire`, `Wire`, `GuatiaoError`,
`Unparsed`, `LibraryNotFound`, `MissingSymbol`, `FloorMismatch`,
`Timestamp`, `Span`, `Length`, `Registry`,
`Instance`, `@kind`, `@provider`, `@required`, `@fallible`, `Lent`,
`Taken`. The rest is reached as `Guatiao.name`.

## Finding the native library

Nothing native is touched until the first call that needs it; the
library is then found in this order:

1. `Guatiao.use_library(path)`, called before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is loaded is an error; naming the same one is
   not.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file, taken as it is.
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

## Values on the wire, and channels

`to_wire(n) -> Vector{UInt8}` (a `Value` or a view) and
`from_wire(bytes; alloc) -> Value` are the wire encoding: a tag byte per
node, LEB128 lengths, a number as its own text, bytes as bytes, a map in
its order, the same bytes in every language. `from_wire` throws a
`GuatiaoError` with status `BadValue` for bytes that are not exactly one
value.

A channel announces a schema before its values; a receiver checks every
value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```julia
frames = [Wire.announce(1, schema), Wire.send(1, Dict("greeting" => "hi")), Wire.close(1)]
r = Wire.Receiver()
for frame in frames
    event = Wire.accept(r, frame)     # Wire.Event(kind, channel, value)
    event.value === nothing || close(event.value)
end
close(r)
```

- `Wire.announce(channel, schema)`, `Wire.send(channel, value)` (a
  `Value`, a view or plain data), `Wire.close(channel)`, each a
  `Vector{UInt8}`; `channel` is any `Integer` from 0 to
  `typemax(UInt64)`. `Wire.close` writes a frame and is not
  `Base.close`, which closes a receiver.
- `Wire.Receiver()`; `Wire.accept(r, frame) -> Wire.Event`; `close(r)`,
  `isopen(r)` (a finalizer is the backstop; calls on one receiver are
  serialised by its lock). `Wire.Event`: `kind` (`:schema`, `:value`,
  `:close`), `channel::UInt64`, `value`: a `Value` the caller closes, or
  `nothing` for a close.
- A refusal throws a `Wire.Refused <: GuatiaoException`: `refusal`
  (`:malformed`, `:no_schema`, `:not_a_schema`, `:invalid`), `status`
  (`Guatiao.status(e)`: `BadValue`, `NotFound`, `WrongKind`, `BadValue`),
  `message`, `channel`, `kind` (the frame's byte), `at` (a malformed
  frame's byte), and for `:invalid` `key`, `expected` or `known`. It
  changes nothing the receiver keeps and quotes nothing it was given.

## Registry

```julia
reg = Registry("my-host", "1.0")
Guatiao.scan_dir(reg, "/path/to/plugins"; rules = "kind=greeter")
Guatiao.providers(reg, "greeter")          # Vector of Dicts, best first
Guatiao.best(reg, "greeter")               # key, id, version, library, display_name, from, kinds, has_config, vtable_size
Guatiao.why_not(reg, "codec")              # Dict("available" => false, "why" => "nothing-claims-it", ...)
greeter = Guatiao.bind(reg, "acme_hello", Greeter)        # a remote (see Kinds)
inst = Guatiao.create(reg, "acme_shouter", Dict("prefix" => "hey"))
shouter = Guatiao.bind(inst, Greeter)
close(inst); close(reg)
```

`Registry(id, version; alloc)` (`alloc` names the allocator its answers
are built in and that it offers libraries); `close`, `isopen`,
`Registry(f, id, version)` closes after the block. Every answer that is
data is plain Julia data, already freed. `load_file(reg, path)`,
`register_entry(reg, name, entry::Ptr{Cvoid})` (a
`guatiao_library_entry`), `register_library(reg, lib)` (see Providers),
`scan_dir(reg, dir; rules, descending)` and `scan_path(reg, spec; rules,
descending)` (`rules`: one `[!]KEY=VALUE` per line, matched against what a
library declares before it is mapped), `libraries`, `providers(reg, kind
= "")`, `available(reg, kind = "")`, `why_not`, `provider(reg, key)`,
`best`, `provider_available(reg, key) -> (Bool, reason)`,
`keyed_by!(reg, template)` / `libraries_keyed_by!` (`%id` by default;
`%name`, `%library`, `%version`, `%source`), `set_priority!` / `priority`,
`provider_table(reg, key; kind) -> (table, size, ctx)` (`C_NULL` for
none; no `kind` for a single table), `provider_config(reg, key)` (a
borrowed, read-only view of the schema, or `nothing`),
`provider_meta(reg, key)` (what the provider declared beyond its schema,
its notices or its form under `GuatiaoIntake.PROVIDER_FORM_KEY`: a `Value`
the caller owns, outliving the library, or `nothing`; `NotFound` for an
unknown key), `kind_descriptions(reg, key)` (the document of the kinds the
provider serves and the object kinds they reach, a `Value` the caller owns,
or `nothing` when it carries none; what `kinds_of` loads), `host(reg)` (the
`guatiao_host_info *`). All reached as `Guatiao.name`.

`create(reg, key, config)` checks the configuration against the
provider's schema first (`BadValue` naming the field), then builds an
`Instance`; `close(inst)` reaches the provider that built it, even after
a retire or a refresh loaded a new build under the key. An unclosed
instance is closed by its finalizer.

`retire!(reg, key)` takes a library out and leaves it mapped until
nothing uses it; `collect!(reg)` closes every retired library whose own
`unload` agrees and answers their keys, as does the next load, retire or
unload. `unload!(reg, key; unchecked = false)` unmaps when the library
agrees: `Busy` while something it handed out lives, `Unsupported` with no
`unload` function, `WrongKind` for a library linked rather than mapped,
`NotFound` for none; `unchecked = true` insists. **Both take the LIBRARY
key** (`%id` by default). No table, ctx or borrowed schema taken from it
is used again.

**Sources**: `add_source_file!(reg, path, config = Dict())` maps a file
exporting `guatiao_source` (a bridge to another language) and adds the
source it answers, refusing with its own words; `watch_dir!(reg, dir;
rules)` watches a directory; `refresh!(reg)` makes the registry agree with
every source, answering `Dict("loaded" => [...], "retired" => [...],
"skipped" => [...], "refused" => [...])`. Nothing refreshes on its own.

Every registry call that can reach a library is made in a GC-safe region,
so a library calling back from a thread of its own never waits for this
one. A Registry is not safe for concurrent use: a host sharing one
between tasks guards it.

## Kinds: calling a table, serving one

A kind is declared once with `@kind`, from its C header: an abstract type,
a generic function per slot, and a remote type that calls the slots.

```julia
module GreeterKind
using Guatiao: @kind, Lent, Taken, MapRef

@kind Listener floor_hash = 1867921573 object = true begin
    @required heard(what::String)::Nothing
end

@kind Greeter floor_hash = 2032737757 begin
    @required @fallible greet(name::String)::MapRef
    shout(name::String)::String
    @fallible start(listener::Taken{Listener})::Conversation
    @fallible greet_into(name::String, listener::Lent{Listener})::Nothing
    @fallible greet_from(config::Taken{MapRef})::MapRef
    @fallible check(config::Lent{MapRef})::Bool
end
end
```

`@kind Name floor_hash = N [name = "c_name"] [object = true] begin ...
end` defines `abstract type Name <: Guatiao.Kind`, a generic function per
slot (new functions of the calling module: a name Base exports, such as
`read` or `count`, then shadows Base's there), `Guatiao.kind(Name)` (its
`KindSpec`), and `NameRemote <: Name`, whose method of each function
calls the slot. Slots are listed in the header's order and found by
position, so a function's Julia name is free; `floor_hash` is the
header's `<table>_FLOOR_HASH`; the C name is the type's in lower case
unless `name` says it. `@required` marks a slot every table fills,
wherever it is declared, `@fallible` a slot ending in
`guatiao_provider_error *err`. An object kind's `destroy` is `close` on
its remote. Object kinds a slot names are declared before it.

**From a description**: `Guatiao.kinds_of(document; into = Module(:Kinds),
known = ())` declares every kind of a description document in `into`, as
`@kind` would (object kinds first), and answers them by C name
(`Dict{String,Type}`): the `<crate>.kinds.json` a kind crate commits, as
its JSON text (read by the library, so no JSON package is needed), the
`Value` `kind_descriptions` answers, or a `Dict`. `known` are `@kind`
types a slot may name that the document lacks. A slot's function is
`getfield(into, :greet)`, reached through `Base.invokelatest` from code
compiled before the call. `Guatiao.kind_declarations(document)` answers the
`@kind` expressions instead. An unknown type, a borrowed list or a cycle
is an `ArgumentError` naming the kind and slot; text that is not a
description a `GuatiaoError` (`BadValue`) naming where.

| shape | C | the caller passes | the provider receives / returns |
| --- | --- | --- | --- |
| `String` | `guatiao_str` | any `AbstractString` | `String` (`BadValue` when not UTF-8) |
| `Vector{UInt8}` | `guatiao_bytes` | bytes | a copy |
| `Lent{Vector{UInt8}}` | `guatiao_bytes_mut` | a `Vector{UInt8}`, written in place | a copy, written back when the method returns |
| `Bool`, `Int8`..`UInt64`, `Int`, `UInt`, `Float32`, `Float64` | the C scalar (`UInt` is `size_t`, `Int` `intptr_t`) | a number that converts | the scalar |
| `ValueRef`, `MapRef` | `const guatiao_value *`, `const guatiao_map *` | a Value, a view or plain data (`nothing` is a null value pointer) | a read-only view, valid for the call |
| `Lent{ValueRef}`, `Lent{MapRef}`, `Lent{ListRef}` | the same without `const`, "Lent for the call" | a Value or a view, changed in place | a writable view, valid for the call; what is added is built in the allocator the tree records, and what a callee built elsewhere, or wrote over it, is moved, once the call returns, into the allocator the tree recorded when it was lent; a refused move is the call's error (the slot's own refusal first), and so is a key the callee wrote into a lent map that is not UTF-8, removed and freed (bad value), what it answered freed |
| `Taken{ValueRef}`, `Taken{MapRef}`, `Taken{ListRef}` | the same, handed over | a Value (moved: it holds absent afterwards), a view (copied) or plain data | a `Value` the method owns (and closes, or its finalizer frees it), moved into this binding's allocator on arrival (copied only where the caller built it elsewhere), so it may be kept past the caller's library; taken, and freed, even when another argument refuses the call |
| answer `ValueRef`, `MapRef`, `ListRef` | `guatiao_value/map/list *out` | a `Value`, moved into this binding's allocator on arrival when the slot built it in its own | any Julia data or a `Value`, built in the allocator the carrier names (a Value elsewhere is copied there and closed) |
| answer `String` | `guatiao_string *out` | a `String` | a `String` |
| answer scalar | `T *out` | the scalar | the scalar |
| answer `Nothing` | no answer | `nothing` | ignored |
| `Taken{K}` (object kind) | `guatiao_object` | any value implementing `K`, or a remote this side owns (handed over) | `KRemote`, owned: `close` destroys it |
| `Lent{K}` | `const guatiao_object *` | any value implementing `K`, or a remote | `KRemote`, valid for the call; `close` does nothing |
| answer `K` | `guatiao_object *out` | `KRemote`, owned | any value implementing `K`, or a remote it owns |

**A remote** (`GreeterRemote`) has a method of each of the kind's
functions; `Guatiao.has(r, :slot)` says whether its table fills a slot
(a provider may leave an optional one null, and calling it is
`Unsupported`: a Rust caller would run the
kind's default body, which Julia cannot); a refusal is a `GuatiaoError`
with the provider's status and words; a slot that cannot fail and fails
answers its status. `Guatiao.bind(K, table, size, ctx; keep)` makes one
over any table, a `FloorMismatch` when the table is smaller than the
kind's required slots or has another floor hash. A remote from
`bind(reg, ...)` is valid while the registry is open and the library
loaded (`InvalidStateException` once the registry is closed); one from
`bind(inst, K)` until the instance is closed. An object remote this side
owns and drops unclosed is destroyed by its finalizer.

**A Julia value as an object of a kind**: any value with methods of the
object kind's required functions (`GreeterKind.heard(e::Ear, what::String)`).
Its table is built once per kind and type (a slot for each function the
type has a method of, null for the rest) and kept for the process. What C
holds of it is a ctx shaped like an address (`0x10000 + 16 * id`, never
reused), so a call naming one released answers `Gone`. It is held until
its holder destroys it (handed over or answered), when `close` is called
on it if it has a method, or until the call returns (lent).
`Guatiao.live_objects()` counts what is held.

**A refusal**: a method throws; a `GuatiaoError` crosses as its status and
words, anything else as `Internal` with its text. A slot that cannot fail
reports the failure through `Guatiao.WARN[]` (a function of the message;
by default a line on `stderr`) and its status crosses alone. Nothing
thrown ever crosses into C.

## Providers written in Julia

```julia
module JlGreeter
using Guatiao, GreeterKind
import GreeterKind: greet, count

struct Hello <: Greeter
    asked::Threads.Atomic{Int}
end
Hello() = Hello(Threads.Atomic{Int}(0))
greet(::Hello, name::String) = isempty(name) ?
    throw(Guatiao.GuatiaoError(Guatiao.BadValue, "nobody to greet")) : Dict("greeting" => "hello, $name")
count(h::Hello) = Threads.atomic_add!(h.asked, 1) + 1
@provider Hello serves = (Greeter, Counter) display_name = "Hello, in Julia"

Guatiao.@config struct ShouterConfig
    "What every greeting starts with."
    prefix::String
end
struct Shouter <: Greeter
    prefix::String
end
Shouter(c::ShouterConfig) = Shouter(c.prefix)
greet(s::Shouter, name::String) = Dict("greeting" => "$(s.prefix) $(uppercase(name))")
@provider Shouter config = ShouterConfig
end

reg = Registry("host", "1.0")
Guatiao.register_library(reg, JlGreeter)   # the package is the library
```

**A provider** is a concrete struct with methods of its kinds' functions.
`@provider T key = value ...` declares it in the module where it is
written (in declaration order); the keys are `Guatiao.Provider(T; ...)`'s:
`serves` (default the kinds `T` subtypes; Julia has single inheritance,
so a struct serving several names them), `name` (default the type's name
in snake case), `id` (default `{last part of the library id}_{name}`),
`display_name`, `version` (empty: the library's), `meta`, `config` (a
type: an instance per configuration, `T(c::C)`, after the host checked it
against `Guatiao.schema(C)`), `config_schema` (plain data: `T(plain)`
with the configuration as plain data), `default` (also offer `T()`; the
default without a configuration), `available` (a function answering
`nothing` when the provider can run here and why not otherwise, asked
every time a host asks). With neither `config` nor `config_schema`, `T()`
is the provider's one instance, built when the library's descriptor is.
A required slot `T` has no method for, an object kind among `serves`, or
two providers sharing an id is refused when the library is made. An
instance (or a provider's one instance) with a `close` method is closed
when its host destroys it (or when the library is closed).

**A library**: `library_of(mod; version, meta)` is the package: id the
module's full name (`Pkg.Sub` for a submodule), version `pkgversion(mod)`
(its `Project.toml`), providers its `@provider` declarations in order.
**A module loaded from no package** (`Main`, a script, a test) is refused
unless `version` says one. Meta, lowest first: the package's
`Project.toml` (`authors`, `uuid`, and `description` as `summary`,
`license`, `homepage` and `repository` as `urls` when it has them), its
licence files as `notices` (`component`, `text`, `format`), the first
paragraph of the module's docstring as `description`, then `meta`. Built
once per module. `Guatiao.Library(id, version; providers, meta)` is the
same with both given (providers as types or `Provider` specs). A
`Library` has `id`, `version`, `providers`, `Guatiao.entry(lib)` (its
`guatiao_library_entry`), `Guatiao.live_instances(lib)` and `close(lib)`
(lets go of each provider's one instance; a host asking for the
descriptor afterwards is declined). Its descriptor is built on first use
and kept for the process, with its `unload` answering `Busy` while an
instance lives; a process builds at most 64 libraries and 256 providers
(the descriptor's own slots come from fixed pools: no C function pointer
is made from a closure). `register_library(reg, lib_or_module)` files it
under its id and keeps it: linked, not mapped, so `unload!` answers
`WrongKind` and `retire!` takes it out.

A host in this process, written in any language, calls a Julia provider
through the same table it calls one written in Rust or C. A library file
written in Julia that any host's scan loads would need Julia embedded in
it; the package offers none.

## FFI: for a package over another guatiao library

`Guatiao.FFI` is what GuatiaoSerde and GuatiaoIntake build on, and what
any package binding a C library that takes and answers guatiao values
may use. Supported API, versioned with this package; nothing in it is
exported (write `FFI.adopt`).

```julia
using Guatiao.FFI: FFI, @cdef, GValue, GAlloc, GStr

FFI.surface!("mylib"; variable = "MYLIB_LIBRARY", probe = "mylib_parse")
@cdef "mylib" "" :plain mylib_parse(text::GStr, a::Ptr{GAlloc}, out::Ptr{GValue})::UInt32

function parse(text::String)
    out = FFI.new_root()
    status = GC.@preserve text mylib_parse(FFI.gstr(text), FFI.binding_alloc(), out)
    status == 0 || (FFI.free_root(out); FFI.check(status))
    return FFI.adopt(out)                      # a Value; it frees the tree and `out`
end
```

- `@cdef base feature mode name(arg::CType, ...)::CRet`: an export of the
  library carrying surface `base`, found on first use (`feature` names the
  build feature it needs, `""` for every build; `mode` `:safe` calls it in
  a GC-safe region, `:plain` with a plain `ccall`). Usable from any
  module; its pointer is resolved again after a reset.
- The order: `Guatiao.use_library(base, path)` (a file, or a directory
  holding the platform's file name), the surface's own variable, then
  `GUATIAO_LIBRARY` (a directory; or a file, used when it exports the
  surface's probe, else the file beside it), then `Libdl.find_library`.
  `surface!(base; variable, probe)` declares a surface's variable and
  probe (`guatiao_serde` and `guatiao_intake` are known);
  `find_library(base)` is where the order lands now (`LibraryNotFound`
  naming every place tried).
- Trees cross as their root node, a `Ptr{GValue}`: `new_root()` /
  `free_root(p)` for a root a call writes; `adopt(root, alloc =
  binding_alloc())` a `Value` taking it over; `with_nodes(f, args...)`
  lends `Value`s, views or plain data to a call as nodes;
  `require_node(x)` / `node_ptr(x)` the node a `Value` or view names now,
  `owner_of(x)` what to keep under `GC.@preserve` while it is used;
  `borrow(source, find)` a read-only view of what `find()` answers, asked
  again on every access, refused once `source` is closed.
- Reading: `load_tag`, `payload_ptr`, `text_of`, `plain`, `map_keys`,
  `map_find`, `TAG_*`, `ZERO_VALUE`. Writing: `gstr(s)` (a `GStr` over a
  `String` kept under `GC.@preserve`), `binding_alloc()` (the core
  library's default allocator). `check(status, message = "")` throws the
  status's `GuatiaoError`; `guatiao_value_free(p)` frees what a node owns.
  The mirrors: `GValue`, `GAlloc`, `GStr`, `GString`, `GBytes`, `GVisitor`.
- A whole document with no tree: `read_plain(call) -> (data, status)`
  calls `call(visitor::Ptr{Cvoid})`, which hands that `guatiao_visitor` to
  a C walk (a `*_visit` export) and answers its status, and answers the
  plain data the walk's events make, as `plain` reads a tree (an
  exception inside them is rethrown after the walk);
  `julia_to_wire(x) -> Vector{UInt8}` writes what `Value(x)` builds as wire
  bytes, in Julia, with the same data taken and the same errors.

**A tree crosses libraries as its root node**: built by one library, it
is read and freed by any other, since a value records its own allocator.
The library that built it stays loaded while the tree lives.

## Errors

A failing `guatiao_status` is a `GuatiaoError` (`status::Guatiao.Status`,
`message`: the other side's own words, empty when it gave none; `code`,
the raw number). `Guatiao.Status` is an enum: `OK`, `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy`, `Internal`, reached as `Guatiao.BadValue` and so
on. `GuatiaoError(status, message)` builds one. `Unparsed` has
`Guatiao.status(e) == BadValue`, and `Wire.Refused` its own status; all three are `Guatiao.GuatiaoException`s. `LibraryNotFound`, `MissingSymbol` and
`FloorMismatch` are this binding's own.

## Threads

A Value is not safe to change from several tasks at once; reading one
from several at once is. A Registry is not safe for concurrent use; a
remote may be called from any task, as the provider behind it allows.

A slot this package serves (a provider, an object) runs on the thread
that calls it, a thread Julia did not start included (Julia adopts it):
nothing is queued, so a library's own threads may call a Julia provider
at once, and the provider guards what it shares as any Julia value shared
between tasks does. A served method must not wait on a task of the Julia
thread that is making the call: that thread is waiting in it.

**A host calling C that calls back from another thread** makes that call
inside a GC-safe region, as every call of this package does: a callback
that allocates may start a collection, which waits for every Julia thread
to reach a safepoint, and a thread blocked in a plain `ccall` never does.
Wrap such a `ccall` of your own the same way, its arguments converted and
rooted first (nothing may allocate inside the region):

```julia
state = ccall(:jl_gc_safe_enter, Int8, ())
status = ccall(f, UInt32, (Ptr{Cvoid},), arg)
ccall(:jl_gc_safe_leave, Cvoid, (Int8,), state)
```
