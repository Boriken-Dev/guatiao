# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A plugin host written in Julia.
#
# Scans the directories it is given for libraries declaring the greeter
# kind, whatever language wrote them, and calls each greeter through the
# kind's table: a map back, a map lent and changed in place, a map handed
# over, an object handed in and one handed back, an object lent for one
# call. A provider that builds instances from a configuration is given
# one its schema refuses, then one it accepts. The greeters written in
# Julia are registered beside them, and the greeters written in Python
# are served by the guatiao-source-py bridge when one is named. Everything it
# opened is closed before it exits, and it prints what it did.
#
#   julia examples/julia_host/host.jl DIR [DIR ...] [--python-source BRIDGE --python-dir DIR ... --python-path DIR ...]

# The binding and the example packages, found by name from this checkout.
let examples = dirname(@__DIR__)
    for dir in (joinpath(examples, "..", "bindings", "julia", "Guatiao"), joinpath(examples, "greeter_kind", "julia"),
        joinpath(examples, "julia_provider"))
        push!(LOAD_PATH, normpath(dir))
    end
end

using Guatiao
using GreeterKind
using JlGreeter

"A listener: what a conversation tells, kept."
mutable struct Ear
    heard::Vector{String}
    closed::Bool
end
Ear() = Ear(String[], false)
GreeterKind.heard(e::Ear, what::String) = (push!(e.heard, what); nothing)
Base.close(e::Ear) = (e.closed = true; nothing)

function exercise(io, g, say)
    answer = GreeterKind.greet(g, "ana")
    say("greet: ", answer["greeting"][])
    close(answer)
    try
        close(GreeterKind.greet(g, ""))
    catch e
        e isa GuatiaoError || rethrow()
        say("greet(\"\") refused: ", e.status, ": ", e.message)
    end
    # The other slots are optional: a provider may leave them out.
    missing = [String(s) for s in (:shout, :check, :greet_from, :greet_into, :start) if !Guatiao.has(g, s)]
    isempty(missing) || say("a table leaving out ", join(missing, ", "))
    Guatiao.has(g, :shout) && say("shout: ", GreeterKind.shout(g, "ana"))
    if Guatiao.has(g, :check)
        config = Value(Dict("name" => "bo"))
        ok = GreeterKind.check(g, config)
        say("check lent a map and it came back ", ok ? "accepted" : "refused", ": ", join(sort(collect(keys(MapRef(config)))), ", "))
        close(config)
    end
    if Guatiao.has(g, :greet_from)
        handed = Value(Dict("name" => "cy"))
        answer = GreeterKind.greet_from(g, handed)
        say("greet_from took a map: ", answer["greeting"][])
        close(answer)
        close(handed)
    end
    if Guatiao.has(g, :greet_into)
        ear = Ear()
        GreeterKind.greet_into(g, "di", ear)
        say("greet_into told a listener: ", join(ear.heard, ", "))
    end
    if Guatiao.has(g, :start)
        listener = Ear()
        chat = try
            GreeterKind.start(g, listener)
        catch e
            e isa GuatiaoError || rethrow()
            say("start refused: ", e.message)
            return
        end
        GreeterKind.say(chat, "hi")
        GreeterKind.say(chat, "again")
        buf = zeros(UInt8, 64)
        n = GreeterKind.read(chat, buf)
        say("a conversation of ", GreeterKind.turns(chat), " turns: ", repr(String(buf[1:n])))
        close(chat)
        say("closed it; the listener heard ", join(listener.heard, ", "), listener.closed ? " and was let go" : "")
    end
end

function main(args = ARGS; io = stdout)
    dirs = String[]
    python_source = nothing
    python_dirs = String[]
    python_paths = String[]
    i = 1
    while i <= length(args)
        a = args[i]
        if a == "--python-source"
            python_source = args[i+1]; i += 2
        elseif a == "--python-dir"
            push!(python_dirs, args[i+1]); i += 2
        elseif a == "--python-path"
            push!(python_paths, args[i+1]); i += 2
        else
            push!(dirs, a); i += 1
        end
    end
    reg = Registry("julia-host", "1.0")
    lib = nothing
    try
        for dir in dirs
            report = Guatiao.scan_dir(reg, dir; rules = "kind=greeter")
            for file in report["loaded"]
                println(io, "loaded ", basename(file))
            end
        end
        lib = Guatiao.library_of(JlGreeter)
        Guatiao.register_library(reg, lib)
        println(io, "registered ", lib.id, " ", lib.version, " from Julia")
        if python_source !== nothing
            # The bridge imports python3.dll from the Python installation, on PATH.
            Guatiao.set_import_search!(reg, :path)
            Guatiao.add_source_file!(reg, python_source, Dict("dirs" => python_dirs, "paths" => python_paths))
            changed = Guatiao.refresh!(reg)
            for served in changed["loaded"]
                println(io, "served ", served["id"], " from Python")
            end
            for refused in changed["refused"]
                println(io, "refused ", get(refused, "id", "?"), ": ", refused["why"])
            end
        end

        for p in Guatiao.providers(reg, "greeter")
            key = p["key"]
            println(io, "\n", key, " (", p["library"], " ", p["version"], ")")
            say(parts...) = println(io, "  ", parts...)
            g = try
                Guatiao.bind(reg, key, Greeter)
            catch e
                e isa FloorMismatch || rethrow()
                say("not this greeter kind: ", e.message)
                continue
            end
            if p["has_config"] !== true
                exercise(io, g, say)
                continue
            end
            # Built from a configuration: one the schema refuses, then one it accepts.
            try
                close(Guatiao.create(reg, key, Dict()))
            catch e
                e isa GuatiaoError || rethrow()
                say("an empty configuration is refused: ", e.message)
            end
            inst = Guatiao.create(reg, key, Dict("prefix" => "hey"))
            say("instance from prefix = hey")
            exercise(io, Guatiao.bind(inst, Greeter), say)
            close(inst)
        end

        # Unmapped where the library agrees; one with no say in it is retired, and stays mapped.
        println(io)
        for l in Guatiao.libraries(reg)
            key = l["key"]
            try
                Guatiao.unload!(reg, key)
                println(io, "unloaded ", key)
            catch e
                e isa GuatiaoError || rethrow()
                Guatiao.retire!(reg, key)
                println(io, "retired ", key, ", still mapped: unload said ", e.status)
            end
        end
    finally
        close(reg)
        # What this host built for itself, once no registry can call it.
        lib === nothing || close(lib)
    end
    println(io, "objects a library can still reach: ", Guatiao.live_objects())
    return nothing
end

if abspath(PROGRAM_FILE) == @__FILE__
    main()
end
