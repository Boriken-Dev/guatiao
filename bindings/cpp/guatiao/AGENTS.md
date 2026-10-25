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
`item::map_from(vector<pair<string, item>>)`, `item::absent()`. A
character or a pointer other than `const char *` is not an `item` (a
compile error: say `std::string(1, c)`, `int(c)` or `p != nullptr`).
`value()` is ABSENT, `value::null()` null; a container may hold an
ABSENT, as the wire encoding may carry one, and a lookup reads it as
nothing there. `map(alloc)` and `list(alloc)` are
empty; `map{{"k", 1}}`, `list{1, 2}` and `map::try_of({...})`,
`list::try_of({...})` build from a braced list; `map::from(value&&)` and
`list::from(value&&)` check (WRONG_KIND).

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
`as_map()`, `as_list()` (a view, empty for another kind; asked of a
temporary `value`, `map` or `list` it is a compile error, since the view
would outlive the tree); `clone(alloc)`. Any view of an owner is valid
while the owner lives and until a write moves the array it reads. The
iterators answer proxies (`entry {key, value}`, `view`), with `->`: an
input iterator by the C++17 categories, random access by C++20's. `operator==` is deep: same kinds, text,
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
WRONG_KIND; text that is not UTF-8 is BAD_VALUE; a null node is NULL. A
value moved in is never the tree holding the node it moves into.

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
  `is_zero()`; `timestamp::to_time_point<Clock>()` (BAD_VALUE for an
  instant the clock cannot hold: a 64-bit nanosecond clock ends in 2262),
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

## Registry

```cpp
guatiao::registry reg("my-host", "1.0");                  // registry::try_open(...) without exceptions
reg.watch_dir("/opt/plugins", guatiao::rules({"kind=greeter"}));
reg.add_source_file("/opt/bridge/libguatiao_source_py.so", guatiao::make(guatiao::item::map({{"dirs", ...}})));
guatiao::value changed = reg.refresh();                   // {"loaded", "retired", "skipped", "refused"}
auto g = greeter_kind::greeter::bind(reg, "derived_greeter_hello");
guatiao::instance inst = reg.create("acme_shouter", guatiao::make(guatiao::item::map({{"prefix", "hey"}})));
auto shouter = greeter_kind::greeter::bind(inst);
```

`registry(id, version, alloc = default_alloc())`: `alloc` is offered to
every library and builds every answer. Move-only; freeing it unmaps
nothing. Not safe to call from two threads at once. Each method has its
`try_` form:

- loading: `load_file(path)`, `register_entry(name, entry)`,
  `register_library<f>(name)` (a C++ library this program carries, `f`
  as `GUATIAO_LIBRARY` takes it), `scan_dir(dir, rules, descending)`,
  `scan_path(spec, rules, descending)`; `rules({...})` joins `[!]KEY=VALUE`
  lines. Each answers the report map. Mapping a file runs its static
  initialisers.
