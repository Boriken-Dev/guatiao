# guatiao (Fortran) — API header

Fortran bindings over the `guatiao` C ABI:
https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no
repository path.

fpm package `guatiao` (and a CMake target, `guatiao::fortran`), Fortran
2008 with `iso_c_binding` and `ieee_arithmetic`, tested with gfortran,
LLVM flang and Intel ifx, 64-bit platforms. No dependencies and no native
library of its own: guatiao is opened at run time, so a program links
nothing of it. The sources are pure Fortran (one `.F90` file chooses
`LoadLibraryW` or `dlopen` with the preprocessor).

## Modules

| `use` | what |
| --- | --- |
| `guatiao` | `guatiao_value`, `guatiao_text`, the constructors, statuses and tags, `GUATIAO_C_TRUE`/`GUATIAO_C_FALSE` (a C `bool`), `guatiao_error`, `guatiao_allocator`, `guatiao_use_library`; schemas and `guatiao_config`; the wire encoding and channels (`guatiao_wire`); `guatiao_registry`, `guatiao_instance`; `guatiao_impl`, `guatiao_kind`, `guatiao_serving`, `guatiao_remote`; `guatiao_library` |
| `guatiao_kinds` | what a kind's own module uses for its glue: served boxes, carriers, lent and handed-over arguments, objects |
| `guatiao_serde` | over the `guatiao_serde` library: `guatiao_parse`, `guatiao_emit`: JSON, TOML, YAML; `guatiao_serde_use_library` |
| `guatiao_intake` | over the `guatiao_intake` library: forms, paths, the flat store; `guatiao_intake_use_library` |
| `guatiao_civil` | dates, times, instants, spans, exact lengths |
| `guatiao_c`, `guatiao_serde_c`, `guatiao_intake_c` | the headers in Fortran: `bind(c)` mirrors (`gt_value`, `gt_entry`, `gt_provider_info`, ...) and a procedure pointer per export (`gt_value_free`, ...), resolved once the library is open |

## Finding the native library

One package, a module per library: `guatiao` (and the core modules)
over `guatiao`, `guatiao_serde` over `guatiao_serde`, `guatiao_intake`
over `guatiao_intake`; the core modules do not use the other two.
Nothing native is touched until a call needs a library; it is then found
in this order:

1. `call guatiao_use_library(path)` (`guatiao_serde_use_library`,
   `guatiao_intake_use_library` for theirs) before that first call: a
   file exporting the library's functions, whatever it is called, or a
   directory. Naming another file once one is open is
   `GUATIAO_ERR_BUSY`.
2. For `guatiao_serde` and `guatiao_intake`, `GUATIAO_SERDE_LIBRARY` /
   `GUATIAO_INTAKE_LIBRARY`: a file, taken as it is, or a directory
   holding the library under its platform name.
3. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib` (or `guatiao_serde.dll` and so
   on), or a file: taken as it is for guatiao, and for the others when it
   exports their functions (an application's own library carrying
   several surfaces), else their library beside it.
4. The platform's own library search, by file name.

Not found is `GUATIAO_STAT_LIBRARY_NOT_FOUND` with where it looked. An
export every build has is checked when a library opens; an optional one
(TOML, YAML) is `GUATIAO_STAT_MISSING_SYMBOL` naming its feature when
used. A library stays open for the process. `guatiao_library_path(which)`
(`GUATIAO_LIB_CORE`, `_SERDE`, `_INTAKE`) is where it was opened from.
The first call opens it: a program that calls from several threads at
once makes one call first.

## Statuses and errors

Every call of this binding takes an optional `stat` and `errmsg` (an
`allocatable` deferred-length character) last, in the standard library's
style: `stat` receives the status, `errmsg` the message when it failed,
and **with no `stat` a failure stops the program** (`error stop`) with
the message. In a provider, always pass `stat`.

Statuses are the C surface's: `GUATIAO_OK`, `GUATIAO_ERR_BAD_VALUE`,
`GUATIAO_ERR_ALLOC`, `GUATIAO_ERR_WRONG_KIND`, `GUATIAO_ERR_NOT_FOUND`,
`GUATIAO_ERR_NULL`, `GUATIAO_ERR_GONE`, `GUATIAO_ERR_UNSUPPORTED`,
`GUATIAO_ERR_BUSY`, `GUATIAO_ERR_INTERNAL`; this binding's own are
`GUATIAO_STAT_CLOSED` (1001), `GUATIAO_STAT_READ_ONLY`,
`GUATIAO_STAT_LIBRARY_NOT_FOUND`, `GUATIAO_STAT_MISSING_SYMBOL`,
`GUATIAO_STAT_FLOOR_MISMATCH` (1005). `guatiao_status_name(stat)` names
one.

`type(guatiao_error)` (`status`, `message`) is how a kind's slot method
reports: `type(guatiao_error), allocatable, intent(out) :: error`,
unallocated being success; `call guatiao_fail(error, status, message)`
refuses.

## Values

A `type(guatiao_value)` is a handle on a tree and a place in it. Copies
(`b = a`) share the tree; the tree is freed when the last handle goes (a
final procedure) or when `close()` is called, after which every handle
on it reads absent and answers `GUATIAO_STAT_CLOSED`. The place is a path
of keys and positions walked again on every access, so a handle stays
right while the array around it grows. A value is not safe to change
from two threads at once; reading one from several is.

```fortran
use guatiao
type(guatiao_value) :: v, tags, port
v = guatiao_map()
call v%set('host', '10.0.0.1')
call v%set('port', 5900)
call v%set('tags', guatiao_list())
tags = v%key('tags')
call tags%push('a')
port = v%key('port')
print *, port%int_or(22_int64)        ! 5900
call v%close()
```

**Building**: `guatiao_map(alloc)`, `guatiao_list(alloc)`,
`guatiao_null()`, `guatiao_absent()`, `guatiao_number(text, alloc)` (the
exact text, JSON's grammar), `guatiao_of(x, alloc)` for a `logical`,
`integer(int32/int64)`, `real(real32/real64)`, `character(*)` or
`integer(int8)` array. `alloc` is a `type(guatiao_allocator)` (`%ptr`,
the address of a `guatiao_alloc` that outlives every tree built through
it); without it trees are built on guatiao's own heap
(`guatiao_default_allocator()`).

**Places**: `v%key(k)`, `v%item(i)` (from 1; 0 the last, -1 the one
before), `v%at(s1, ..., s8)` (each step text or an integer). Fortran
allows no `%` after a function reference: assign the handle first, or
use `associate`.

**Reading**: `tag()` (`GUATIAO_TAG_ABSENT`, `_NULL`, `_BOOL`, `_NUMBER`,
`_STRING`, `_BYTES`, `_LIST`, `_MAP`), `is_absent()`, `is_null()`,
`length()` (entries, elements, bytes), `has(k)`, `keys()` (an array of
`guatiao_text`, `%text`, insertion order), `path_text()` (the intake
grammar, positions from 0), `is_closed()`, `raw()` (the node's address
now). With a fallback: `bool_or`, `int_or` (int64: a fraction, an
exponent, another kind or out of range falls back, never truncates),
`real_or` (real64, no finite one falls back), `string_or`,
`number_text()` ('' unless a number). Into a variable, generic over its
type: `call v%get(x, stat, errmsg)` and `call v%get_key(k, x, stat,
errmsg)` for `logical`, `integer(int32/int64)`, `real(real32/real64)`,
`character(:), allocatable` (a string; a number is not one), `integer(int8),
allocatable :: x(:)` (bytes, copied) and `guatiao_value` (a deep copy):
`GUATIAO_ERR_NOT_FOUND` for nothing there, `GUATIAO_ERR_WRONG_KIND` for
another kind, `GUATIAO_ERR_BAD_VALUE` for a number that does not fit.

**Writing**: `call v%set(k, x, stat, errmsg)` (a map; replaces in place,
keeping the position) and `call v%push(x, stat, errmsg)` (a list), for
the same types (a `guatiao_value` is copied, its source left as it was);
reals are written as the shortest text that reads back the same, `-0`
kept, NaN and the infinities refused. `remove(k)`, `remove_item(i)`,
`clear()`, `copy_from(src)` (a map's entries, collisions replaced),
`clone(alloc)`. A container grows through the allocator it records, so
what is added to a tree another side lent lands in its allocator. A
read-only handle answers `GUATIAO_STAT_READ_ONLY` from every write.

`guatiao_value_from_raw(node, alloc)` takes over a tree a C function
wrote into a `guatiao_value` at `node`: its 40 bytes move into a new
value and `node` is left ABSENT.

## Schemas and configurations

```fortran
type, extends(guatiao_config) :: shout_config
  character(:), allocatable :: prefix
  integer(int64) :: times = 1
