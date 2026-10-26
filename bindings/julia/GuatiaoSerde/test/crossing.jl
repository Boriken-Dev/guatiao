# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A tree crosses between guatiao_serde and guatiao: built by one library,
# read and freed through the other, every block accounted for; and the
# library is found as each route says.

using Guatiao: MapRef

@testset "a tree serde built is changed and freed through the core" begin
    c = Counting()
    v = Serde.parse("""{"a": [1, 2], "b": "x"}"""; alloc = c.alloc)
    @test live(c) > 0
    MapRef(v)["c"] = "added through the core"
    @test v[] == Dict("a" => [1, 2], "b" => "x", "c" => "added through the core")
    close(v)
    @test live(c) == 0
end

@testset "a tree the core built is written by serde" begin
    c = Counting()
    v = Value(Dict("port" => 5900); alloc = c.alloc)
    @test Serde.emit(v) == """{"port":5900}"""
    @test Serde.emit(Dict("a" => 1); alloc = c.alloc) == """{"a":1}"""
    close(v)
    @test live(c) == 0
end

@testset "nothing is left after a document that does not parse" begin
    c = Counting()
    @test_throws ParseError Serde.parse("{"; alloc = c.alloc)
    @test live(c) == 0
end

@testset "a parse error is laid out as the header says" begin
    @test sizeof(GuatiaoSerde.GParseError) == (sizeof(Ptr{Cvoid}) == 8 ? 16 : 12) + 8 + 4 * sizeof(Ptr{Cvoid})
    @test fieldoffset(GuatiaoSerde.GParseError, 4) == (sizeof(Ptr{Cvoid}) == 8 ? 16 : 12)
end

@testset "the library is found through each route" begin
    dir = library_dir()
    file(base) = joinpath(dir, Guatiao.library_filename(base))
    nowhere = joinpath(dir, "nowhere")
    for (own, shared, want) in (
        (file("guatiao_intake"), nowhere, file("guatiao_intake")),
        (dir, nowhere, file("guatiao_serde")),
        ("", file("guatiao_serde"), file("guatiao_serde")),
        ("", file("guatiao"), file("guatiao_serde")),
    )
        withenv("GUATIAO_SERDE_LIBRARY" => own, "GUATIAO_LIBRARY" => shared) do
            @test Guatiao.FFI.find_library("guatiao_serde") == want
        end
    end
    # A name nothing in this process has loaded, so the platform's search
    # finds nothing either, declared with its own variable.
    Guatiao.FFI.surface!("not_a_real_guatiao_library"; variable = "NOT_A_REAL_GUATIAO_LIBRARY", probe = "x")
    withenv("NOT_A_REAL_GUATIAO_LIBRARY" => nowhere, "GUATIAO_LIBRARY" => nowhere) do
        e = try
            Guatiao.FFI.find_library("not_a_real_guatiao_library")
        catch e
            e
        end
        @test e isa LibraryNotFound
        @test any(occursin("NOT_A_REAL_GUATIAO_LIBRARY", l) for l in e.looked)
        @test any(occursin("GUATIAO_LIBRARY", l) for l in e.looked)
    end
end
