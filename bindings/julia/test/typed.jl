# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A configuration's schema, read from its fields.

Guatiao.@config struct ListenConfig
    "Where to listen."
    host::String = "0.0.0.0"
    port::UInt16
    "A label, when there is one."
    label::Union{Nothing,String} = nothing
    tags::Vector{String} = String[]
end
Guatiao.keywords(::Type{ListenConfig}) = (host = (minLength = 1,),)

struct Plain
    name::String
    note::Union{Nothing,String}
end

@testset "a struct reads as an object with its required fields" begin
    s = schema(ListenConfig)
    @test keys(MapRef(s["properties"])) == ["host", "port", "label", "tags"]
    @test s["required"][] == ["port"]
    @test s["additionalProperties"][] === false
    @test s["properties"]["host"][] == Dict("type" => "string", "description" => "Where to listen.",
        "default" => "0.0.0.0", "minLength" => 1)
    @test s["properties"]["port"][] == Dict("type" => "integer", "minimum" => 0, "maximum" => 65535)
    @test s["properties"]["label"]["anyOf"][] == [Dict("type" => "string"), Dict("type" => "null")]
    close(s)
    p = schema(Plain)
    @test p["required"][] == ["name"]
    close(p)
end

@testset "plain data builds the struct back" begin
    @test fieldwise(decode(ListenConfig, Value((port = 80,))), ListenConfig(port = 80))
    @test fieldwise(decode(ListenConfig, Value((port = 80, host = "h", label = "x"))),
        ListenConfig(host = "h", port = 80, label = "x"))
    @test decode(Plain, Value((name = "a",))) == Plain("a", nothing)
    @test_throws GuatiaoError decode(Plain, Value((note = "a",)))
end

@testset "a schema the library's validator agrees with" begin
    s = schema(ListenConfig)
    for (config, ok) in ((Dict("port" => 80), true), (Dict("port" => 80, "host" => ""), false),
        (Dict("host" => "h"), false), (Dict("port" => 80, "extra" => 1), false), (Dict("port" => 70000), false))
        Value(config) do c
            out = Guatiao.new_root()
            status = GC.@preserve s c Guatiao.guatiao_schema_validate(Guatiao.require_node(s), Guatiao.require_node(c),
                Guatiao.binding_alloc(), out)
            Guatiao.free_root(out)
            @test (status == 0) == ok
        end
    end
    close(s)
end

@testset "a type no schema describes is named" begin
    @test_throws ArgumentError schema(Ptr{Int})
end