contains
  procedure :: describe => shout_describe   ! subroutine (self, schema): type(guatiao_schema_builder)
  procedure :: decode => shout_decode       ! subroutine (self, value, error)
end type

subroutine shout_describe(self, schema)
  class(shout_config), intent(in) :: self
  type(guatiao_schema_builder), intent(inout) :: schema
  call schema%string_field('prefix', description='What every shout starts with', min_length=1)
  call schema%integer_field('times', default=1_int64, minimum=1_int64)
end subroutine
```

`guatiao_object_schema(title, description)` starts an object schema
that refuses keys it does not declare; `string_field(name, description,
default, min_length, max_length, pattern, format, sensitive, required)`,
`integer_field(name, description, default, minimum, maximum, required)`
(int64), `number_field` (real64), `boolean_field(name, description,
default, required)`, `field(name, schema, required)`; a field is
required unless it has a default or `required=.false.`; `sensitive`
writes `x-sensitive`. `%value()` is the document.
`guatiao_schema_of(config)` is a configuration type's schema;
`call guatiao_decode_config(config, value, stat, errmsg)` checks the value
against it and calls `decode`.

`call guatiao_validate(schema, value, why, stat, errmsg)`: `GUATIAO_OK`,
or `GUATIAO_ERR_BAD_VALUE` with `why` the map saying why (`why` optional).
`call guatiao_redact(schema, value, placeholder, shown, stat, errmsg)`:
a copy with every field the schema marks (`x-sensitive`, `writeOnly`)
replaced, local `$ref`s followed, and whatever it does not describe
hidden too.

## Values on the wire, and channels

The wire encoding is a tag byte per node, LEB128 lengths, a number as
its own text, bytes as bytes, a map in its order: the same bytes in every
language. Bytes cross as `integer(int8)` arrays.

```fortran
integer(int8), allocatable :: bytes(:), frame(:)
call guatiao_to_wire(v, bytes)                  ! what v names
call guatiao_from_wire(bytes, back, alloc, stat, errmsg)
```

`guatiao_from_wire` answers `GUATIAO_ERR_BAD_VALUE` for bytes that are not
exactly one value (truncated, an unknown tag, a length not in its
shortest form, text that is not UTF-8, a number outside the grammar, a
repeated key, anything after it), leaving `value` as it was.

A channel announces a schema before its values, and a receiver checks
every value against it. A frame is `kind:u8 channel:LEB128 payload`; the
transport says where one ends.

```fortran
type(guatiao_receiver) :: r
type(guatiao_wire_event) :: event
type(guatiao_wire_refusal) :: why
call guatiao_wire_announce(1_int64, schema, frame)
r = guatiao_open_receiver()
call r%accept(frame, event, why, stat)   ! event%kind, %channel, %value
call guatiao_wire_send(1_int64, v, frame)
call guatiao_wire_close(1_int64, frame)
call r%close()
```

- `guatiao_wire_announce(channel, schema, frame, stat, errmsg)`,
  `guatiao_wire_send(channel, value, frame, ...)`,
  `guatiao_wire_close(channel, frame, ...)`: `channel` is an
  `integer(int64)` holding the 64 bits C reads as unsigned; `frame` an
  allocatable `integer(int8)` array. A schema is not judged when it is
  sent: the receiver refuses one that is not a map.
- `guatiao_open_receiver(alloc, stat, errmsg)`: what it answers and the
  schemas it keeps are built in `alloc`, which outlives it. Copies share
  one receiver, freed when the last goes or on `close()`; then
  `GUATIAO_STAT_CLOSED`. Not safe from two threads at once.
- `r%accept(frame, event, refused, stat, errmsg)`: `event%kind`
  (`GUATIAO_FRAME_SCHEMA`, `_VALUE`, `_CLOSE`), `event%channel`,
  `event%value` (a copy of the schema now the channel's, or the value its
  schema accepted, this side's to keep; absent for a close). A refusal is
  its status, and `refused` (`type(guatiao_wire_refusal)`) says which:
  `code` `GUATIAO_WIRE_MALFORMED` (`GUATIAO_ERR_BAD_VALUE`, `at` the byte
  of the frame from 0), `GUATIAO_WIRE_NO_SCHEMA` (`GUATIAO_ERR_NOT_FOUND`),
  `GUATIAO_WIRE_NOT_A_SCHEMA` (`GUATIAO_ERR_WRONG_KIND`),
  `GUATIAO_WIRE_INVALID` (`GUATIAO_ERR_BAD_VALUE`, `key` the field,
  `expected` what it accepts when the schema says, `detail` the map
  `guatiao_validate` answers); `channel`, `kind` and `message` too. It
  changes nothing the receiver keeps and quotes nothing it was given.

## Registry

```fortran
type(guatiao_registry) :: reg
type(guatiao_value) :: providers
reg = guatiao_open_registry('my-host', '1.0')
call reg%scan_dir('/path/to/plugins', rules=['kind=greeter'])
call reg%providers('greeter', providers)      ! a list of maps, best first
```

`guatiao_open_registry(id, version, alloc, stat, errmsg)`: `alloc` is the
allocator its answers are built in and that it offers libraries (a
counting one in a test). Copies share one registry; `close()` frees it
once nothing else (an instance, a remote) holds it, and every call then
answers `GUATIAO_STAT_CLOSED`. Not safe from two threads at once: a host
sharing one guards it.

Every answer that is data is a `guatiao_value` the caller owns, as the C
surface documents its keys: `scan_dir(dir, rules, descending, answer)` and
`scan_path(spec, rules, descending, answer)` (`rules` an array of
`[!]KEY=VALUE`, matched against what a library declares before it is
mapped; `answer` `{loaded, skipped, failed, unreadable}`),
`load_file(path, answer)`, `register_entry(name, entry, answer)` (a
`guatiao_library_entry` as `c_funloc` gives it), `libraries(answer)`,
`providers(kind, answer)` (`''` for every kind; maps with `key`, `id`,
`version`, `library`, `display_name`, `from`, `kinds`, `has_config`,
`vtable_size`), `provider(key, answer)` (`GUATIAO_ERR_NOT_FOUND` for
none), `available(kind, answer)`, `why_not(kind, answer)`, `best(kind,
answer)`, `provider_available(key, ok, reason)`, `keyed_by(template)`
and `libraries_keyed_by(template)` (`%id` by default, `%version`,
`%name`, `%library`, `%source`), `set_priority(id, p)`, `priority(id)`,
`provider_table(key, kind, table, size)` (`kind` `''` for a single
table), `provider_ctx(key)`, `provider_config(key)` (a read-only handle on
the schema, absent for none), `provider_meta(key, answer [, stat, errmsg])`
(what the provider declared beyond its schema, its notices or its form
under `GUATIAO_INTAKE_PROVIDER_FORM_KEY`: a copy outliving the library,
absent when it declares nothing, `GUATIAO_ERR_NOT_FOUND` for an unknown
key),
`host()` (the `guatiao_host_info *`).

`create(key, config, instance)` checks the configuration against the
provider's schema first (`GUATIAO_ERR_BAD_VALUE` naming the field), then
builds a `guatiao_instance`; its `close()` (or its last copy going)
reaches the provider that built it, even after a retire or a refresh
loaded a new build under the key, and it keeps its registry alive.

`retire(key)` takes a library out and leaves it mapped until nothing
uses it; `collect(answer)` closes every retired library whose own
`unload` agrees. `unload(key)` unmaps when the library agrees:
`GUATIAO_ERR_BUSY` while something it handed out lives,
`GUATIAO_ERR_UNSUPPORTED` with no `unload`, `GUATIAO_ERR_WRONG_KIND` for a
library registered rather than mapped, `GUATIAO_ERR_NOT_FOUND` for none;
`unload_unchecked` insists. Both take the library's key.

**Sources**: `add_source_file(path, config)` maps a file exporting
`guatiao_source` (a bridge to another language) and adds the source it
answers, refused with its own words; `watch_dir(dir, rules)` watches a
directory; `refresh(answer)` makes the registry agree with every source
(`{loaded, retired, skipped, refused}`). Nothing refreshes on its own.

## Kinds: calling a table

A kind is an abstract type its author writes once, in a module of its
own (see "Writing a kind module"). Its remote type calls another side's
table with the same methods a provider implements, so code written
against `class(greeter)` takes either:

```fortran
use greeter_kinds
type(greeter_remote) :: g
type(guatiao_value) :: answer
type(guatiao_error), allocatable :: error
call greeter_bind(reg, 'acme_hello', g)
call g%greet('ana', answer, error)
if (allocated(error)) print *, error%status, error%message
```

Binding: `call guatiao_bind(reg, key, kind, remote%r)` (a kind module wraps
it as `<kind>_bind`), `guatiao_bind_instance(instance, kind, remote%r)`,
`guatiao_bind_table(table, size, ctx, kind, remote%r)` for any table:
`GUATIAO_STAT_FLOOR_MISMATCH` when the table is smaller than the kind's
required slots or carries another floor hash. A slot past the table's
size or null answers `GUATIAO_ERR_UNSUPPORTED` (a Rust caller would run
the kind's default body instead); a closed remote `GUATIAO_ERR_GONE`. A
tree answer is built in the registry's allocator (the carrier the remote
passes names it) and moved there when the callee built it elsewhere; a
tree lent to a slot is moved, once the call returns, into the allocator
it recorded when it was lent, whatever the callee grew it with or wrote
over it, and a refused move is the call's error when the slot answered.
The kind module's remote does both (Writing a kind module). A
remote holding an object another side handed over destroys it once, when
its last copy goes or on `close()`.

`type(guatiao_remote)` (`table`, `size`, `ctx`, `alloc`): `slot(i)` (the
`i`th slot after the header, from 1, or null), `has(i)`, `is_closed()`,
`close()`, `give(obj, stat)` (hands an owned object over as the
`gt_object` that crosses).

## Kinds: serving one

**A slot method's Fortran types**: `guatiao_str` is `character(*)`;
`guatiao_bytes_mut` an `integer(int8)` array over the caller's memory,
valid for the call; scalars their `iso_c_binding`/`int64` kinds (a
`uint64` is the `int64` holding its bits: Fortran has no unsigned integer,
so past 2^63 - 1 it is negative and 2^64 - 1 is -1), `bool` `logical`; a
tree answer a `guatiao_value` holding an empty map in the carrier's
allocator, filled in place (an answer the method replaces is copied
there); a `guatiao_string` answer `character(:), allocatable`; a borrowed
tree a read-only `guatiao_value`, a lent one a writable one, both closed
when the call returns (keep a copy and it reads closed); a tree handed
over a `guatiao_value` the method owns; an object handed over a
`class(<kind>), allocatable` the method keeps with `move_alloc` (or
leaves, and it is released); a lent object `class(<kind>), intent(inout),
target`; an object answer `class(<kind>), allocatable, intent(out)`. Every
slot method ends with `type(guatiao_error), allocatable, intent(out) ::
error`: unallocated is success; `guatiao_fail(error, status, message)`
refuses, and the status and words cross. A slot with no `err` in C reports
a failure's words on standard error and its status alone.

A served slot runs on the thread that calls it, a thread Fortran did not
start included; nothing is queued, so a provider guards what it shares.
A served `ctx` is the address of a box holding the objects; it is valid
until the destroy that releases it, as a C provider's is.

**An object this side serves** (a listener handed to another side, a
conversation a method answers) is moved into a box the holder destroys:
its `close()` runs then, and it is deallocated. A Fortran object lent for
one call is served in place. An object is never copied: keep state that
must be seen from outside behind a pointer component.
`guatiao_live_objects()` counts what is served and not yet destroyed.

## Providers and libraries

```fortran
type, extends(greeter) :: hello
contains
  procedure :: greet => hello_greet          ! deferred in greeter: the compiler checks it
  procedure :: shout => hello_shout          ! optional: declared below
