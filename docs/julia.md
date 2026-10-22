# Julia: host and provider

Julia plays both parts. As a **host** it loads libraries written in any
language and calls their providers; as a **provider** it writes a
library, structs with methods for a kind's functions, that a host in the
same process calls, whatever language the host is written in. Both use
the package `Guatiao`: `ccall` over the ABI, for Julia 1.10 and later,
with nothing to compile, no dependency outside Julia's standard library,
and no native code of its own.

```julia
using Pkg
Pkg.add(url = "https://github.com/Boriken-Dev/guatiao", subdir = "bindings/julia/Guatiao")
```

The package loads the guatiao library on its first native call: the one
`Guatiao.use_library(path)` named, else `GUATIAO_LIBRARY` (a file or a
directory), else the platform's library search. The library must be built
for the Julia process's architecture; there is no Julia for Windows on
ARM64, so an x64 Julia there needs an x64 build.

## A kind, declared once

A kind is the table its C header declares. In Julia it is an abstract
type and a generic function per slot, declared with `@kind` from the
header: the slots in the header's order, what each takes and answers as
type annotations, and the floor hash the header defines.

```julia
module GreeterKind
using Guatiao: @kind, Lent, Taken, MapRef

@kind Listener floor_hash = 1867921573 object = true begin
    @required heard(what::String)::Nothing
end

@kind Conversation floor_hash = 2769624503 object = true begin
    @required @fallible say(what::String)::Nothing
    @required read(dst::Lent{Vector{UInt8}})::Int64
    @required turns()::Int64
end

@kind Greeter floor_hash = 2695856491 begin
    @required @fallible greet(name::String)::MapRef
    shout(name::String)::String
    @fallible start(listener::Taken{Listener})::Conversation
    @fallible greet_into(name::String, listener::Lent{Listener})::Nothing
    @fallible greet_from(config::Taken{MapRef})::MapRef
    @fallible check(config::Lent{MapRef})::Bool
end
end
```

`@required` marks the slots every table has, the ones the floor hash
covers; the rest were appended, and an older table may lack them.
`@fallible` marks a slot ending in `guatiao_provider_error *err`. A slot
is found by its position, so its Julia name is free. `MapRef` answers a
map, `String` a text, `Nothing` nothing; `Lent{MapRef}` is the caller's
map changed in place, `Taken{MapRef}` one handed over; an object kind is
`Taken{K}`, `Lent{K}` or an answer `K`.

`@kind Greeter` also defines `GreeterRemote <: Greeter`, whose methods of
`greet`, `shout` and the rest call a table's slots. Code written against
the kind (`f(g::Greeter)`) takes a remote or a Julia provider alike.

## As a host

```julia
using Guatiao, GreeterKind

reg = Registry("my-host", "1.0")
Guatiao.scan_dir(reg, "/opt/plugins"; rules = "kind=greeter")

g = Guatiao.bind(reg, "derived_greeter_hello", Greeter)
answer = GreeterKind.greet(g, "ana")          # a Value: Dict("greeting" => "hello, ana")
close(answer)

config = Value(Dict("name" => "bo"))
GreeterKind.check(g, config)                  # lent: filled in where it stands

ear = Ear()                                   # any struct with a method of heard
chat = GreeterKind.start(g, ear)              # handed over; a conversation back
GreeterKind.say(chat, "hi")
close(chat)                                   # its destroy slot

inst = Guatiao.create(reg, "derived_greeter_shouter", Dict("prefix" => "hey"))
GreeterKind.greet(Guatiao.bind(inst, Greeter), "ana")
close(inst)
close(reg)
```

The registry answers plain Julia data (`providers`, `libraries`, `best`,
`why_not`); `create` checks a configuration against the provider's schema
before the provider sees it. A table whose floor hash is not the kind's is
refused before anything in it runs (`FloorMismatch`); a slot an older
table lacks answers `Unsupported`, which `Guatiao.has(g, :shout)` asks
first. A refusal is a `GuatiaoError` with the provider's status and words.

