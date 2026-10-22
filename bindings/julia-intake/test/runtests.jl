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
using GuatiaoIntake

const Intake = GuatiaoIntake

include("helpers.jl")
include("json.jl")

@testset "GuatiaoIntake" begin
    @testset "intake" begin
        include("intake.jl")
    end
    @testset "crossing" begin
        include("crossing.jl")
    end
end
