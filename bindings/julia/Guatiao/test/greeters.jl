# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# One behaviour, four languages: a Julia host calls the greeters written in
# Rust, C, Python and Julia through one description of the kind, and a
# Rust caller calls each of them through the kind's Rust proxy.

using GreeterKind, JlGreeter
using Libdl

const GREETERS = [
    (:rust, "derived_greeter_hello", "hello, ana"),
    (:c, "c_greeter", "hello from C, ana"),
    (:python, "py_greeter_hello", "hello, ana"),
    (:go, "go_greeter_hello", "hello, ana"),
    (:julia, "JlGreeter_hello", "hello, ana"),
]

"An absent greeter is a skip, or a failure when `GUATIAO_REQUIRE_ALL` is 1 (CI)."
function absent_greeter(which, why)
    if get(ENV, "GUATIAO_REQUIRE_ALL", "") == "1"
        @error "the $which greeter is required here: $why"
        @test false
    else
        @info "skipping the $which greeter: $why"
        @test_skip false
    end
end

mutable struct Ear
    heard::Vector{String}
    closed::Bool
end
Ear() = Ear(String[], false)
GreeterKind.heard(e::Ear, what::String) = (push!(e.heard, what); nothing)
Base.close(e::Ear) = (e.closed = true; nothing)

@testset "the kinds copy their floor hashes from the header" begin
    header = read(GREETER_KIND_H, String)
    for (table, K) in (("greeter_vtable", Greeter), ("counter_vtable", Counter),
        ("conversation_vtable", Conversation), ("listener_vtable", Listener), ("wide_vtable", Wide))
        found = match(Regex("#define $(table)_FLOOR_HASH (\\d+)"), header)
        @test parse(UInt32, found.captures[1]) == Guatiao.kind(K).floor_hash
        body = match(Regex("typedef struct $table \\{(.*?)\\} $table;", "s"), header).captures[1]
        @test Guatiao.kind(K).size == 8 + 8 * length(collect(eachmatch(r"\(\*\w+\)", body)))
    end
end

const PRESENT = Set{Symbol}()
const HOST = Registry("julia-greeters", "0.1")
# Every greeter written in another language comes through a source the
# registry keeps, and one refresh loads them all: the workspace's own
# directory and the C and Go greeters' as watched directories, the Python
# greeters through the guatiao-py bridge.
Guatiao.watch_dir!(HOST, library_dir(); rules = "kind=greeter")
push!(PRESENT, :rust)
let dir = build_c_greeter()
    if dir === nothing
        absent_greeter(:c, "no C compiler: the greeter written in C is not built")
    else
        Guatiao.watch_dir!(HOST, dir; rules = "kind=greeter")
        push!(PRESENT, :c)
    end
end
let dir = get(ENV, "GUATIAO_GO_LIBRARY", "")
    if isempty(dir)
        absent_greeter(:go, "GUATIAO_GO_LIBRARY names no directory holding the Go greeter built with -buildmode=c-shared")
    else
        Guatiao.watch_dir!(HOST, dir; rules = "kind=greeter")
        push!(PRESENT, :go)
    end
end
let py = bridge()
    named = haskey(ENV, "GUATIAO_PY_BRIDGE")
    if py === nothing
        absent_greeter(:python, "no guatiao-py build: set GUATIAO_PY_BRIDGE")
    else
        try
            Guatiao.add_source_file!(HOST, py, bridge_config())
            push!(PRESENT, :python)
        catch e
            named && rethrow()
            absent_greeter(:python, "the bridge at $py does not map here: $(sprint(showerror, e))")
        end
    end
end
const REFRESHED = Guatiao.refresh!(HOST)
@testset "one refresh loads every greeter its sources serve" begin
    @test isempty(REFRESHED["refused"])
    loaded = Set(s["key"] for s in REFRESHED["loaded"])
    @test "derived_greeter" in loaded
    :c in PRESENT && @test "c_greeter_library" in loaded
    :python in PRESENT && @test "py_greeter" in loaded
    :go in PRESENT && @test any(id -> endswith(id, "go_greeter"), loaded)
end
Guatiao.register_library(HOST, JlGreeter)
push!(PRESENT, :julia)

