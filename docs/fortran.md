# Fortran: host and provider

Fortran plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a library,
types extending a kind's abstract type, that a host in the same program
calls, or that it builds as a library file any host's scan finds. Both
use the package `guatiao` in `bindings/fortran/guatiao`: pure Fortran 2008 over
`iso_c_binding`, tested with gfortran, LLVM flang and Intel ifx, opening
the guatiao library at run time and shipping no native code of its own.

With fpm, as a git dependency (fpm reads the repository's root manifest,
which builds `bindings/fortran/guatiao/src`):

```toml
[dependencies]
guatiao = { git = "https://github.com/Boriken-Dev/guatiao", tag = "..." }
```

With CMake, `FetchContent` with `SOURCE_SUBDIR bindings/fortran/guatiao` and
`target_link_libraries(app PRIVATE guatiao::fortran)`. A program links
nothing of guatiao: the binding opens it on its first native call, the one
`guatiao_use_library(path)` named, else `GUATIAO_LIBRARY` (a file or a
directory), else the platform's library search.

Every call of the binding takes an optional `stat` and `errmsg` last, the
standard library's way: without `stat`, a failure stops the program.

## Values

A `guatiao_value` is a handle on a tree and a place in it. Copies share
the tree, which is freed when the last copy goes or on `close()`; the
place is walked again on every access.

```fortran
use guatiao
type(guatiao_value) :: v, tags
v = guatiao_map()
call v%set('host', '10.0.0.1')
call v%set('port', 5900)
call v%set('tags', guatiao_list())
tags = v%key('tags')
call tags%push('a')
```

`set`, `push` and `get` are generic over Fortran's types; numbers keep
their exact text (`guatiao_number('1.10')`), and a real is written as the
shortest text that reads back the same. Fortran allows no `%` after a
function reference, so a place is assigned to a handle before it is used.

## A kind, as an abstract type

A kind is the table its C header declares. In Fortran it is an abstract
type extending `guatiao_impl`, with a deferred procedure for every slot
the table must have and an ordinary one, answering
`GUATIAO_ERR_UNSUPPORTED`, for each slot appended later. The kind's
author writes it once, in a module of its own beside the header
(`examples/greeter_kind/fortran`), with a remote type calling another
side's table and a `bind(c)` procedure per slot serving one:

```fortran
type, abstract, extends(guatiao_impl) :: greeter
contains
  procedure(greeter_greet), deferred :: greet
  procedure :: shout => greeter_shout          ! optional
end type

abstract interface
  subroutine greeter_greet(self, name, answer, error)
    import :: greeter, guatiao_value, guatiao_error
    class(greeter), intent(inout) :: self
    character(*), intent(in) :: name
    type(guatiao_value), intent(inout) :: answer        ! an empty map, in the caller's allocator
    type(guatiao_error), allocatable, intent(out) :: error
  end subroutine
end interface
```

A slot method reports through `error`: unallocated is success, and
`guatiao_fail(error, status, message)` refuses with a status and words
that cross to the caller.

The remote's method per slot moves what it keeps home: a tree answer with
`guatiao_settle_home` once the call answered, and a tree it lent with
`guatiao_settle_lent` once the call returns, into the allocator the tree
recorded when `guatiao_lend_tree` lent it. Otherwise a slot that built
in its own allocator leaves blocks of its library in the caller's tree,
freed through code that is gone once the library unloads. The binding's
`AGENTS.md` lists every helper a remote uses.

## As a host

```fortran
use guatiao
use greeter_kinds
type(guatiao_registry) :: reg
type(greeter_remote) :: g
type(guatiao_value) :: answer, providers
type(guatiao_error), allocatable :: error

reg = guatiao_open_registry('my-host', '1.0')
call reg%scan_dir('/path/to/plugins', rules=['kind=greeter'])
call reg%providers('greeter', providers)
call greeter_bind(reg, 'acme_hello', g)
call g%greet('ana', answer, error)
```

A remote has the kind's methods, so code written against
`class(greeter)` takes a local provider or a remote alike. The registry's
answers are values in its allocator; `create(key, config, instance)` checks
a configuration against the provider's schema and builds an instance;
`add_source_file`, `watch_dir` and `refresh` keep libraries that come and
go, the guatiao-py bridge serving Python modules among them.

## As a provider

```fortran
type, extends(greeter) :: hello
contains
  procedure :: greet => hello_greet           ! the compiler checks it is there
  procedure :: shout => hello_shout
end type

type, extends(guatiao_config) :: shout_config
  character(:), allocatable :: prefix
contains
  procedure :: describe => shout_describe     ! its schema, field by field
  procedure :: decode => shout_decode         ! the checked value into the type
end type

function my_greeters() result(lib)
  type(guatiao_library) :: lib
  type(hello), pointer :: h
  type(shout_config) :: proto
  allocate(h)
  lib = guatiao_new_library('my_greeters', '1.0.0')          ! as fpm.toml says
  call lib%single('hello', [greeter_serving(h, shout=.true.)])
  call lib%configured('shouter', proto, build_shouter, [greeter_serving()])
end function
```

