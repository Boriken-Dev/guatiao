# R: host and provider

R plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a
library, S4 or Reference classes with methods of a kind's generics, that
a host in the same R session calls, whatever language the host is written
in. Both use the package `guatiao`: a source package whose small C glue,
compiled when it installs, opens the guatiao library at run time and calls
it, for R 4.2 and later, with no dependency outside base R.

```r
remotes::install_github("Boriken-Dev/guatiao", subdir = "bindings/r")
```

Installing needs R's compiler setup (Rtools on Windows) and libffi
(`libffi-dev` on Debian and Ubuntu, `libffi-devel` on Fedora; macOS and
Rtools carry their own). The package loads the guatiao library on its
first native call: the one `use_library(path)` named, else
`GUATIAO_LIBRARY` (a file or a directory), else the platform's library
search. The library must be built for the R process's architecture; CRAN's
R for Windows is x64, so on Windows ARM64 it needs an x64 build.

## A kind, declared once

A kind is the table its C header declares. In R it is a virtual S4 class
and a generic per slot, declared with `kind()` from the header: the slots
in the header's order, what each takes and answers, and the floor hash
the header defines.

```r
Listener <- kind("listener", floor_hash = 1867921573, object = TRUE,
  heard = method(what = "string", required = TRUE))

Greeter <- kind("greeter", floor_hash = 2695856491,
  greet = method(name = "string", returns = "map", required = TRUE, fallible = TRUE),
  shout = method(name = "string", returns = "string"),
  start = method(listener = taken(Listener), returns = Conversation, fallible = TRUE),
  greet_into = method(name = "string", listener = lent(Listener), fallible = TRUE),
  greet_from = method(config = taken("map"), returns = "map", fallible = TRUE),
  check = method(config = lent("map"), returns = "bool", fallible = TRUE))
```

`kind()` defines, where it is called (a package's namespace as it
installs): the class `Greeter`, a generic per slot (`greet(self, name)`,
dispatching on `self`), and `GreeterRemote`, a class extending `Greeter`
whose method of each generic calls the slot. A map, list or value is
borrowed by default; `lent()` makes it the caller's tree for the call,
changed in place; `taken()` hands it over, as Rust's arguments by value
do. An object kind (`object = TRUE`) is passed `lent()` or `taken()`, and
its `destroy` slot is `close()`. The declarations of the example kinds
are `examples/greeter_kind/r/greeterkind`.

## As a host

```r
library(guatiao)
library(greeterkind)

reg <- registry("my-host", "1.0")
reg$watch_dir("/opt/plugins", rules = "kind=greeter")
reg$add_source_file("/opt/app/libguatiao_py.so", list(dirs = list("/opt/app/py")))
reg$refresh()

g <- reg$bind("py_greeter_hello", Greeter)
answer <- greet(g, "ana")             # a value: list(greeting = "hello, ana")
value_data(answer$greeting)
close(answer)

chat <- start(g, my_listener)         # an S4 object with a method of heard()
say(chat, "hi")
close(chat)                           # destroy: the greeter lets the listener go
reg$close()
```

A registry is an R object with methods (`reg$scan_dir`, `reg$providers`,
`reg$bind`, `reg$create`, `reg$retire`, `reg$unload`, `reg$refresh`);
every answer that is data is plain R data. A remote checks its registry
or instance on every call; a refusal signals a `guatiao_error` carrying
the provider's status and words.

## As a provider

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
```

A provider is an S4 or Reference class containing the kinds it serves:
`setMethod` refuses a method whose arguments are not the generic's, and a
required slot with no method is refused when the library is made. A
configuration is an S4 class whose typed slots give the schema the host
checks before anything is built; `config_class()` adds defaults, docs and
keywords. The package is the library: `library_of("rgreeter")` takes its
id and version from `DESCRIPTION`, its authors, description and licence
too, its licence and notice files as notices, and its providers from the
`provider()` declarations, in order. `reg$register_library(lib)`
registers it, and any host in this session, Rust, C, Python, Go, Julia or
R, calls its providers through the same tables it calls any library's
through.

## Threads

R code runs only on R's main thread. A slot an R provider or object
serves runs when a host calls it on that thread: an R host's own call, or
a library calling back during one. **Called from any other thread it
does not run**: it answers `Unsupported` at once, with "R code runs only
on R's main thread", touching no R object; a provider's `available`
answers false with that reason; a destroy asked for there is queued and
done on R's thread at the package's next call. Nothing waits for R's
thread, so nothing deadlocks; a library that calls providers from worker
threads of its own needs providers written in another language.

A method runs inside `R_ToplevelExec`, so an R error, an interrupt or any
condition nobody handles ends there and never unwinds through the C or
Rust frames between the library and R; it crosses as a status
(`Internal` with its message, or the status of a `guatiao_error`).

## What R does not do that Python does

- **Run a provider on a library's own thread.** Python takes the GIL on
  any thread; R has no such lock, and its interpreter belongs to its main
  thread, so a call from another thread is refused.
- **Serve another process.** guatiao-py serves Python modules to any
  host; nothing serves R packages to a host that is not running R, which
  would need R embedded in a bridge.
- **Hold a 64-bit integer.** An R number holds integers exactly to 2^53:
  a slot's `int64` answer, a span's units and an exact length past that
  are refused or rounded. Inside a value a number keeps its exact text.
- **Run on Windows ARM64 as released.** CRAN builds only an experimental
  R-devel for it; the x64 R runs under emulation, against an x64 build of
  the library.
- **Run a kind's default body** for a slot an older table lacks, as a
  Rust caller does: a remote answers `Unsupported`, as in Python, Node,
  Go and Julia.

## The examples

- `examples/r_host/host.R` scans for greeters in any language, registers
  the R library, optionally adds the guatiao-py bridge, and calls every
  slot of the kind; its README has the commands.
- `examples/r_provider/rgreeter/` is the library above.
- `examples/greeter_kind/r/greeterkind/` declares the greeter kinds.
