# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

using Dates

struct Point
    x::Int
    y::Union{Nothing,Int}
end
@enum Color red green
Base.@kwdef struct Listen
    host::String
    port::UInt16 = 5900
    tags::Vector{String} = String[]
    note::Union{Nothing,String} = nothing
end
struct Unit end
fieldwise(a, b) = typeof(a) == typeof(b) && all(getfield(a, i) == getfield(b, i) for i in 1:nfields(a))
struct Celsius
    degrees::Float64
end
Guatiao.lower(c::Celsius) = "$(c.degrees)C"

@testset "every kind round-trips" begin
    data = Dict("n" => nothing, "b" => true, "i" => -7, "f" => 2.5, "s" => "héllo",
        "y" => UInt8[0x00, 0xff], "l" => Any[1, "two"], "m" => Dict("k" => "v"))
    Value(data) do v
        @test v[] == data
        @test Guatiao.tag(v["y"]) === :bytes
        @test Guatiao.tag(v["n"]) === :null
        @test Guatiao.isnull(v["n"])
    end
    @test Guatiao.isabsent(Value())
    @test Value(absent)[] === absent
    @test Value(nothing)[] === nothing
end

@testset "numbers keep their exact text" begin
    Value(Dict("a" => NumberText("1.10"), "big" => NumberText("18446744073709551615.5"))) do v
        @test decode(NumberText, v["a"]) == NumberText("1.10")
        @test decode(NumberText, v["big"]) == NumberText("18446744073709551615.5")
        @test decode(Float64, v["a"]) == 1.1
    end
    @test Value(big(2)^100)[] == big(2)^100
    @test Value(typemax(UInt64))[] == big(typemax(UInt64))
    @test decode(UInt64, Value(typemax(UInt64))) == typemax(UInt64)
    @test decode(NumberText, Value(-0.0)) == NumberText("-0.0")
    @test decode(NumberText, Value(0.1f0)) == NumberText("0.1")
    @test decode(NumberText, Value(1.0f20)) == NumberText("1.0e20")
    for bad in (NaN, Inf, -Inf32)
        @test_throws GuatiaoError Value(bad)
    end
    @test_throws GuatiaoError Value(NumberText("01"))
end

@testset "integers are read exactly, never rounded" begin
    Value(Dict("frac" => 1.5, "big" => 300, "neg" => -1, "exp" => NumberText("1e3"))) do v
        @test decode(Int16, v["big"]) == 300
        @test_throws GuatiaoError decode(Int, v["frac"])
        @test_throws GuatiaoError decode(UInt8, v["big"])
        @test_throws GuatiaoError decode(UInt, v["neg"])
        @test_throws GuatiaoError decode(Int, v["exp"])
        e = try decode(Int, v["frac"]) catch e; e end
        @test occursin("at frac", e.message)
    end
end

@testset "a view survives growth past reallocation" begin
    Value(Dict("first" => 1)) do v
        r = v["first"]
        for i in 1:200
            v["k$i"] = i
        end
        @test r[] == 1
        @test length(MapRef(v)) == 201
    end
end

@testset "a closed value refuses every use" begin
    v = Value(Dict("a" => 1))
    r = v["a"]
    close(v)
    @test !isopen(v)
    @test_throws InvalidStateException v[]
    @test_throws InvalidStateException r[]
    @test_throws InvalidStateException (v["b"] = 2)
    close(v)   # a second close does nothing
    @test sprint(show, v) == "Guatiao.Value(closed)"
end

@testset "a do block closes" begin
    kept = nothing
    out = Value(Dict("a" => 1)) do v
        kept = v
        v["a"][]
    end
    @test out == 1
    @test !isopen(kept)
end

@testset "a map is an AbstractDict" begin
    Value(Dict{String,Any}()) do v
        m = MapRef(v)
        m["host"] = "a"
        m["port"] = 1
        m["host"] = "b"           # replaced in place
        @test keys(m) == ["host", "port"]
        @test length(m) == 2
        @test haskey(m, "port") && !haskey(m, "nope")
        @test m["host"][] == "b"
        @test get(m, "nope", 7) == 7
        @test [k for (k, _) in m] == ["host", "port"]
        @test_throws KeyError m["nope"]
        delete!(m, "host")
        @test keys(m) == ["port"]
        delete!(m, "nope")        # nothing to delete is not an error
        merge!(m, Value(Dict("x" => 1, "port" => 2)))
        @test v[] == Dict("port" => 2, "x" => 1)
        empty!(m)
        @test isempty(m)
    end
    @test_throws GuatiaoError MapRef(Value([1]))