Libraries that come and go are a **source**: `watch_dir!` makes a
directory one, `add_source_file!` adds a bridge to another language (the
guatiao-py bridge serves Python modules), and `refresh!` makes the
registry agree with every source when the host asks. `retire!`, `unload!`
and `collect!` take a library out, unmapping it when it agrees.

## As a provider

A provider is a concrete struct with methods for its kinds' functions,
dispatching on it; `@provider` says what it serves. A configuration is a
struct, its schema read from its fields.

```julia
module JlGreeter
using Guatiao, GreeterKind
import GreeterKind: greet, count

struct Hello <: Greeter
    asked::Threads.Atomic{Int}
end
Hello() = Hello(Threads.Atomic{Int}(0))

function greet(::Hello, name::String)
    isempty(name) && throw(Guatiao.GuatiaoError(Guatiao.BadValue, "nobody to greet"))
    return Dict("greeting" => "hello, $name")
end
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
```

- A struct subtypes the one kind it serves, or names several in `serves`
  (Julia has single inheritance). A required slot it has no method for is
  refused when the library is built.
- With no `config`, the struct's `T()` is the provider's one instance.
  With `config = C`, the host's configuration is checked against
  `Guatiao.schema(C)` and the instance is `T(c::C)`. `Guatiao.@config` is
  `@kwdef` that also puts each default and docstring in the schema.
- A method that throws answers a status: a `GuatiaoError` crosses as its
  status and words, anything else as `Internal`. Nothing thrown crosses
  into C.
- An answer is built in the allocator the caller carried in. An instance
  or object with a `close` method is closed when its host destroys it.

**The package is the library.** `Guatiao.library_of(JlGreeter)` takes its
id from the module's name, its version, authors, description and licence
from its `Project.toml`, and its providers from the `@provider`
declarations, in order; a module loaded from no package is refused unless
a version is given. `Guatiao.register_library(reg, JlGreeter)` registers
it, and any host in this process, Rust, C, Python, Go or Julia, calls its
providers through the same tables it calls any library's through.

## Threads

A slot a Julia provider or object serves runs on the thread that calls
it, a thread Julia did not start included: Julia adopts it, and nothing
is queued, so a library's own threads call Julia at once, and a provider
guards what it shares as any Julia value shared between tasks does.

Every call this package makes that can run a library's code is made in a
**GC-safe region**. A callback arriving on another thread may allocate
enough to start a collection, which waits for every Julia thread to reach
a safepoint; a thread blocked in a plain `ccall` never reaches one, and
the process hangs. A host's own `ccall` into C that may call back from
another thread does the same (`ccall(:jl_gc_safe_enter, Int8, ())`
before, `jl_gc_safe_leave` after, with its arguments converted and rooted
first).

## What Julia does not do that Python does

- **Serve another process.** guatiao-py serves Python modules to any
  host; nothing serves Julia packages to a host that is not running
  Julia, which would need `libjulia` embedded in a bridge.
- **Read a kind from a class.** Python reads a `Protocol` class's
  annotations when it runs; a Julia kind is a macro over the slot list,
  and a provider's struct cannot list the kinds it serves by inheritance
  beyond one: `serves` names them.
- **Read a struct's defaults.** A dataclass's defaults are data; a
  `@kwdef` struct's are code, which `Guatiao.@config` records.
- **Build without limit.** A library's entry and its providers' `create`
  and `available` take no context naming them, and Julia makes no C
  function from a closure, so each comes from a fixed pool: 64 libraries
  and 256 providers per process.
- **Run on Windows ARM64.** Julia ships no build for it; an x64 Julia
  there runs under emulation, against an x64 build of the library.
- **Run a kind's default body** for a slot an older table lacks, as a
  Rust caller does: a remote answers `Unsupported`, as in Python, Node and
  Go.

## The examples

- `examples/julia_host/host.jl` scans for greeters in any language,
  registers the Julia library, optionally adds the guatiao-py bridge, and
  calls every slot of the kind; its README has the commands.
- `examples/julia_provider/JlGreeter/` is the library above.
- `examples/greeter_kind/julia/GreeterKind/` declares the greeter kinds.
