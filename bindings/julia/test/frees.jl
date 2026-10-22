# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Everything built is freed on every path, counted by an allocator that
# knows how many blocks it handed out and has not taken back.

"A value no tree can hold."
const UNWRITABLE = Ptr{Cvoid}(1)

@testset "a clean build and close leaves nothing" begin
    c = Counting()
    v = Value(Dict("a" => [1, "two", UInt8[3]], "m" => Dict("x" => nothing)); alloc = c.alloc)
    @test live(c) > 0
    v["b"] = "more"
    push!(v["a"], 4)
    close(v)
    @test live(c) == 0
end

@testset "a conversion that fails part way frees what it built" begin
    c = Counting()
    for bad in (Dict("a" => [1, 2, UNWRITABLE]), [1, Dict("x" => NaN)], Dict(1.5 => "key"))
        @test_throws Exception Value(bad; alloc = c.alloc)
        @test live(c) == 0
    end
end

@testset "a failed write leaves the tree as it was" begin
    c = Counting()
    v = Value(Dict("a" => [1]); alloc = c.alloc)
    before = live(c)
    @test_throws Exception (v["b"] = [1, UNWRITABLE])
    @test_throws Exception push!(v["a"], Dict("x" => Inf))
    @test_throws Exception (ListRef(v["a"])[1] = [UNWRITABLE])
    @test_throws Exception insert!(ListRef(v["a"]), 1, UNWRITABLE)
    @test live(c) == before
    @test v[] == Dict("a" => [1])
    ListRef(v["a"])[1] = "one"
    insert!(ListRef(v["a"]), 1, "zero")
    @test v[] == Dict("a" => ["zero", "one"])
    close(v)
    @test live(c) == 0
end

@testset "answers are built in the allocator named, and freed" begin
    c = Counting()
    r = redact(Dict("type" => "object"), Dict("a" => 1); alloc = c.alloc)
    @test live(c) > 0
    close(r)
    @test live(c) == 0
end

@testset "an unclosed value is freed by the collector" begin
    c = Counting()
    let
        Value(Dict("a" => [1, 2, 3]); alloc = c.alloc)
    end
    for _ in 1:5
        GC.gc(true)
        live(c) == 0 && break
    end
    @test live(c) == 0
end
