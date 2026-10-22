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
