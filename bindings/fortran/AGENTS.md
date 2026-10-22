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
| `guatiao` | `guatiao_value`, `guatiao_text`, the constructors, statuses and tags, `guatiao_error`, `guatiao_allocator`, `guatiao_use_library`; schemas and `guatiao_config` |
| `guatiao_serde` | `guatiao_parse`, `guatiao_emit`: JSON, TOML, YAML |
| `guatiao_intake` | forms, paths, the flat store |
| `guatiao_civil` | dates, times, instants, spans, exact lengths |
| `guatiao_c`, `guatiao_serde_c`, `guatiao_intake_c` | the headers in Fortran: `bind(c)` mirrors (`gt_value`, `gt_entry`, `gt_provider_info`, ...) and a procedure pointer per export (`gt_value_free`, ...), resolved once the library is open |

## Finding the native library

Nothing native is touched until a call needs it; the library is then
found in this order:

1. `call guatiao_use_library(path)` before that first call: a file
   exporting the ABI, whatever it is called, or a directory. Naming
   another file once one is open is `GUATIAO_ERR_BUSY`.
2. `GUATIAO_LIBRARY`: a directory holding `guatiao.dll` /
   `libguatiao.so` / `libguatiao.dylib`, or a file (taken as it is for
   guatiao; `guatiao_serde` and `guatiao_intake` are looked for beside it
   under their own names).
3. The platform's own library search, by file name.

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
library. gfortran 14 and older keep the length of a function's
deferred-length character result in static storage, so two threads
calling such a function at once (`string_or`, `number_text`,
`path_text`, `guatiao_status_name`) corrupt the heap there; the
subroutine forms (`get`, `get_key`) are safe on every compiler.
