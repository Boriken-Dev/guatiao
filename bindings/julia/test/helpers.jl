# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# What several test files share: the repository's paths, an allocator that
# counts the blocks it handed out and has not taken back, and the shared
# vectors, read by `json.jl` so every number stays exact.

using Guatiao: GAlloc, GValue, binding_alloc

const REPO = normpath(joinpath(@__DIR__, "..", "..", ".."))
const HEADER = joinpath(REPO, "crates", "guatiao", "include", "guatiao.h")
const GREETER_KIND_H = joinpath(REPO, "examples", "greeter_kind", "include", "greeter_kind.h")
const VECTORS = joinpath(REPO, "crates", "guatiao", "tests", "vectors", "time.json")

# ---- a counting allocator -------------------------------------------------------

const COUNTS = Threads.Atomic{Int}[]

function counting_alloc(ctx::Ptr{Cvoid}, size::Csize_t, align::Csize_t)::Ptr{Cvoid}
    base = unsafe_load(binding_alloc())
    p = ccall(base.alloc, Ptr{Cvoid}, (Ptr{Cvoid}, Csize_t, Csize_t), base.ctx, size, align)
    p == C_NULL || Threads.atomic_add!(COUNTS[Int(ctx)], 1)
    return p
end

function counting_free(ctx::Ptr{Cvoid}, p::Ptr{Cvoid}, size::Csize_t, align::Csize_t)::Cvoid
    p == C_NULL || Threads.atomic_sub!(COUNTS[Int(ctx)], 1)
    base = unsafe_load(binding_alloc())
    ccall(base.free, Cvoid, (Ptr{Cvoid}, Ptr{Cvoid}, Csize_t, Csize_t), base.ctx, p, size, align)
    return nothing
end

"The library's default allocator, wrapped to count live blocks; kept for the process."
struct Counting
    alloc::Ptr{GAlloc}
    index::Int
end
function Counting()
    push!(COUNTS, Threads.Atomic{Int}(0))
    index = length(COUNTS)
    a = Ptr{GAlloc}(Libc.calloc(1, sizeof(GAlloc)))
    unsafe_store!(a, GAlloc(sizeof(GAlloc), Ptr{Cvoid}(index),
        @cfunction(counting_alloc, Ptr{Cvoid}, (Ptr{Cvoid}, Csize_t, Csize_t)),
        @cfunction(counting_free, Cvoid, (Ptr{Cvoid}, Ptr{Cvoid}, Csize_t, Csize_t)), C_NULL))
    return Counting(a, index)
end
live(c::Counting) = COUNTS[c.index][]

# ---- the shared vectors ------------------------------------------------------------

vectors() = Value(read_json(read(VECTORS, String)))

# ---- C, built for the tests that call a library written in it ------------------------

"The first C compiler this machine has (`CC` first), or `nothing`."
function c_compiler()
    for cc in (get(ENV, "CC", ""), "clang", "cc", "gcc")
        isempty(cc) && continue
        path = Sys.which(cc)
        path === nothing || return path
    end
    return nothing
end

"`source` built as the library `name` in `dir`; `nothing` without a compiler."
function build_c(source::AbstractString, dir::AbstractString, name::AbstractString; includes = String[])
    cc = c_compiler()
    cc === nothing && return nothing
    mkpath(dir)
    out = joinpath(dir, Guatiao.library_filename(name))
    args = ["-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"]
    Sys.iswindows() || append!(args, ["-fPIC"])
    Sys.islinux() && push!(args, "-pthread")
    for inc in [joinpath(REPO, "crates", "guatiao", "include"); joinpath(REPO, "examples", "greeter_kind", "include"); includes]
        append!(args, ["-I", inc])
    end
    append!(args, ["-o", out, source])
    run(`$cc $args`)
    return out
end

const C_CONSUMER = joinpath(REPO, "crates", "guatiao", "tests", "c_consumer")

"A fresh directory under the system's temporary one, left for the system to clean."
scratch(name) = mktempdir(; prefix = "guatiao-julia-$name-", cleanup = false)

"The greeter written in C, in a directory of its own; `nothing` without a compiler."
function build_c_greeter()
    dir = scratch("c-greeter")
    return build_c(joinpath(C_CONSUMER, "greeter_in_c.c"), dir, "c_greeter") === nothing ? nothing : dir
end

"The source written in C (a shelf of counters), as a file; `nothing` without a compiler."
build_c_source(name) = build_c(joinpath(C_CONSUMER, "source_in_c.c"), scratch("c-shelf"), name)

"The workspace build of `base`, beside the guatiao library."
workspace_library(base) = joinpath(ENV["GUATIAO_LIBRARY"], Guatiao.library_filename(base))

"The guatiao-py bridge: `GUATIAO_PY_BRIDGE`, else its debug build when it is there, else `nothing`."
function bridge()
    named = get(ENV, "GUATIAO_PY_BRIDGE", joinpath(REPO, "crates", "guatiao-py", "target", "debug", Guatiao.library_filename("guatiao_py")))
    return isfile(named) ? named : nothing
end

"What the bridge is told: where the Python greeters are, and what they import."
bridge_config() = Dict("dirs" => [joinpath(REPO, "examples", "python_provider")],
    "paths" => [joinpath(REPO, "bindings", "python", "src"), joinpath(REPO, "examples", "greeter_kind", "python")])

# The Julia kinds and greeters are packages of the examples, found by name.
for dir in (joinpath(REPO, "examples", "greeter_kind", "julia"), joinpath(REPO, "examples", "julia_provider"))
    dir in LOAD_PATH || push!(LOAD_PATH, dir)
end
