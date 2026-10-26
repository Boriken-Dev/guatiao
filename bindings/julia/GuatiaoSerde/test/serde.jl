# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

@testset "JSON keeps key order and exact numbers" begin
    text = """{"z":1,"a":[1,2.50,"x"],"big":123456789012345678901234567890.5}"""
    v = Serde.parse(text)
    @test keys(MapRef(v)) == ["z", "a", "big"]
    @test Serde.emit(v) == text
    @test length(Serde.emit(v; pretty = true)) > length(text)
    close(v)
end

@testset "TOML and YAML round-trip" begin
    if Guatiao.exports("guatiao_serde", "guatiao_toml_emit")
        v = Serde.parse("name = \"ana\"\nn = 3\n"; format = :toml)
        @test v[] == Dict("name" => "ana", "n" => 3)
        @test occursin("name = \"ana\"", Serde.emit(v; format = :toml))
        @test_throws GuatiaoError Serde.emit(Value([1]); format = :toml)
        close(v)
    end
    if Guatiao.exports("guatiao_serde", "guatiao_yaml_emit")
        v = Serde.parse("a: [1, 2]\n"; format = :yaml)
        @test v[] == Dict("a" => [1, 2])
        @test Serde.parse(Serde.emit(v; format = :yaml); format = :yaml)[] == v[]
        close(v)
    end
end

@testset "a malformed document names its format and position" begin
    e = try
        Serde.parse("{\n  \"a\": }")
    catch e
        e
    end
    @test e isa ParseError
    @test e.format === :json
    @test e.line == 2
    @test e.column > 0
    @test !isempty(e.message)
    @test Guatiao.status(e) == Guatiao.BadValue
    @test_throws ArgumentError Serde.parse("{}"; format = :xml)
end

@testset "plain data emits too" begin
    @test Serde.emit(Dict("a" => [1, nothing])) == """{"a":[1,null]}"""
end

using Guatiao.Dates: Date, DateTime, Second

@enum Colour red green
struct Point
    x::Int
    label::Union{Nothing,String}
end

@testset "parse(Any, ...) reads what parse(...)[] reads" begin
    docs = [(:json, """{"z":1,"a":[1,2.50,"x",null,true],"big":123456789012345678901234567890.5,"o":{}}"""),
        (:toml, "name = \"ana\"\nn = 3\n[t]\nx = [1.5, 2]\n"), (:yaml, "a: [1, 2]\nb: {c: text}\n")]
    for (format, text) in docs
        format === :json || Guatiao.exports("guatiao_serde", "guatiao_$(format)_emit") || continue
        v = Serde.parse(text; format)
        @test isequal(Serde.parse(Any, text; format), v[])
        close(v)
    end
    for (format, text) in [(:json, "{\"a\":\n 1,,}"), (:toml, "a = = 1")]
        format === :json || Guatiao.exports("guatiao_serde", "guatiao_$(format)_emit") || continue
        e1 = try Serde.parse(text; format); nothing catch e; e end
        e2 = try Serde.parse(Any, text; format); nothing catch e; e end
        @test e2 isa ParseError && e1.line == e2.line && e1.column == e2.column && e1.message == e2.message
    end
    @test_throws ArgumentError Serde.parse(Any, "{}"; format = :xml)
end

@testset "emit of plain data writes what emit of its Value writes" begin
    when = DateTime(2026, 10, 26, 13, 45)
    kept = Value(Dict("k" => [1, 2]))
    data = Dict{String,Any}("s" => "ü€", "n" => NumberText("1.10"), "i" => -3, "f" => 0.1, "b" => true,
        "none" => nothing, "bytes" => UInt8[1, 2], "sym" => :tag, "c" => 'x', "tuple" => (1, "two"),
        "set" => Set([3]), "nt" => (a = 1, b = [2.5]), "colour" => green, "point" => Point(1, nothing),
        "date" => Date(2026, 10, 26), "when" => when, "length" => Second(90), "ref" => kept,
        "ints" => Dict(1 => "a", 2 => "b"), "clash" => Dict{Any,Int}(1 => 1, "1" => 2))
    for format in (:json, :yaml), pretty in (false, true)
        format === :json || Guatiao.exports("guatiao_serde", "guatiao_yaml_emit") || continue
        v = Value(data)
        @test Serde.emit(data; format, pretty) == Serde.emit(v; format, pretty)
        close(v)
    end
    close(kept)
    deep = foldl((a, _) -> Any[a], 1:200; init = Any[1])
    for bad in (NaN, NumberText("1."), String([0xff]), Dict(String([0xfe]) => 1), C_NULL, deep)
        tree = try Value(bad); nothing catch e; e end
        tape = try Serde.emit(bad); nothing catch e; e end
        @test (tree === nothing) == (tape === nothing) && (tree === nothing || typeof(tree) == typeof(tape))
    end
end
