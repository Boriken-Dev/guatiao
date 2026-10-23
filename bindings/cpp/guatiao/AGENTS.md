# guatiao (C++) — API header

C++ bindings over the `guatiao` C ABI: https://github.com/Boriken-Dev/guatiao/
Self-contained — this file ships with the package, so it names no
repository path.

Header-only, C++17 or later, over `<guatiao.h>` and the `guatiao` shared
library, which a program links. Tested with MSVC, clang and g++. No
dependencies outside the standard library. `#include <guatiao/guatiao.hpp>`;
everything is in namespace `guatiao`.

## The package and the library

A CMake package: `find_package(guatiao CONFIG)` (with `guatiao_DIR`
naming this package's directory), `add_subdirectory`, or `FetchContent`
with `SOURCE_SUBDIR` naming it; then
`target_link_libraries(app PRIVATE guatiao::guatiao)`. The target carries
both headers' directories, C++17, and the library. The library is, in
order:

1. `GUATIAO_LIBRARY` (a CMake variable, else the environment): a
   directory holding `guatiao.dll` (with its import library
   `guatiao.dll.lib`) / `libguatiao.so` / `libguatiao.dylib`, or a file,
   taken as it is (an application's own library carrying every guatiao
   surface);
2. otherwise built by cargo from the Rust sources beside the package
   (`GUATIAO_CARGO_PROFILE`, default `release`; `GUATIAO_CARGO_TARGET` for
   another target), at configure time, into the build directory. Rust
   from https://rustup.rs is then the only prerequisite.

No prebuilt library is distributed. On Windows a program finds the DLL
beside itself or on `PATH`: `guatiao_copy_runtime(<target>)` copies it
there after each build, and `guatiao_runtime_dirs(<var>)` answers the
directories. Elsewhere the build records an rpath.

## Errors: exceptions, or `try_` and `result<T>`

Every call that can fail has two forms. `try_x(...)` answers
`guatiao::result<T>`: `ok()`, `operator bool`, `code()` (a `status`),
`error()`, `operator*`/`->` (unchecked), `value()` (throws the error),
`value_or(fallback)`. `x(...)` is `try_x(...).value()`: it throws
`guatiao::error`. Built with exceptions off (`-fno-exceptions`, `/EHs-c-`,
or `GUATIAO_NO_EXCEPTIONS` defined), only the `try_` forms exist and
`value()` on a failure prints and aborts.

`guatiao::error` derives from `std::exception`: `code()` (a `status`),
`message()` (the other side's words, possibly empty), `what()`, and
`detail()`, a `view` of a map some calls attach (each says which), absent
otherwise. `enum class status : uint32_t` is `ok`, `bad_value`, `alloc`,
`wrong_kind`, `not_found`, `null`, `gone`, `unsupported`, `busy`,
`internal`, or any other code a library answers; `status_name(s)`.

## Values

| type | what |
| --- | --- |
| `value` | owns one tree (a root `guatiao_value`), freed when destroyed; move-only, moved-from is ABSENT; `clone()` copies |
| `map`, `list` | an owned value that is always a map / a list |
| `view` | a borrowed read-only node (`const guatiao_value *`; null reads ABSENT) |
| `map_view`, `list_view` | a borrowed map / list container, iterable |
| `node` | a writable place inside a tree (`guatiao_value *`), valid until its parent container changes, like an iterator |
| `map_ref`, `list_ref` | a writable map / list inside a tree |
| `item` | a description of a tree to build |

**Building**: `try_make(item, alloc = default_alloc())` / `make(...)`.
An `item` is `nullptr` (null), a `bool`, any integer, a finite `double`
or `float` (its shortest text; NaN and infinities are refused when
built), text (`const char *`, `std::string_view`, `std::string`), a
`bytes_view`, a `view` or `const value&` (a deep copy), or
`item::number(text)` (exact text, the JSON grammar), `item::bytes(b)`,
`item::list({...})`, `item::map({{"k", x}, ...})`, `item::list_from(vector)`,
`item::map_from(vector<pair<string, item>>)`, `item::absent()`. `value()`
is ABSENT, `value::null()` null. `map(alloc)` and `list(alloc)` are
empty; `map{{"k", 1}}`, `list{1, 2}` and `map::try_of(...)`,
`list::try_of(...)` build; `map::from(value&&)` checks (WRONG_KIND).

```cpp
guatiao::value v = guatiao::make(guatiao::item::map({{"host", "10.0.0.1"}, {"port", 5900},
                                                     {"tags", guatiao::item::list({"a"})}}));
v.set("port", 5901);
v["tags"].push("b");
v["port"].int_or(22);              // 5901
for (guatiao::entry e : v.as_map()) { e.key; e.value; }
```

**Reading** (on every handle): `tag()`, `is_absent()`, `is_null()`,
`is_bool()`, `is_number()`, `is_string()`, `is_bytes()`, `is_list()`,
`is_map()`; `boolean()`, `string()`, `number_text()`, `bytes()`
(`std::optional`, empty for another kind; a NUMBER is not a STRING);
`as<T>()` (`std::optional<T>` for `bool`, every integer type — exactly,
from the number's text, never truncated: a fraction, an exponent or out
of range is empty — `float`/`double` (nearest), `std::string_view`,
`std::string`, `bytes_view`); `bool_or`, `int_or`, `float_or`,
`string_or`; `operator[](key)`, `operator[](index)`, `find(key)`,
`at(index)` (an ABSENT `view` when nothing is there; on a non-const
`value`, `node` or `map`/`list`, a writable `node`, null when nothing is
there); `contains(key)`; `size()` (entries or elements, else 0);
`as_map()`, `as_list()` (a view, empty for another kind: safe in a
range-`for`); `clone(alloc)`. `operator==` is deep: same kinds, text,
bytes, booleans, maps holding the same pairs in the same order; numbers
compare as text. Keys compare as bytes; a map keeps insertion order.

**Writing**, on a `node`: `set(key, x)` (replaces in place, keeping the
position), `erase(key)` (false when absent), `push(x)`, `erase_at(i)`,
`clear()`, `update(view)` (copies a map's entries in), `append(text)` /
`append(bytes)` (a STRING / BYTES), `assign(value&&)` and `take()` (moves
the node out, leaving null). `value` has `set`, `erase`, `push`,
`erase_at`, `clear` and `ref()` (its root as a `node`); `map`/`map_ref`
the map's, `list`/`list_ref` the list's. `x` is a `value&&`, moved in and
left ABSENT (a refusal leaves it as it was), or an `item`, built through
the allocator the node records. A write the kind does not take is
WRONG_KIND; text that is not UTF-8 is BAD_VALUE; a null node is NULL.

**Raw**: `value::adopt(guatiao_value)` takes over a root a C call wrote;
`release()` gives one up; `raw()`/`address()` answer the node's address.
`map::adopt(guatiao_map)`, `list::adopt(guatiao_list)` and their
`release()` do the same for a bare container.

**Allocators**: a tree records the `guatiao_alloc` that built each of its
buffers, so whoever frees it needs nothing else. `system_alloc()` is one
over the C heap, one per binary, alive for the process;
`default_alloc()` is the one a call builds in when given none:
`system_alloc()`, unless every file of a program defines
`GUATIAO_CPP_DEFAULT_ALLOC` as the name of a function answering another
before including this header (a test counting blocks does).

## Dates, times, spans, exact lengths

Text forms fixed at <https://boriken-dev.github.io/guatiao/encodings/>,
read and written by the library's own functions.

- `date {year, month, day}`, `time_of_day {hour, minute, second,
  nanosecond}`, `date_time {date, time}`, `timestamp {seconds,
  nanoseconds}` (from the Unix epoch, nanoseconds forward), `span {years,
  months, weeks, days, hours, minutes, seconds, nanoseconds}` (one sign
  for the whole; `==` unit by unit), `length {seconds, nanoseconds}` (an
  exact length).
- `T::parse(text)` / `T::try_parse`, `.to_string()` / `.try_to_string()`;
  `date::add(span)`, `date_time::add(span)`; `span::negated()`,
  `is_zero()`; `timestamp::to_time_point<Clock>()`,
  `timestamp::from(time_point)`; `length::read(view)` (its three
  spellings), `length::to_item()` (`[s, ns]`), `length::from(duration)`,
  `to_duration()`; `normalize(form, text)` with `form::{date, time,
  datetime, timestamp, span}`.
- Text a grammar refuses is BAD_VALUE with detail `{"at": byte,
  "expected": words}`; it never quotes the text. A field past its C width
  (a month of 300) is BAD_VALUE.

## Values on the wire, and channels

`to_wire(view)` → `std::vector<uint8_t>`; `from_wire(bytes, alloc)` (BAD_VALUE
for bytes that are not exactly one value). `wire_announce(channel,
schema)`, `wire_send(channel, value)`, `wire_close(channel)` write
frames. A `receiver` (move-only, `receiver(alloc)`) keeps each channel's
schema: `accept(frame)` answers an `event {frame kind; uint64_t channel;
value data;}` (`frame::{schema, value, close}`), or a refusal: an error
whose status is the refusal's (`malformed` BAD_VALUE, `no_schema`
NOT_FOUND, `not_a_schema` WRONG_KIND, `invalid` BAD_VALUE), message the
refusal in words, detail `{"refusal", "channel", "kind", "at", "detail"}`
(`detail`: the refused field, for `invalid`). A refusal changes nothing the
receiver keeps. One caller at a time.

## Schemas, secrets, layering

`validate(schema, config)`: BAD_VALUE with detail `{key, expected or
known, message}`, never the refused value. `redact(schema, value,
placeholder, alloc)`: a copy with every secret (`x-sensitive`,
`writeOnly`) and everything the schema does not describe replaced.
`merge(merge_mode::{simple, deep, substitute}, earlier, later,
merge_options{merge_lists, overrides}, alloc)`: a new tree, later winning;
a disagreement's detail is the map describing it.

## Threads

A value is not safe to change from two threads at once; reading one from
several is. Every handle is a plain C++ object with no hidden locks.

## Not offered

- Streaming a value through a visitor or a walker without building it:
  call `guatiao_value_visit` and the walker functions of `<guatiao.h>`.
- A source written in C++ (`guatiao_source_info`): fill the C table.