A provider declares the optional slots it overrides where it is
registered: Fortran cannot ask whether a type overrides a procedure. A
provider of several kinds is one object per kind, sharing state through a
pointer, since a type extends one parent. A configured provider's
configuration is checked against the schema its type describes, decoded
into a copy of the prototype, and built into an instance's objects.

The library's id and version are the ones its `fpm.toml` states, written
once more in the source and checked by the package's test: fpm puts
`{version}` into a preprocessor macro only as a bare token, which
gfortran's preprocessor cannot turn into text.

`guatiao_register_library(reg, lib)` registers it in the host's own
program.

## As a library file

Built as a shared library, the providers are a file any host's scan
finds: Rust, C, Python, Go or Fortran. The entry and the declaration a
scan reads are written in Fortran:

```fortran
character(*), parameter :: DECLARES = 'kind=greeter' // c_null_char // c_null_char
character(kind=c_char), bind(c, name='guatiao_declares') :: guatiao_declares(len(DECLARES)) = &
  transfer(DECLARES, 'a', len(DECLARES))

function guatiao_library_entry(host) bind(c, name='guatiao_library_entry') result(info)
  type(c_ptr), value :: host
  type(c_ptr) :: info
  info = library%answer(host)     ! a module variable holding my_greeters()
end function
```

On Windows a `.def` file names the two for the linker (`EXPORTS`,
`guatiao_library_entry`, `guatiao_declares DATA`); elsewhere they are
exported as any global symbol is. CMake builds the file with the binding
and the kinds linked in (`examples/fortran_greeter/CMakeLists.txt`). The
library opens its own guatiao as any program using the binding does, and
it is never unmapped: its `unload` refuses, since a Fortran runtime linked
into it keeps state no host can see.

## Threads

A slot a Fortran provider or object serves runs on the thread that calls
it, a thread Fortran did not start included: nothing is queued, so a
library's own threads call Fortran at once, and a provider guards what it
shares. A value, a registry and a remote holding an object are not safe
to use from two threads at once. gfortran 14 and older keep the length of
a function's deferred-length text result in static storage, so functions
such as `string_or` are not safe from two threads at once there; the
subroutine forms (`get`, `get_key`) are. Intel ifx keeps the
derived-type and pointer locals of a procedure that is not `recursive`
in static storage: the binding marks every procedure `recursive`, and a
provider's procedures that a slot reaches need the same (or ifx's
`-assume recursion`).

## What Fortran does not do that C does

- **Pass every struct by value.** LLVM flang 22 gets a struct of more
  than 16 bytes passed by value wrong in a `bind(c)` procedure and in a
  call through a procedure pointer on AArch64 and Windows. The one such
  argument a kind can have, a `guatiao_object` handed over, crosses there
  as the address of the caller's copy, which is what those ABIs pass; a
  kind module says so with the preprocessor. On x86-64, flang 22 calling
  through a procedure pointer puts a 16-byte struct (a `guatiao_str`) on
  the stack when it lands in the last two argument registers; such a call
  passes the struct's two words instead.
- **Pass a `logical` to C.** Intel ifx stores a `logical(c_bool)`
  .true. as -1, which no C or Rust reader accepts; a C `bool` crosses as
  `integer(c_int8_t)`, `GUATIAO_C_TRUE` or `GUATIAO_C_FALSE`.
- **Make a function pointer at run time.** A served slot is a `bind(c)`
  module procedure written in the kind's module; the descriptor slots that
  name no provider (a configured provider's `create`, the entry of a
  library registered in process) come from fixed pools: 64 and 16.
- **Ask a type what it overrides**, or extend two types: optional slots
  are declared at registration, and a provider of two kinds is two
  objects.
- **Hold an object by reference.** An object handed to another side is
  moved into what serves it, never copied; state that must be seen from
  outside lives behind a pointer component.
- **Run a kind's default body** for a slot an older table lacks, as a
  Rust caller does: a remote answers `GUATIAO_ERR_UNSUPPORTED`.
- **Be unloaded.** A Fortran library file stays mapped once loaded.
- **Version a pre-release in fpm.** fpm takes three numbers: a
  pre-release `0.0.0-alpha.N` is `0.0.0` in `fpm.toml`, and the tag names
  the build.

## The examples

- `examples/fortran_host/` scans for greeters in any language, registers
  the Fortran library, optionally adds the guatiao-py bridge, and calls
  every slot of the kind; its tests call the Rust, C, Python, Go and
  Fortran greeters both ways and the Fortran one from native threads.
- `examples/fortran_greeter/` is the library above, its test checking
  its identity against its `fpm.toml`, and its CMake project the same
  library as a file.
- `examples/greeter_kind/fortran/` holds the greeter kinds as abstract
  types, with their remotes and glue.
