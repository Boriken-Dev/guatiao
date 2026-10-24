# C++: host and provider

C++ plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it serves a kind
from an ordinary class, in a library a host in the same program registers
or a library file any host's scan finds. Three header-only packages, one
per crate: `bindings/cpp/guatiao` (`guatiao::guatiao`),
`bindings/cpp/guatiao-serde` (`guatiao::serde`) and
`bindings/cpp/guatiao-intake` (`guatiao::intake`). C++17 or later, tested
with MSVC, clang and g++, over the committed C headers: a C++ program
links the guatiao libraries like any C consumer.

## The CMake packages

```cmake
find_package(guatiao CONFIG REQUIRED)          # -Dguatiao_DIR=<repo>/bindings/cpp/guatiao
find_package(guatiao-serde CONFIG REQUIRED)    # -Dguatiao-serde_DIR=<repo>/bindings/cpp/guatiao-serde
target_link_libraries(app PRIVATE guatiao::guatiao guatiao::serde)
guatiao_copy_runtime(app)                      # Windows: the DLLs beside the program
```

`add_subdirectory` and `FetchContent` (with `SOURCE_SUBDIR
bindings/cpp/guatiao`) work too. Each package finds the library it links
in this order: its own variable (`GUATIAO_LIBRARY`,
`GUATIAO_SERDE_LIBRARY`, `GUATIAO_INTAKE_LIBRARY`) naming a file or a
directory; then `GUATIAO_LIBRARY` (a directory, or a file: an
application's own library carrying every surface, see
[One library for your application](app-library.md)); otherwise it builds
the library with cargo from the repository's sources when CMake
configures, so Rust is the only prerequisite. No prebuilt library is
distributed. `GUATIAO_SERDE_FEATURES` (default `toml;yaml`) says which
formats the serde library carries.

## Values

```cpp
#include <guatiao/guatiao.hpp>

guatiao::value v = guatiao::make(guatiao::item::map({{"host", "10.0.0.1"}, {"port", 5900},
                                                     {"tags", guatiao::item::list({"a"})}}));
v.set("port", 5901);
v["tags"].push("b");
std::int64_t port = v["port"].int_or(22);
for (guatiao::entry e : v.as_map()) { /* e.key, e.value */ }
```

`value` owns a tree and frees it in its destructor; it moves, and
`clone()` copies. `map` and `list` are owned values of one kind; `view`,
`map_view` and `list_view` borrow read-only; `node`, `map_ref` and
`list_ref` are writable places inside a tree, valid until their parent
container changes, as an iterator is. `item` describes a tree to build.
Numbers keep their exact text: `as<std::int64_t>()` answers only a value
that fits, never a truncation, and a double is written as the shortest
text that reads back the same.