@testset "a Julia host calls every greeter: $which" for (which, key, greeting) in GREETERS
    which in PRESENT || continue
    g = Guatiao.bind(HOST, key, Greeter)
    @test g isa Greeter
    answer = GreeterKind.greet(g, "ana")
    @test answer[] == Dict("greeting" => greeting)
    close(answer)

    e = try GreeterKind.greet(g, "") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.BadValue && occursin("nobody to greet", e.message)

    if which === :c
        close(GreeterKind.greet(g, "panic"))   # the greeter written in C greets anyone
    else
        e = try GreeterKind.greet(g, "panic") catch e; e end
        @test e isa GuatiaoError && e.status == Guatiao.Internal
    end

    config = Value(Dict("name" => "bo"))
    @test GreeterKind.check(g, config) === true
    @test config["name"][] == "bo"
    @test length(MapRef(config)) == 2
    close(config)

    handed = Value(Dict("name" => "ana"))
    answer = GreeterKind.greet_from(g, handed)
    @test answer[] == Dict("greeting" => greeting)
    @test Guatiao.isabsent(handed)
    close(answer)
    close(handed)

    ear = Ear()
    GreeterKind.greet_into(g, "ana", ear)
    @test ear.heard == [greeting]

    if Guatiao.has(g, :shout)
        @test GreeterKind.shout(g, "ana") == uppercase(greeting)
    else
        @test_throws GuatiaoError GreeterKind.shout(g, "ana")
    end

    if Guatiao.has(g, :start)
        listener = Ear()
        chat = GreeterKind.start(g, listener)
        GreeterKind.say(chat, "hi")
        GreeterKind.say(chat, "again")
        e = try GreeterKind.say(chat, "") catch e; e end
        @test e isa GuatiaoError && e.status == Guatiao.BadValue
        buf = zeros(UInt8, 64)
        n = GreeterKind.read(chat, buf)
        @test String(buf[1:n]) == "hi\nagain"
        @test GreeterKind.turns(chat) == 2
        @test listener.heard == ["hi", "again"]
        close(chat)
        @test listener.closed
    else
        @test which === :c
    end
end

# Each of these unloads a library of its own: no other registry keeps it mapped.
@testset "a tree built in the library's allocator outlives the unload" begin
    @testset "an answer from a greeter that ignores the carrier" begin
        dir = build_c_greeter(; ignores_carrier = true)
        if dir === nothing
            absent_greeter(:c, "no C compiler: the greeter written in C is not built")
        else
            own = Registry("julia-careless", "0.1")
            Guatiao.scan_dir(own, dir; rules = "kind=greeter")
            answer = GreeterKind.greet(Guatiao.bind(own, "c_greeter", Greeter), "hal")
            # The C greeter has no `unload` function: nothing it handed out is
            # alive, which the caller states.
            Guatiao.unload!(own, "c_greeter_library"; unchecked = true)
            @test isempty(Guatiao.libraries(own))
            @test answer[] == Dict("greeting" => "hello from C, hal")
            close(answer)
            close(own)
        end
    end
    @testset "what a slot grew a lent map with" begin
        dir = build_c_greeter()
        if dir === nothing
            absent_greeter(:c, "no C compiler: the greeter written in C is not built")
        else
            own = Registry("julia-lent", "0.1")
            Guatiao.scan_dir(own, dir; rules = "kind=greeter")
            config = Value(Dict("name" => "ivy"))
            @test GreeterKind.check(Guatiao.bind(own, "c_greeter", Greeter), config) === true
            Guatiao.unload!(own, "c_greeter_library"; unchecked = true)
            @test config[] == Dict("name" => "ivy", "checked_by" => "C")
            close(config)
            close(own)
        end
    end
end

@testset "the Rust proxy calls every greeter: $which" for (which, key, greeting) in GREETERS
    which in PRESENT || continue
    caller = Libdl.dlopen(workspace_library("greeter_caller"))
    run = Libdl.dlsym(caller, :greeter_caller_run)
    out = Guatiao.new_root()
    host = Guatiao.host(HOST)
    k, n = key, "ana"
    status = GC.@preserve k n begin
        gs = Guatiao.gc_safe_enter()
        s = ccall(run, UInt32, (Ptr{Cvoid}, Guatiao.GStr, Guatiao.GStr, Ptr{Guatiao.GValue}), host, Guatiao.gstr(k), Guatiao.gstr(n), out)
        Guatiao.gc_safe_leave(gs)
        s
    end
    @test status == 0
    report = Value(out)[]
    @test report["greet"] == greeting
    @test report["refusal"]["status"] == Int(Guatiao.BadValue)
    @test report["failure"] == (which === :c ? 0 : Int(Guatiao.Internal))
    @test report["shout"] == uppercase(greeting)
    @test report["checked"] === true
    @test length(report["filled"]) == 2
    @test report["greet_from"] == greeting
    @test report["heard"] == [greeting]
    conversation = report["conversation"]
    if which === :c
        # No `start` in its table: the Rust proxy runs the kind's default.
        @test conversation == Dict("status" => Int(Guatiao.NullArgument), "message" => "this greeter holds no conversations")
    else
        @test conversation["turns"] == 2
        @test conversation["transcript"] == "hi\nagain"
        @test conversation["heard"] == ["hi", "again"]
        @test conversation["dropped"] === true
    end
end