end type

function my_library() result(lib)
  type(guatiao_library) :: lib
  type(hello), pointer :: h
  type(shout_config) :: proto
  allocate(h)
  lib = guatiao_new_library('my_greeters', '1.0.0', summary='Greeters', license='MPL-2.0')
  call lib%single('hello', [greeter_serving(h, shout=.true.)], display_name='Hello')
  call lib%configured('shouter', proto, build_shouter, [greeter_serving()])
end function
```

**A provider** is a type extending the kind; what it serves is declared
where it is registered, through the kind module's `<kind>_serving(impl,
<optional slot>=.true., ...)`, since Fortran cannot ask whether a type
overrides a procedure: an optional slot left undeclared is null in the
table (a Rust caller runs the kind's default body). A provider of several
kinds is one object per kind (Fortran has single inheritance), sharing
state through a pointer: the servings list each. `lib%single(name,
servings, display_name, id, version)` is a provider that is its one
instance; `lib%configured(name, prototype, build, serves, display_name,
id, version)` builds an instance per configuration: the host's value is
checked against `guatiao_schema_of(prototype)`, decoded into a copy of
`prototype`, and `build(config, made, error)` answers the instance's
servings (objects allocated through a pointer; they are closed and
deallocated when the host destroys the instance). The id defaults to
`<library id>_<name>`; two providers sharing one, an object kind among
the kinds, or an object not of its kind is refused. A process holds at
most 64 providers that build instances and registers at most 16
libraries in process (fixed pools of the descriptor slots that carry no
`ctx`).

**A library**: `guatiao_new_library(id, version, summary, license,
authors)`; write the id and version its `fpm.toml` states (fpm can bake
`{version}` into a preprocessor macro only as a bare token, which
gfortran's preprocessor cannot turn into text), and check them in the
package's own test. `lib%answer(host)` is the descriptor, built on first
use and kept for the process; it is null (declining the host, the reason
on standard error) for a host of another ABI or when guatiao cannot be
opened. `guatiao_register_library(reg, lib)` registers it in this
process: linked, not mapped, so `unload` answers `GUATIAO_ERR_WRONG_KIND`
and `retire` takes it out. **A Fortran library is never unmapped**: its
`unload` answers `GUATIAO_ERR_BUSY`, since a Fortran runtime linked into
it keeps state no host can see.

**A library file**: a shared library holding the binding, the kind
modules and the providers, exporting the entry and the declaration, both
written in Fortran:

```fortran
character(*), parameter :: DECLARES = 'kind=greeter' // c_null_char // c_null_char
character(kind=c_char), bind(c, name='guatiao_declares') :: guatiao_declares(len(DECLARES)) = &
  transfer(DECLARES, 'a', len(DECLARES))
