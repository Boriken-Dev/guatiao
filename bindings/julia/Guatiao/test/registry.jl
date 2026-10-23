# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The registry against a real build: scanning, answers, instances, retire,
# collect and unload, and sources.

using GreeterKind

const TARGET = library_dir()

function scanned()
    reg = Registry("julia-tests", "0.1")
    Guatiao.scan_dir(reg, TARGET; rules = "kind=greeter")
    return reg
end
ids(list) = [p["id"] for p in list]

@testset "scans by rule and lists the providers" begin
    Registry("julia-tests", "0.1") do reg
        report = Guatiao.scan_dir(reg, TARGET; rules = "kind=greeter")
        @test any(p -> occursin("derived_greeter", p), report["loaded"])
        @test any(p -> occursin("hello_library", p), report["loaded"])
        @test "derived_greeter_hello" in ids(Guatiao.providers(reg, "greeter"))
        @test "hello_library_greeter" in ids(Guatiao.providers(reg, "greeter"))
    end
end

@testset "answers the documented fields for the best provider" begin
    reg = scanned()
    best = Guatiao.best(reg, "greeter")
    for field in ("key", "id", "version", "library", "display_name", "from", "kinds", "has_config", "vtable_size")
        @test haskey(best, field)
    end
    @test "greeter" in best["kinds"]
    why = Guatiao.why_not(reg, "nothing-claims-this-kind")
    @test why["available"] === false && why["why"] == "nothing-claims-it"
    e = try Guatiao.provider(reg, "nonesuch") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.NotFound
    close(reg)
end

@testset "answers a single table, and a table per kind" begin
    reg = scanned()
    table, size, _ = Guatiao.provider_table(reg, "hello_library_greeter")
    @test table != C_NULL && size > 0
    @test Guatiao.provider_table(reg, "derived_greeter_hello")[1] == C_NULL
    @test Guatiao.provider_table(reg, "derived_greeter_hello"; kind = "greeter")[1] != C_NULL
    @test Guatiao.provider_table(reg, "derived_greeter_hello"; kind = "codec")[1] == C_NULL
    @test GreeterKind.count(Guatiao.bind(reg, "derived_greeter_hello", Counter)) isa Int64
    close(reg)
end

@testset "builds an instance, and lends its configuration schema" begin
    reg = scanned()
    schema = Guatiao.provider_config(reg, "derived_greeter_shouter")
    @test Guatiao.tag(schema) === :map
    @test_throws ArgumentError (schema["x"] = 1)
    @test Guatiao.provider_config(reg, "derived_greeter_hello") === nothing
    inst = Guatiao.create(reg, "derived_greeter_shouter", Dict("prefix" => "hey"))
    @test inst.ctx != C_NULL
    g = Guatiao.bind(inst, Greeter)
    @test startswith(GreeterKind.greet(g, "ana")["greeting"][], "hey")
    close(inst)
    close(inst)
    @test_throws InvalidStateException GreeterKind.greet(g, "ana")
    close(reg)
    @test_throws InvalidStateException schema[]
end

@testset "keys, ranks and re-keys" begin
    reg = scanned()
    Guatiao.set_priority!(reg, "derived_greeter_hello", 7)
    @test Guatiao.priority(reg, "derived_greeter_hello") == 7
    Guatiao.keyed_by!(reg, "%id@%version")
    @test any(p -> startswith(p["key"], "derived_greeter_hello@"), Guatiao.providers(reg, "greeter"))
    @test_throws GuatiaoError Guatiao.keyed_by!(reg, "%nonsense")
    close(reg)
end

@testset "retires a library and frees its key" begin
    reg = scanned()
    Guatiao.retire!(reg, "derived_greeter")
    @test !("derived_greeter_hello" in ids(Guatiao.providers(reg, "greeter")))
    @test "hello_library_greeter" in ids(Guatiao.providers(reg, "greeter"))
    again = Guatiao.scan_dir(reg, TARGET; rules = "kind=greeter")
    @test any(p -> occursin("derived_greeter", p), again["loaded"])
    e = try Guatiao.retire!(reg, "derived_greeter_hello") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.NotFound
    close(reg)
end

@testset "unloads only when the library agrees" begin
    reg = scanned()
    inst = Guatiao.create(reg, "derived_greeter_shouter", Dict("prefix" => "hey"))
    e = try Guatiao.unload!(reg, "derived_greeter") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.Busy
    close(inst)
    Guatiao.unload!(reg, "derived_greeter")
    @test !("derived_greeter" in ids(Guatiao.libraries(reg)))
    e = try Guatiao.unload!(reg, "derived_greeter") catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.NotFound
    close(reg)