@testset "a table that is not the kind's is refused before it runs" begin
    table, size, ctx = Guatiao.provider_table(HOST, "JlGreeter_hello"; kind = "counter")
    @test_throws FloorMismatch Guatiao.bind(Greeter, table, size, ctx)
    @test_throws GuatiaoError Guatiao.bind(HOST, "c_greeter", Counter)
end

@testset "nothing a Julia greeter served is left once its holders are done" begin
    before = Guatiao.live_objects()
    g = Guatiao.bind(HOST, "JlGreeter_hello", Greeter)
    chat = GreeterKind.start(g, Ear())
    @test Guatiao.live_objects() == before + 2   # the conversation, and the listener it holds
    close(chat)
    @test Guatiao.live_objects() == before
end

# ---- 64-bit integers cross a slot exactly --------------------------------------------

const SIGNED = Int64[Int64(2)^53 + 1, typemin(Int64), typemax(Int64)]
const UNSIGNED = UInt64[UInt64(2)^53 + 1, UInt64(2)^63, typemax(UInt64)]

"Answers what it is handed, and refuses 7."
struct Echo <: Wide end
function GreeterKind.echo_i64(::Echo, x::Int64)
    x == 7 && throw(GuatiaoError(Guatiao.BadValue, "seven is no number"))
    return x
end
function GreeterKind.echo_u64(::Echo, x::UInt64)
    x == 7 && throw(GuatiaoError(Guatiao.BadValue, "seven is no number"))
    return x
end

"Answers a number its slot cannot hold."
struct Overshoot <: Wide end
GreeterKind.echo_i64(::Overshoot, x::Int64) = big(x) + 1
GreeterKind.echo_u64(::Overshoot, x::UInt64) = -1

Guatiao.register_library(HOST, Guatiao.Library("julia_wide", "0.1.0"; providers = [Echo, Overshoot]))

const WIDE = [
    (:rust, "derived_greeter_hello"),
    (:c, "c_greeter"),
    (:julia, "julia_wide_echo"),
]

@testset "a slot carries 64-bit integers exactly: $which" for (which, key) in WIDE
    which in PRESENT || which === :julia || continue
    w = Guatiao.bind(HOST, key, Wide)
    for x in SIGNED
        @test GreeterKind.echo_i64(w, x) === x
    end
    for x in UNSIGNED
        @test GreeterKind.echo_u64(w, x) === x
    end
    # A number that is whole converts; what does not fit or is no number is refused.
    @test GreeterKind.echo_i64(w, 5.0) === Int64(5)
    @test GreeterKind.echo_u64(w, 2^53) === UInt64(2)^53
    for bad in (UInt64(2)^63, big(2)^63, big(-2)^63 - 1, 1.5, 9.3e18)
        @test_throws InexactError GreeterKind.echo_i64(w, bad)
    end
    for bad in (-1, big(2)^64, big(-1), 3.0e19, 1.5)
        @test_throws InexactError GreeterKind.echo_u64(w, bad)
    end
    @test_throws MethodError GreeterKind.echo_i64(w, "5")
    @test_throws MethodError GreeterKind.echo_u64(w, nothing)
end

@testset "a Julia provider refuses with its own words, and fails a call whose answer does not fit" begin
    w = Guatiao.bind(HOST, "julia_wide_echo", Wide)
    for call in (() -> GreeterKind.echo_i64(w, 7), () -> GreeterKind.echo_u64(w, 7))
        e = try call() catch e; e end
        @test e isa GuatiaoError && e.status == Guatiao.BadValue && occursin("seven is no number", e.message)
    end
    o = Guatiao.bind(HOST, "julia_wide_overshoot", Wide)
    @test GreeterKind.echo_i64(o, Int64(5)) == 6
    for call in (() -> GreeterKind.echo_i64(o, typemax(Int64)), () -> GreeterKind.echo_u64(o, UInt64(0)))
        e = try call() catch e; e end
        @test e isa GuatiaoError && e.status == Guatiao.Internal
    end
end

@testset "the Rust proxy calls a wide provider exactly: $which" for (which, key) in WIDE
    which in PRESENT || which === :julia || continue
    caller = Libdl.dlopen(workspace_library("greeter_caller"))
    wide = Libdl.dlsym(caller, :greeter_caller_wide)
    out = Guatiao.new_root()
    host = Guatiao.host(HOST)
    status = GC.@preserve key begin
        gs = Guatiao.gc_safe_enter()
        s = ccall(wide, UInt32, (Ptr{Cvoid}, Guatiao.GStr, Ptr{Guatiao.GValue}), host, Guatiao.gstr(key), out)
        Guatiao.gc_safe_leave(gs)
        s
    end
    @test status == 0
    report = Value(out)[]
    @test report["i64"] == SIGNED
    @test report["u64"] == UNSIGNED
end