type(guatiao_library), save :: library
logical, save :: made = .false.

function guatiao_library_entry(host) bind(c, name='guatiao_library_entry') result(info)
  type(c_ptr), value :: host
  type(c_ptr) :: info
  if (.not. made) then
    library = my_library()
    made = .true.
  end if
  info = library%answer(host)
end function
```

On Linux and macOS both are exported as any global symbol is. On Windows
link a `.def` file naming them (`EXPORTS`, `guatiao_library_entry`,
`guatiao_declares DATA`), which MinGW's `ld`, `lld-link` and MSVC's
`link` all take: flang ignores `!DIR$ ATTRIBUTES DLLEXPORT`, and gfortran
refuses `!GCC$ ATTRIBUTES DLLEXPORT` on an initialised `bind(c)`
variable. Build the file with CMake (`add_library(... SHARED)`, the `.def`
among its sources on Windows): fpm builds a shared library per package,
and the binding's would then be a second file named like guatiao's own.
The library
finds guatiao as any program does (`GUATIAO_LIBRARY` or the platform's
search).

## Writing a kind module

One module per kind header, written once by the kind's author: a
`bind(c)` mirror of each table; per kind an abstract type extending
`guatiao_impl` (a deferred procedure per required slot, an ordinary one
answering `GUATIAO_ERR_UNSUPPORTED` per optional slot), a `guatiao_kind`
description (`name`, `floor_hash` as the uint32's bits, `floor_size`, the
table's `size`, `object`, `object_table`, and an `accepts` function doing
a `select type`), a remote type extending the kind with a `guatiao_remote`
component and a method per slot calling the slot through
`c_f_procpointer`, a `<kind>_serving` function filling a table per set of
optional slots, and a `bind(c)` module procedure per slot: it finds its
object with `guatiao_impl_at(ctx, kind)`, converts the arguments with the
`guatiao_kinds` helpers (`guatiao_text_in`, `guatiao_bytes_in`,
`guatiao_map_carrier` and `guatiao_settle_map`, `guatiao_value_carrier`
and `guatiao_settle_value`, `guatiao_lent_map` and
`guatiao_end_lent_map`, `guatiao_taken_map`, `guatiao_borrowed_map`,
`guatiao_borrowed_value`, `guatiao_take_object`, `guatiao_lend_object`
and `guatiao_end_lent_object`, `guatiao_serve_object`,
`guatiao_lend_impl`, `guatiao_string_out`), calls the method in a
`select type`, and answers `guatiao_failed(error, err)` or
`guatiao_warn(error, slot)`.

The remote's method per slot answers `GUATIAO_ERR_UNSUPPORTED` when
`self%r%has(i)` is false (`GUATIAO_ERR_GONE` when `self%r%is_closed()`),
takes the slot with
`c_f_procpointer(self%r%slot(i), f)`, and builds its arguments and reads
its answer with the remote-side helpers:

| helper | for |
| --- | --- |
| `guatiao_string_in(text)` | a `guatiao_str` argument, a view of `text` for the call |
| `guatiao_lend_tree(v, tag, ptr, stat, home)` | a lent tree: `ptr` the arm to pass, `home` the allocator it records now |
| `guatiao_settle_lent(v, tag, home, stat)` | right after the call, answered or refused: the lent tree moved into `home`; a non-zero `stat` (optional) is the error to report when the slot itself answered, after freeing what it answered (a tree's carrier, an object) unread |
| `guatiao_hand_over(v, tag, node, stat)` | a tree handed over: moved out of `v` (or copied), `node`'s arm passed |
| `guatiao_object_arg(impl, remote, kind, obj, stat)` | an object handed to the other side: a remote this side owns given, any other object served |
| `guatiao_map_out(answer, out, alloc)` | a tree answer's carrier: an empty map naming `alloc` (the remote's `self%r%alloc`) |
| `guatiao_settle_home(answer, stat)` | once the call answered: the answer moved into the carrier's allocator; a non-zero `stat` (optional) is an error to report |
| `guatiao_call_error(status, err, error)` | the slot's status and words as `error`, its string freed |

A remote that lends a tree without `guatiao_settle_lent`, or keeps an
answer without `guatiao_settle_home`, may leave blocks of the callee's
library in the caller's tree, which are freed through code that is gone
once the library unloads.

A C `bool` crosses as `integer(c_int8_t)`,
written `GUATIAO_C_TRUE` or `GUATIAO_C_FALSE` and read as `/= 0`: ifx
stores a `logical(c_bool)` .true. as -1. Mark each `recursive`, initialise no local
in its declaration (that implies `save`), and call no function returning
deferred-length text (see Threads).

**A `guatiao_object` passed by value** (24 bytes) is read wrongly by
flang 22 in a `bind(c)` procedure and passed wrongly through a procedure
pointer, on AArch64 and Windows, where the ABI passes the address of the
caller's copy. Write such an argument as that address under flang there,
by value elsewhere:

```fortran
#if defined(__flang__) && (defined(__aarch64__) || defined(_WIN32))
  type(c_ptr), value :: obj_at        ! c_f_pointer it to a type(gt_object)