end

@testset "a list is an AbstractVector, from 1" begin
    Value(Any["a", "b"]) do v
        l = ListRef(v)
        push!(l, "c", "d")
        @test length(l) == 4
        @test l[1][] == "a"
        @test l[end][] == "d"
        @test [x[] for x in l] == ["a", "b", "c", "d"]
        l[2] = "B"
        insert!(l, 1, "zero")
        deleteat!(l, 5)
        @test v[] == ["zero", "a", "B", "c"]
        @test_throws BoundsError l[9]
        @test_throws BoundsError (l[9] = 1)
        empty!(l)
        @test isempty(l)
    end
end

@testset "a copy is independent" begin
    Value(Dict("a" => Any[1])) do v
        c = copy(v["a"])
        push!(c, 2)
        @test v[] == Dict("a" => [1])
        @test c[] == [1, 2]
        close(c)
    end
end

@testset "Julia data converts in" begin
    Value((host = "h", port = 1, tags = ("a", :b), set = Set([1]), when = Date(2026, 10, 21))) do v
        @test v[] == Dict("host" => "h", "port" => 1, "tags" => ["a", "b"], "set" => [1], "when" => "2026-10-21")
    end
    @test Value(Dict(1 => "one", 2 => "two"))[] == Dict("1" => "one", "2" => "two")
    @test Value(Dict(true => "yes"))[] == Dict("true" => "yes")
    @test Value('é')[] == "é"
    @test Value(Point(1, nothing))[] == Dict("x" => 1)
    @test Value(Point(1, 2))[] == Dict("x" => 1, "y" => 2)
    @test Value(green)[] == "green"
    @test decode(Color, Value("red")) == red
    @test_throws ArgumentError Value([Ptr{Cvoid}(1)])
    @test Value(Unit())[] === nothing
    Value(Dict("inner" => 1)) do v
        Value(Dict("copy" => v)) do w
            @test w[] == Dict("copy" => Dict("inner" => 1))
        end
    end
end

@testset "trees read into Julia types" begin
    Value((host = "10.0.0.1", tags = ["a"], extra = 1)) do v
        l = decode(Listen, v)
        @test fieldwise(l, Listen("10.0.0.1", 5900, ["a"], nothing))
    end
    @test_throws GuatiaoError decode(Listen, Value((port = 1,)))
    @test decode(Dict{String,Int}, Value(Dict("a" => 1))) == Dict("a" => 1)
    @test decode(Dict{Int,String}, Value(Dict("2" => "two"))) == Dict(2 => "two")
    @test_throws GuatiaoError decode(Dict{UInt,String}, Value(Dict("-1" => "x")))
    @test decode(Tuple{Int,String}, Value((7, "x"))) == (7, "x")
    @test_throws GuatiaoError decode(Tuple{Int,String}, Value((7,)))
    @test_throws GuatiaoError decode(Set{Int}, Value([1, 1]))
    @test decode(NamedTuple{(:a, :b),Tuple{Int,Union{Nothing,Int}}}, Value((a = 1,))) == (a = 1, b = nothing)
    @test decode(Union{Nothing,Int}, Value(nothing)) === nothing
    @test decode(Vector{UInt8}, Value(UInt8[1, 2])) == UInt8[1, 2]
    @test decode(Char, Value("é")) == 'é'
    copied = decode(Value, Value(Dict("a" => 1)))
    @test copied[] == Dict("a" => 1)
    close(copied)
end

@testset "a lowered type writes what lower answers" begin
    @test Value(Celsius(21.5))[] == "21.5C"
end

@testset "a path in the intake grammar" begin
    Value(Dict("agent" => [Dict("name" => "x")])) do v
        @test Guatiao.path(v["agent"][1]["name"]) == "agent[0].name"
        @test Guatiao.path(v) == "the root"
    end
end