Every call that can fail has two forms: `x(...)` throws `guatiao::error`
(a status, the other side's words, and for some calls a detail map), and
`try_x(...)` answers `guatiao::result<T>`. Built with exceptions off, only
the `try_` forms exist, so `-fno-exceptions` code uses the same headers.

The text forms ([Encodings](encodings.md)) are `date`, `time_of_day`,
`date_time`, `timestamp`, `span` and `length`, each read and written by
the library's own functions; `to_wire`/`from_wire` and a `receiver` carry
values and channels; `validate`, `redact` and `merge` act on schemas.
`guatiao::serde::parse`/`emit` read and write JSON, TOML and YAML;
`guatiao::intake` checks forms, lays them out and stores values flat.

## A host

```cpp
guatiao::registry reg("my-host", "1.0");
reg.watch_dir("/opt/plugins", guatiao::rules({"kind=greeter"}));
reg.add_source_file(bridge, guatiao::make(guatiao::item::map({{"dirs", guatiao::item::list({"py"})}})));
guatiao::value changed = reg.refresh();       // {"loaded", "retired", "skipped", "refused"}

auto g = greeter_kind::greeter::bind(reg, "derived_greeter_hello");
guatiao::map answer = g.greet("ana");

guatiao::instance inst = reg.create("derived_greeter_shouter", guatiao::make(guatiao::item::map({{"prefix", "hey"}})));
auto shouter = greeter_kind::greeter::bind(inst);
```

The registry loads files, scans directories by what a library declares,
keeps sources (watched directories, a bridge to another language) for
`refresh()`, ranks providers, builds instances from a configuration it
checks against the provider's schema, and retires and unloads libraries.

## A kind

A kind is the table its C header declares. In C++ it is declared once,
over that table, by the kind's author, beside the header
(`examples/greeter_kind/cpp/greeter_kind.hpp`):

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

Each line is a slot: whether a table must carry it, its C field, and its
C++ signature. **A signature that does not stand for its C field does
not compile.** A `std::string_view` is a `guatiao_str`; `view`,
`map_view` and `list_view` are trees borrowed read-only; `map_ref`,
`list_ref` and `node` are trees **lent** for the call and changed in
place; `map`, `list` and `value` are trees **handed over**, the callee's
from then on; `object<K>` is an object handed over and `lent<K>` one lent
for the call. Lent and handed over are the same C pointer: the
declaration says which, as the C header's slot comment does, and a test
checks it against the kind's description:

```cmake
find_package(guatiao CONFIG REQUIRED)
guatiao_check_kinds(greeter_kind_cpp_agrees
  HEADER ${CMAKE_CURRENT_SOURCE_DIR}/cpp/greeter_kind.hpp
  DESCRIPTION ${CMAKE_CURRENT_SOURCE_DIR}/include/greeter_kind.kinds.json)
```

It fails, naming each, on a slot declared `map` that the description
says is lent (`map_ref`), a required slot declared optional, a slot left
out, or any signature other than the one the description spells.
[Kinds in every language](kinds.md) sets this beside the other
languages' declarations.

`remote<K>` (a provider's table), `object<K>` and `lent<K>` call any
table of the kind, whatever language wrote it, with a method per slot:
`g.greet("ana")`, `g.try_greet("ana")`, `g.has_shout()`. A slot the table
does not carry answers UNSUPPORTED. What is handed over is moved in; an
answer arrives in the host's allocator. A provider's table may be called
from any thread; an object's slots, from one holder at a time.

## A provider

A provider is a class whose methods are the slots of the kinds it
serves. No base class: the compiler checks each method against its slot,
and **a method that does not fit, or a required slot left out, does not
compile**.

```cpp
class hello {
 public:
  guatiao::map greet(std::string_view name) {
    if (name.empty()) throw guatiao::error(guatiao::status::bad_value, "nobody to greet");
    return guatiao::map{{"greeting", "hello, " + std::string(name)}};
  }
  std::string shout(std::string_view name);
  guatiao::object<conversation> start(guatiao::object<listener> listener);   // keeps the listener
  void greet_into(std::string_view name, guatiao::lent<listener> listener);   // lent for this call
  bool check(guatiao::map_ref config);                                         // the caller's map
  std::int64_t count();
};

class shouter {   // built from a configuration
 public:
  explicit shouter(guatiao::view config);
  static guatiao::item schema();
  guatiao::map greet(std::string_view name);
};

guatiao::library greeters() {
  guatiao::library lib("cpp_greeter", "1.0.0");
  lib.single<hello, greeter, counter>("hello").display_name("Hello, in C++");
  lib.configured<shouter, greeter>("shouter");
  return lib;
}

GUATIAO_LIBRARY(greeters, greeter, counter)
```

A method refuses by throwing: a `guatiao::error` crosses as its status
and words, `std::bad_alloc` as ALLOC, anything else as INTERNAL with its
`what()`; nothing unwinds into the caller. Without exceptions a method
answers a `guatiao::result`. A tree the method answers is moved into the
allocator the caller named; a lent map is written back whatever happens.

`GUATIAO_LIBRARY(f, kinds...)` makes the shared library a guatiao library:
it exports `guatiao_library_entry` and the `guatiao_declares` a scan reads
before mapping the file, and the library agrees to be unmapped once
nothing it served is alive. Build it with `guatiao_library_target(<target>)`
in CMake, which exports those two symbols and nothing else, so no table
or inline function is merged with the host's or another library's.
`examples/cpp_greeter` is the example; the Rust and Python example hosts
scan it and call it. A host carrying the library itself calls
`reg.register_library<greeters>("name")` instead. On Windows the library
imports `guatiao.dll`, found already loaded in the process, beside the
host's executable, on `PATH`, or beside the library itself: a load that
fails is tried again with the library's own directory searched.

## What C++ cannot do that Rust can

- Run a kind's default body for a slot an older table leaves out: such a
  call answers UNSUPPORTED.
- Tell a lent tree from one handed over by its C type: the kind's
  declaration says which, and `guatiao_check_kinds` holds it to the
  kind's description.
- Hold a lease: a `remote` is valid while the provider's library is open,
  as a raw table is in C.

## Tests

```bash
cargo build --workspace --all-features
cd bindings/cpp/guatiao
GUATIAO_LIBRARY=../../../target/debug cmake -B build && cmake --build build && ctest --test-dir build
```

Each suite builds twice, with exceptions on and off, with warnings as
errors; files that must not compile (a misdeclared method, a view of a
temporary) are tests too. `examples/cpp_host` calls the greeters written
in Rust, C, Python, Go and C++ through one description of the kind.