#else
  type(gt_object), value :: obj
#endif
```

`guatiao_str`, `guatiao_bytes` and `guatiao_bytes_mut` (16 bytes) cross
by value, with one exception: flang 22 on x86-64, calling through a
procedure pointer, puts such a struct on the stack when it lands in the
fifth and sixth integer argument registers (counting a pointer or integer
as one and such a struct as two). A remote slot call where one does
declares it as its two words off Windows (`type(c_ptr), value` and
`integer(c_size_t), value`, which every 64-bit ABI but Windows' passes
the same way) and as the struct on Windows, where it goes by address.
The binding does this for `guatiao_registry_scan_dir_rules` and
`guatiao_registry_scan_path`. A `bind(c)` procedure receives such a
struct correctly.

## guatiao_serde: JSON, TOML, YAML

```fortran
call guatiao_parse('{"port": 5900}', GUATIAO_FORMAT_JSON, v)
call guatiao_emit(v, GUATIAO_FORMAT_YAML, text)
```

`guatiao_parse(text, format, value, how, alloc, where, stat, errmsg)`:
`format` is `GUATIAO_FORMAT_JSON`, `_TOML` or `_YAML`; `how` the
`GUATIAO_BYTES_*` constants or'd with `GUATIAO_READ_DATA_URIS`. A
document that does not parse is `GUATIAO_ERR_BAD_VALUE`, `value` left as
it was, and `where` (`type(guatiao_parse_error)`: `format`, `line`,
`column` from 1, the column in characters, 0 when unknown; `reason`, which
never quotes the document) says where. `guatiao_emit(value, format, text,
pretty, how, stat, errmsg)`: `pretty` indents JSON; a TOML document is a
map (`GUATIAO_ERR_WRONG_KIND` otherwise). A number keeps its exact text.

## guatiao_intake

```fortran
call guatiao_intake_check(schema, form, why, stat)          ! why: absent, or the error map
call guatiao_intake_layout(schema, form, sections)          ! [{"section": ..., "fields": [key, ...]}]
call guatiao_intake_is_visible(schema, form, 'key', values, shown)
at = guatiao_intake_at(value, 'agent[1].name')               ! read-only handle
call guatiao_intake_for_schema(schema, form)
call guatiao_intake_form_for(form, schema, 'path', sub)      ! absent when there is none
field = guatiao_intake_resolve(schema, 'auth.password')       ! read-only handle
call guatiao_intake_flat_keys(schema, 'auth', keys)          ! guatiao_text array
call guatiao_intake_flatten(schema, 'auth', value, store)    ! a map of text to text
call guatiao_intake_unflatten(schema, 'auth', store, value)  ! absent when nothing is stored
form = meta%key(GUATIAO_INTAKE_PROVIDER_FORM_KEY)             ! 'form', in reg%provider_meta(key, meta)
```

A handle `guatiao_intake_at` or `guatiao_intake_resolve` answers is on
its source's tree, found again on every access, read-only, closed with
it. A key the schema does not declare is `GUATIAO_ERR_NOT_FOUND`
(`is_visible`) or `GUATIAO_ERR_WRONG_KIND` (`flat_keys`); a store that is
not text to text is `GUATIAO_ERR_WRONG_KIND`.

## guatiao_civil: dates, times, spans, exact lengths

The forms are text in a value, fixed at
<https://boriken-dev.github.io/guatiao/encodings/>, and every text is
read and written by the library's own functions.

```fortran
type(guatiao_date) :: d, later
call guatiao_from_text('2026-01-31', d)
call guatiao_add_span(d, guatiao_span(months=1), later)
call guatiao_to_text(later, text)                         ! "2026-02-28"
call guatiao_normalize(GUATIAO_FORM_SPAN, '1h 30m', text)  ! "PT1H30M"
```

- `guatiao_date` (`year`, `month`, `day`), `guatiao_time` (`hour`,
  `minute`, `second`, `nanosecond`), `guatiao_datetime` (`date`, `time`):
  civil, no zone. `guatiao_timestamp` (`seconds` int64 from the epoch,
  `nanoseconds` forward). `guatiao_span` (seven int64 fields and
  `nanoseconds`, one sign for all). `guatiao_length` (`seconds`, an
  unsigned 64-bit count in an int64's bits; `nanoseconds`).
- `guatiao_from_text(text, x, stat, errmsg, unparsed)` and
  `guatiao_to_text(x, text, stat, errmsg)`, generic over the first five;
  a field past its C width (a month of 300) or off the calendar is
  `GUATIAO_ERR_BAD_VALUE`. Text the grammar refuses is
  `GUATIAO_ERR_BAD_VALUE` with `unparsed` (`type(guatiao_unparsed)`: `at`,
  the byte offset from 0; `expected`); it never quotes the text.
- `guatiao_normalize(form, text, canonical, ...)` with
  `GUATIAO_FORM_DATE`, `_TIME`, `_DATETIME`, `_TIMESTAMP`, `_SPAN`.
- `guatiao_add_span(start, span, result, ...)` for a date or a datetime.
- `guatiao_read_length(value, length, ...)` reads `[seconds,
  nanoseconds]`, a number of seconds or `{secs, nanos}`;
  `guatiao_length_value(length)` writes `[seconds, nanoseconds]`.

## Threads

The binding keeps no mutable state a call shares, past opening the
library and building a library's descriptor (the first entry call). A
served slot runs on the calling thread; a registry, a value and a remote
holding an object are not safe from two threads at once. gfortran 14 and older keep the length of a function's
deferred-length character result in static storage, so two threads
calling such a function at once (`string_or`, `number_text`,
`path_text`, `guatiao_status_name`) corrupt the heap there; the
subroutine forms (`get`, `get_key`) are safe on every compiler.

Every procedure of the binding and of the example kind is `recursive`:
Intel ifx keeps the derived-type and pointer locals of a procedure that
is not in static storage, so one reached again while it runs (a close
that destroys another served object) or from a second thread overwrites
them. A provider's own procedures that a slot reaches need the same, or
build with ifx's `-assume recursion`.
