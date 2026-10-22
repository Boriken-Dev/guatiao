# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# What several test files share: the repository's paths, an allocator that
# counts the blocks it handed out and has not taken back, and the shared
# vectors, read with guatiao's own JSON parser so every number stays exact.

using Guatiao: GAlloc, GValue, binding_alloc

const REPO = normpath(joinpath(@__DIR__, "..", "..", ".."))
const HEADER = joinpath(REPO, "crates", "guatiao", "include", "guatiao.h")
const SERDE_HEADER = joinpath(REPO, "crates", "guatiao-serde", "include", "guatiao_serde.h")
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

vectors() = Serde.parse(read(VECTORS, String))

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
