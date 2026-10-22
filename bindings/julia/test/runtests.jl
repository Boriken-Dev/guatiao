# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The suite drives a real build: GUATIAO_LIBRARY names it, and without it
# the repository's own `target/debug` is used.

let repo = normpath(joinpath(@__DIR__, "..", "..", ".."))
    haskey(ENV, "GUATIAO_LIBRARY") || (ENV["GUATIAO_LIBRARY"] = joinpath(repo, "target", "debug"))
end

using Test
using Guatiao

include("helpers.jl")

@testset "Guatiao" begin
    @testset "abi" begin
        include("abi.jl")
    end
    @testset "values" begin
        include("values.jl")
    end
    @testset "typed" begin
        include("typed.jl")
    end
    @testset "serde" begin
        include("serde.jl")
    end
    @testset "intake" begin
        include("intake.jl")
    end
    @testset "time" begin
        include("time.jl")
    end
    @testset "frees" begin
        include("frees.jl")
    end
    @testset "greeters" begin
        include("greeters.jl")
    end
    @testset "provide" begin
        include("provide.jl")
    end
    @testset "registry" begin
        include("registry.jl")
    end
    @testset "examples" begin
        include("examples.jl")
    end
end
