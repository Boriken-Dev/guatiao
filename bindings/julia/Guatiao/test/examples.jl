# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The example host runs as its README says, in a process of its own.

@testset "the Julia host example runs" begin
    host = joinpath(REPO, "examples", "julia_host", "host.jl")
    args = String[library_dir()]
    c = build_c_greeter()
    c === nothing || push!(args, c)
    go = get(ENV, "GUATIAO_GO_LIBRARY", "")
    isempty(go) || push!(args, go)
    py = bridge()
    if py !== nothing && haskey(ENV, "GUATIAO_PY_BRIDGE")
        append!(args, ["--python-source", py, "--python-dir", joinpath(REPO, "examples", "python_provider"),
            "--python-path", joinpath(REPO, "bindings", "python", "guatiao", "src"),
            "--python-path", joinpath(REPO, "examples", "greeter_kind", "python")])
    end
    out = read(pipeline(`$(Base.julia_cmd()) --startup-file=no $host $args`; stderr = devnull), String)
    @test occursin("registered JlGreeter 1.0.0 from Julia", out)
    @test occursin("JlGreeter_hello (JlGreeter 1.0.0)\n  greet: hello, ana", out)
    @test occursin("derived_greeter_hello", out)
    @test occursin("an empty configuration is refused", out)
    @test occursin("not this greeter kind", out)   # hello_library's table is another kind's
    c === nothing || @test occursin("c_greeter (c_greeter_library 1.0)\n  greet: hello from C, ana", out)
    isempty(go) || @test occursin("go_greeter_hello", out)
    "--python-source" in args && @test occursin("served py_greeter from Python", out)
    @test occursin("retired JlGreeter, still mapped: unload said WrongKind", out)
    @test endswith(out, "objects a library can still reach: 0\n")
end