end

@testset "collects a retired library once nothing of it is alive" begin
    reg = scanned()
    @test Guatiao.collect!(reg) == []
    inst = Guatiao.create(reg, "derived_greeter_shouter", Dict("prefix" => "hey"))
    Guatiao.retire!(reg, "derived_greeter")
    @test Guatiao.collect!(reg) == []
    close(inst)
    @test Guatiao.collect!(reg) == ["derived_greeter"]
    @test Guatiao.collect!(reg) == []
    close(reg)
end

@testset "refuses use once closed, and closes twice quietly" begin
    reg = Registry("julia-tests", "0.1")
    close(reg)
    @test_throws InvalidStateException Guatiao.libraries(reg)
    close(reg)
    @test !isopen(reg)
end

@testset "serves libraries that come and go from a source file" begin
    shelf = build_c_source("julia_shelf_a")
    if shelf === nothing
        absent_greeter(:c_source, "no C compiler: the source written in C is not built")
    else
        controls = Libdl.dlopen(shelf)
        take = Libdl.dlsym(controls, :shelf_take)
        put = Libdl.dlsym(controls, :shelf_put)
        refuse = Libdl.dlsym(controls, :shelf_refuse)
        closed = Libdl.dlsym(controls, :shelf_closed)
        Registry("julia-sources", "0.1") do reg
            name = Guatiao.add_source_file!(reg, shelf, Dict("ids" => ["shelf_a", "shelf_b"]))
            @test name == shelf
            @test Guatiao.libraries(reg) == []
            first = Guatiao.refresh!(reg)
            @test [s["id"] for s in first["loaded"]] == ["shelf_a", "shelf_b"]
            @test GreeterKind.count(Guatiao.bind(reg, "shelf_a_counter", Counter)) == 1
            @test Guatiao.refresh!(reg) == Dict("loaded" => [], "retired" => [], "skipped" => [], "refused" => [])
            ccall(take, Cvoid, (Cint,), 1)
            @test [s["id"] for s in Guatiao.refresh!(reg)["retired"]] == ["shelf_b"]
            Guatiao.collect!(reg)
            @test ccall(closed, Cint, (Cint,), 1) == 1
            ccall(refuse, Cvoid, (Bool,), true)
            ccall(put, Cvoid, (Cint,), 2)
            @test Guatiao.refresh!(reg)["refused"] == [Dict("source" => name, "id" => "shelf_c", "why" => "the shelf refuses, says C")]
        end
    end
end

@testset "refuses with the source's own words, and a file that is no source" begin
    shelf = build_c_source("julia_shelf_b")
    if shelf === nothing
        absent_greeter(:c_source, "no C compiler: the source written in C is not built")
    else
        Registry("julia-sources", "0.1") do reg
            e = try Guatiao.add_source_file!(reg, shelf, Dict()) catch e; e end
            @test e isa GuatiaoError && occursin("`ids`", e.message)
            Guatiao.add_source_file!(reg, shelf, Dict("ids" => String[]))
            e = try Guatiao.add_source_file!(reg, shelf, Dict("ids" => String[])) catch e; e end
            @test occursin("already added", e.message)
            e = try Guatiao.add_source_file!(reg, workspace_library("derived_greeter")) catch e; e end
            @test e.status == Guatiao.WrongKind
            e = try Guatiao.add_source_file!(reg, joinpath(TARGET, "no-such-file")) catch e; e end
            @test e.status == Guatiao.NotFound
        end
    end
end

@testset "follows the files of a watched directory" begin
    dir = scratch("watched")
    watched = mkpath(joinpath(dir, "watched"))
    aside = mkpath(joinpath(dir, "aside"))
    Registry("julia-sources", "0.1") do reg
        Guatiao.watch_dir!(reg, watched; rules = "kind=greeter")
        @test_throws GuatiaoError Guatiao.watch_dir!(reg, watched)
        @test Guatiao.refresh!(reg)["loaded"] == []
        copy = joinpath(watched, Guatiao.library_filename("hello_julia_watched"))
        cp(workspace_library("hello_library"), copy)
        @test [s["key"] for s in Guatiao.refresh!(reg)["loaded"]] == ["hello_library"]
        # A mapped image cannot be deleted on Windows; moved aside, it is gone.
        mv(copy, joinpath(aside, basename(copy)))
        @test [s["key"] for s in Guatiao.refresh!(reg)["retired"]] == ["hello_library"]
        @test Guatiao.libraries(reg) == []
    end
end
