# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A library written in Julia: its package is the library, its structs the
# providers. What a provider answers is built in the allocator its caller
# carried in, a lent tree grows in the allocator it records, what it was
# handed is freed, and a slot called from a thread Julia did not start runs
# there, the caller's thread waiting in a GC-safe call.

using GreeterKind, JlGreeter
using Guatiao: GStr, GMap, GValue, GProviderError, TAG_MAP, node_of, new_root, free_root

# ---- kinds and providers declared here -------------------------------------------

@kind Pair floor_hash = 7 begin
    @required both(a::Taken{MapRef}, b::Taken{MapRef})::Int64
end

@kind Mirror floor_hash = 8 begin
    @required show_me()::MapRef
end

struct Sizes <: Pair end
function both(::Sizes, a::Value, b::Value)
    n = length(MapRef(a)) + length(MapRef(b))
    close(a)
    close(b)
    return n
end

struct Built <: Mirror end
show_me(::Built) = Value(Dict("built" => "here"))

struct Missing <: Pair end

struct Picky <: Greeter end
GreeterKind.greet(::Picky, name::String) = Dict("greeting" => name)
const PICKY_WHY = Ref{Union{Nothing,String}}("not on this machine")

mutable struct Closing <: Greeter
    closed::Bool
end
GreeterKind.greet(::Closing, name::String) = Dict("greeting" => name)
const CLOSED = Ref(0)
Base.close(c::Closing) = (CLOSED[] += 1; c.closed = true; nothing)
Guatiao.@config struct ClosingConfig
    label::String = "x"
end
Closing(::ClosingConfig) = Closing(false)

const REG = Registry("julia-provide", "0.1")

"The slot at `i` of a provider's table, the table's ctx."
function slot_of(key, kindname, i)
    table, _, ctx = Guatiao.provider_table(REG, key; kind = kindname)
    return unsafe_load(Ptr{Ptr{Cvoid}}(table + 8 + (i - 1) * 8)), ctx
end

@testset "a package is its library" begin
    lib = Guatiao.library_of(JlGreeter)
    @test lib.id == "JlGreeter"
    @test lib.version == "1.0.0"
    @test [p.name for p in lib.providers] == ["hello", "shouter"]
    @test lib.meta["authors"] == ["jose-pr <jose-pr@coqui.dev>"]
    @test lib.meta["license"] == "MPL-2.0"
    @test startswith(lib.meta["description"], "Greeters written in Julia")
    @test Guatiao.library_of(JlGreeter) === lib
    @test_throws ArgumentError Guatiao.library_of(Main)
    @test_throws ArgumentError Guatiao.library_of(@__MODULE__; version = "1.0")
    Guatiao.register_library(REG, JlGreeter)
    p = Guatiao.provider(REG, "JlGreeter_hello")
    @test p["display_name"] == "Hello, in Julia"
    @test sort(p["kinds"]) == ["counter", "greeter"]
    @test Guatiao.provider(REG, "JlGreeter_shouter")["has_config"] === true
    schema = Guatiao.provider_config(REG, "JlGreeter_shouter")
    @test schema["required"][] == ["prefix"]
    @test schema["properties"]["prefix"]["minLength"][] == 1
end

@testset "a provider is checked when its library is built" begin
    @test_throws ArgumentError Guatiao.Library("lib", "1"; providers = [Missing])
    @test_throws ArgumentError Guatiao.Provider(Ear)                  # serves no kind
    @test_throws ArgumentError Guatiao.Provider(Picky; serves = (Listener,))   # an object kind
    @test_throws ArgumentError Guatiao.Library("lib", "1"; providers = [Picky, Guatiao.Provider(Picky)])
end