- sources: `watch_dir(dir, rules)`, `add_source_file(path, config)` (a
  file exporting `guatiao_source`, kept for the registry's life; a
  refusal carries the source's words), `add_source(name, table)`,
  `refresh()`. Nothing refreshes on its own.
- asking: `libraries()`, `providers(kind)` (best first; every provider
  for an empty kind), `provider(key)`, `available(kind)`, `why_not(kind)`,
  `best(kind)`, `provider_unavailable(key)` (`std::optional<std::string>`,
  asked every time), `provider_config(key)` (a borrowed `view`, until the
  library is retired), `provider_meta(key)` (a copy), `provider_table(key,
  kind)` and `provider_ctx(key)` (raw), `host()` (the
  `guatiao_host_info *`, alive for the process).
- ranking and keys: `set_priority(id, n)`, `priority(id)`,
  `keyed_by(template)`, `libraries_keyed_by(template)`.
- instances: `create(key, config)` checks the configuration against the
  provider's schema first (BAD_VALUE naming the field), then builds an
  `instance` (move-only; `close()` or its destructor destroys it through
  the provider that built it, even after a retire or a refresh, and it
  keeps that library open until then). An instance is closed before its
  registry is freed.
- leaving: `retire(library_key)`, `unload(library_key)` (BUSY while
  something it handed out lives, UNSUPPORTED with no `unload` slot,
  WRONG_KIND for a library this program links), `unload_unchecked`,
  `collect()`. They take the LIBRARY key.

## Kinds

A kind is declared once, over its C table, by its author (beside its C
header):

```cpp
GUATIAO_OBJECT_KIND(listener, listener_vtable,
    (required, heard, void(std::string_view)));
GUATIAO_KIND(greeter, greeter_vtable,
    (required, greet, guatiao::map(std::string_view)),
    (optional, shout, std::string(std::string_view)),
    (optional, start, guatiao::object<conversation>(guatiao::object<listener>)),
    (optional, greet_into, void(std::string_view, guatiao::lent<listener>)),
    (optional, greet_from, guatiao::map(guatiao::map)),
    (optional, check, bool(guatiao::map_ref)));
```

`GUATIAO_KIND(name, table, slots...)` (a provider kind) and
`GUATIAO_OBJECT_KIND` (its table starts with `destroy`, which is not
listed) declare a struct `name` (`kind_name`, `floor_hash` from
`<table>_FLOOR_HASH`, `floor_size()`); `GUATIAO_KIND_NAMED(name,
"kind-name", ...)` names the kind otherwise. A slot is `(required |
optional, <C field>, <C++ signature>)`. **A signature that does not stand
for its C field is a compile error.** The C++ types and the C types they
stand for:

| C++ parameter | C | |
| --- | --- | --- |
| `std::string_view` | `guatiao_str` | UTF-8, else BAD_VALUE when served |
| `guatiao::bytes_view` / `bytes_mut` | `guatiao_bytes` / `guatiao_bytes_mut` | `bytes_mut` is the caller's memory, written in place |
| `bool`, integers, `float`, `double` | themselves | |
| `view`, `map_view`, `list_view` | `const guatiao_value/map/list *` | borrowed, read-only, for the call |
| `node`, `map_ref`, `list_ref` | `guatiao_value/map/list *` | LENT: the caller's tree for the call, changed in place |
| `value`, `map`, `list` | `guatiao_value/map/list *` | HANDED OVER: the callee owns it, on every path |
| `guatiao::object<K>` | `guatiao_object` | an object handed over |
| `guatiao::lent<K>` | `const guatiao_object *` | an object lent for the call, never destroyed or kept |

| C++ answer | C out slot |
| --- | --- |
| `void` | none |
| `value`, `map`, `list` | `guatiao_value/map/list *out`, built where the carrier says |
| `std::string` | `guatiao_string *out` |
| a scalar `T` | `T *out` |
| `guatiao::object<K>` | `guatiao_object *out` |

Whether a slot can refuse with words is read from the table (a trailing
`guatiao_provider_error *`). **Lent and handed over are both a plain
pointer in C**: the kind's declaration says which, as the C header's slot
comment does ("Lent for the call"); declare it as the header says.

**Checked against the description**: a kind with a `<crate>.kinds.json`
is checked, not compiled, by `guatiao_check_kinds(<test name> HEADER
<file.hpp> DESCRIPTION <file.kinds.json> [EXPECT <regex>])`, a CMake
function `find_package(guatiao)` defines (`cmake/GuatiaoKindCheck.cmake`).
It adds a test failing on any declared kind that is not described, or of
the other sort (object or not), and any slot missing, undescribed,
required or optional against its description, or whose signature is not
the one the description spells: lent `node`/`map_ref`/`list_ref`/`lent<K>`,
handed over `value`/`map`/`list`/`object<K>`, borrowed
`view`/`map_view`/`list_view`, and the scalar types as `std::int64_t`,
`std::size_t` (`usize`), `std::ptrdiff_t` (`isize`), `float`, `double`.
Namespace qualifiers are ignored; a `_NAMED` kind is looked up by the name
it gives, and `object<K>`/`lent<K>` of it spelled by its struct. Relative
paths are from the calling directory. With `EXPECT`, the test passes only
when the check's output matches. A kind with no description is declared by
hand alone.

**Calling a table**: `remote<K>` (a provider's table and `ctx`, from
`K::bind(registry, key)`, `K::bind(instance)`,
`try_bind_table<K>(table, size, ctx, alloc)`), `object<K>` (an object
this side owns) and `lent<K>` have a method per slot — `g.greet("ana")`,
`g.try_greet("ana")` (a `result`), `g.has_shout()`. A table of another
kind (another floor hash, or too short for its required slots) is
WRONG_KIND; a slot past the table's size, or empty, answers UNSUPPORTED
(C++ cannot run the kind's default body, as Rust does). A refusal is the
status's `error` with the provider's words. What a call hands over is
moved in (`std::move`); when the call never happens (no slot) it is freed
here. An answer arrives in the remote's allocator (the registry's), moved
there if built elsewhere; a lent tree is settled once the call returns,
answered or refused, into the allocator it recorded when it was lent,
whatever the callee grew it with or wrote over it. Passing an argument
the slot does not
take is a compile error.

A `remote` is shared: its methods are `const` and may be called from any
thread, as the provider allows. An `object` or a `lent` is one holder's:
its slot methods need a handle that is not `const`, one call at a time. A
`remote` holds no lease: it is valid while the provider's library is open
(until it is unloaded, or collected once retired), as a raw table is in
C; a `remote` bound to an instance, while the instance is.

**Objects**: `object<K>::make<T>(args...)` and `object<K>::of(unique_ptr<T>)`
serve a C++ object of class `T` as kind `K` (its destructor is the
`destroy` slot); `lent<K>(t)` lends one for a call (`g.greet_into("ana",
my_listener)` lends implicitly); `adopt(raw)`, `release()`, `reset()`,
`raw()`, `usable()`. An object destroys itself through its table when the
handle goes.

## Providers and libraries

A provider is a C++ class whose methods are the slots of the kinds it
serves, by the slot's name. A method fits its slot when it can be called
with the slot's parameters and answers its answer — the answer type, one
convertible to it, or a `result<answer>`. **A method that does not fit,
or a required slot left out, is a compile error**; an optional slot the
class has no method for is left empty (an overloaded or template method
counts as none). A method refuses by throwing: a `guatiao::error` crosses
as its status and words, `std::bad_alloc` as ALLOC, any other exception as
INTERNAL with its `what()`; or, without exceptions, by answering a
failed `result`. Nothing unwinds into C. Several threads may call a
provider at once; it guards what it shares.

```cpp
class hello {
 public:
  guatiao::map greet(std::string_view name) {
    if (name.empty()) throw guatiao::error(guatiao::status::bad_value, "nobody to greet");
    return guatiao::map{{"greeting", "hello, " + std::string(name)}};
  }
  std::int64_t count() { return ++asked_; }
 private:
  std::atomic<std::int64_t> asked_{0};
};

class shouter {   // one instance per configuration
 public:
  explicit shouter(guatiao::view config);         // or static make(view) -> unique_ptr / result
  static guatiao::item schema();                  // what the host's configuration is checked against
  guatiao::map greet(std::string_view name);
};

guatiao::library acme() {
  guatiao::library lib("acme", "1.0.0");
  lib.meta(guatiao::item::map({{"summary", "..."}, {"license", "MPL-2.0"}}));
  lib.single<hello, greeter_kind::greeter, greeter_kind::counter>("hello").display_name("Hello");
  lib.configured<shouter, greeter_kind::greeter>("shouter");
  return lib;
}

GUATIAO_LIBRARY(acme, greeter_kind::greeter, greeter_kind::counter)
```

- `library(id, version)`: `meta(item)` (a map), `single<P, Ks...>(name,
  args...)` (its one instance, a `P` built from `args`; `single_of` takes
  a `unique_ptr<P>`), `configured<P, Ks...>(name[, schema])` (an instance
  per configuration: `P::make(view)` when it has one, else `P(view)`;
  the schema `P::schema()` unless given). Each answers a `provider_ref`:
  `id(..)` (default `{library id}_{name}`), `display_name(..)`,
  `version(..)` (empty: the library's), `meta(..)`. A class may have
  `const char *unavailable() const` (null when it can run here; the text
  outlives every reader: a literal).
- `GUATIAO_LIBRARY(f, kinds...)` once per shared library: exports
  `guatiao_library_entry` (building the library from `f` on the first
  host's call, declining a host of another envelope version) and
  `guatiao_declares` (`kind=<name>` per kind listed, read by a scan before
  the file is mapped). The library's `unload` slot agrees once nothing it
  served (an instance, an object) is alive in that binary.
- Build the shared library with `guatiao_library_target(<target>)` (CMake):
  only the two entry symbols exported (`-fvisibility=hidden`,
  `-fvisibility-inlines-hidden`), so no table, inline function or static
  of it is merged with the host's or another library's by the platform's
  loader; and named as a Rust cdylib names its file.
- A host carrying the library itself registers it with
  `reg.register_library<f>(name)`.
- On Windows the library imports `guatiao.dll`, found as any DLL's
  dependency is: already loaded in the process (a host linking guatiao,
  or one that loaded it), beside the host's executable, or on `PATH`.

## Building with and without exceptions

Everything is declared in an inline namespace named for the mode
(`guatiao::cxx_exceptions`, `guatiao::cxx_no_exceptions`), so a program
whose files mix the two links each to its own definitions; a value does
not cross between files built in different modes. Doubles cross as text
through `<charconv>` where it converts floating point, else through
`snprintf`/`strtod` with the locale's decimal point put back to `.`
(`GUATIAO_CPP_NO_FLOAT_CHARCONV` chooses that path).

## Threads

A value is not safe to change from two threads at once; reading one from
several is. A `remote` may be called from any thread; an `object` or a
`lent` from one at a time; a `registry` from one at a time. A slot served
by a C++ class runs on the thread that calls it, a thread the program did
not start included.

## Not offered

- Streaming a value through a visitor or a walker without building it:
  call `guatiao_value_visit` and the walker functions of `<guatiao.h>`.
- A source written in C++ (`guatiao_source_info`): fill the C table.