@testset "what a provider answers lands in the caller's allocator" begin
    c = Counting()
    greet, ctx = slot_of("JlGreeter_hello", "greeter", 1)
    carrier = Ref(GMap(C_NULL, 0, 0, c.alloc))
    err = Ref(GProviderError(0, Guatiao.NO_STRING))
    name = "ana"
    status = GC.@preserve name carrier err ccall(greet, UInt32, (Ptr{Cvoid}, GStr, Ptr{GMap}, Ptr{GProviderError}), ctx, Guatiao.gstr(name), carrier, err)
    @test status == 0
    @test carrier[].alloc == c.alloc
    @test live(c) > 0
    v = Value(let r = new_root(); unsafe_store!(r, node_of(TAG_MAP, carrier[])); r end)
    @test v[] == Dict("greeting" => "hello, ana")
    close(v)
    @test live(c) == 0

    # A carrier naming no allocator: the answer is built in the binding's; a refusal carries its words.
    carrier = Ref(GMap(C_NULL, 0, 0, C_NULL))
    empty = ""
    status = GC.@preserve empty carrier err ccall(greet, UInt32, (Ptr{Cvoid}, GStr, Ptr{GMap}, Ptr{GProviderError}), ctx, Guatiao.gstr(empty), carrier, err)
    @test status == UInt32(Guatiao.BadValue)
    @test Guatiao.text_of(err[].message) == "nobody to greet"
    e = try Guatiao.finish_call(status, err, true, "greet") catch e; e end
    @test e.message == "nobody to greet"
end

@testset "a value built elsewhere is copied into the carrier's allocator" begin
    c = Counting()
    Guatiao.register_library(REG, Guatiao.Library("julia_mirror", "0.1.0"; providers = [Built]))
    show, ctx = slot_of("julia_mirror_built", "mirror", 1)
    carrier = Ref(GMap(C_NULL, 0, 0, c.alloc))
    @test GC.@preserve carrier ccall(show, UInt32, (Ptr{Cvoid}, Ptr{GMap}), ctx, carrier) == 0
    @test carrier[].alloc == c.alloc
    @test live(c) > 0
    v = Value(let r = new_root(); unsafe_store!(r, node_of(TAG_MAP, carrier[])); r end)
    @test v[] == Dict("built" => "here")
    close(v)
    @test live(c) == 0
end

@testset "a lent map grows in the allocator it records" begin
    c = Counting()
    config = Value(Dict("name" => "bo"); alloc = c.alloc)
    before = live(c)
    @test GreeterKind.check(Guatiao.bind(REG, "JlGreeter_hello", Greeter), config) === true
    @test live(c) > before
    @test config[] == Dict("name" => "bo", "greeting" => "hello, bo")
    close(config)
    @test live(c) == 0
end

@testset "a map handed over is freed, whatever the provider answers" begin
    c = Counting()
    g = Guatiao.bind(REG, "JlGreeter_hello", Greeter)
    config = Value(Dict("name" => "ana"); alloc = c.alloc)
    close(GreeterKind.greet_from(g, config))
    @test live(c) == 0
    refused = Value(Dict("name" => ""); alloc = c.alloc)
    @test_throws GuatiaoError GreeterKind.greet_from(g, refused)
    for _ in 1:3
        GC.gc(true)
    end
    @test live(c) == 0
end

@testset "a caller frees what it handed over when a later argument fails" begin
    c = Counting()
    Guatiao.register_library(REG, Guatiao.Library("julia_pair", "0.1.0"; providers = [Sizes]))
    p = Guatiao.bind(REG, "julia_pair_sizes", Pair)
    first = Value(Dict("a" => 1); alloc = c.alloc)
    @test_throws GuatiaoError both(p, first, [1])
    @test Guatiao.isabsent(first)
    @test live(c) == 0
    again = Value(Dict("a" => 1); alloc = c.alloc)
    @test both(p, again, Dict("b" => 2, "c" => 3)) == 3
    @test live(c) == 0
end

@testset "an instance per configuration, closed when its host destroys it" begin
    lib = Guatiao.Library("julia_closing", "0.1.0"; providers = [Guatiao.Provider(Closing; config = ClosingConfig)])
    Guatiao.register_library(REG, lib)
    @test_throws GuatiaoError Guatiao.create(REG, "julia_closing_closing", Dict("label" => 1))
    inst = Guatiao.create(REG, "julia_closing_closing", Dict())
    @test Guatiao.live_instances(lib) == 1
    e = try Guatiao.unload!(REG, "julia_closing") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.WrongKind   # linked, not mapped
    @test Guatiao.library_unload(lib.slot) == UInt32(Guatiao.Busy)
    before = CLOSED[]
    close(inst)
    @test CLOSED[] == before + 1
    @test Guatiao.live_instances(lib) == 0
    @test Guatiao.library_unload(lib.slot) == 0
    Guatiao.retire!(REG, "julia_closing")
    @test !any(p -> p["id"] == "julia_closing_closing", Guatiao.providers(REG))
end

@testset "create builds nothing when it has nowhere to put the instance" begin
    lib = Guatiao.library_of(JlGreeter)
    j = findfirst(p -> p !== nothing && p.library === lib && p.spec.name == "shouter", Guatiao.PROVIDER_SLOTS)
    before = Guatiao.live_objects()
    config = Value(Dict("prefix" => "hey"))
    err = Ref(GProviderError(0, Guatiao.NO_STRING))
    status = GC.@preserve config err Guatiao.provider_create(j, C_NULL, Guatiao.root_node(config), Ptr{Ptr{Cvoid}}(C_NULL),
        Base.unsafe_convert(Ptr{GProviderError}, err))
    @test status == UInt32(Guatiao.NullArgument)
    @test Guatiao.live_objects() == before
    close(config)
end

@testset "a provider says why it cannot run here" begin
    Guatiao.register_library(REG, Guatiao.Library("julia_picky", "0.1.0";
        providers = [Guatiao.Provider(Picky; available = () -> PICKY_WHY[])]))
    @test Guatiao.provider_available(REG, "julia_picky_picky") == (false, "not on this machine")
    @test isempty(filter(p -> p["id"] == "julia_picky_picky", Guatiao.available(REG, "greeter")))
    PICKY_WHY[] = nothing
    @test Guatiao.provider_available(REG, "julia_picky_picky") == (true, nothing)
end

@testset "a closed library declines a host" begin
    lib = Guatiao.Library("julia_declines", "0.1.0"; providers = [Picky])
    e = Guatiao.entry(lib)
    close(lib)
    @test ccall(e, Ptr{Cvoid}, (Ptr{Cvoid},), C_NULL) == C_NULL
    Guatiao.register_entry(REG, "julia_declines", e)
    @test !any(p -> p["library"] == "julia_declines", Guatiao.providers(REG))
end

@testset "a slot called from threads Julia did not start" begin
    fixture = build_c(joinpath(@__DIR__, "fixtures", "foreign_threads.c"), scratch("foreign"), "foreign_threads")
    if fixture === nothing
        absent_greeter(:foreign_thread_fixture, "no C compiler: the fixture is not built")
    else
        lib = Libdl.dlopen(fixture)
        table, _, ctx = Guatiao.provider_table(REG, "JlGreeter_hello"; kind = "greeter")
        free = Libdl.dlsym(Libdl.dlopen(Guatiao.native_lib("guatiao").path), :guatiao_value_free)
        joined = Libdl.dlsym(lib, :foreign_greet_joined)
        name, expect = "ana", "hello, ana"
        # The caller waits inside a GC-safe region: the provider's code
        # allocates on eight foreign threads, and a collection one of them
        # starts must not wait for this thread.
        failures = GC.@preserve name expect begin
            gs = Guatiao.gc_safe_enter()
            n = ccall(joined, Cint, (Ptr{Cvoid}, Ptr{Cvoid}, Ptr{Cvoid}, Cstring, Cstring, Cint, Cint),
                table, ctx, free, name, expect, 8, 500)
            Guatiao.gc_safe_leave(gs)
            n
        end
        @test failures == 0
        detached = Libdl.dlsym(lib, :foreign_greet_detached)
        @test ccall(detached, Cint, (Ptr{Cvoid}, Ptr{Cvoid}, Ptr{Cvoid}, Cstring, Cstring), table, ctx, free, name, expect) == 1
        done = Libdl.dlsym(lib, :foreign_done)
        for _ in 1:500
            ccall(done, Cint, ()) == 1 && break
            sleep(0.01)
        end
        @test ccall(done, Cint, ()) == 1
        @test ccall(Libdl.dlsym(lib, :foreign_failures), Cint, ()) == 0
    end
end
