# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# An allocator counting the blocks it handed out and has not taken back,
# over the core library's default: a tree built in it by one library and
# freed through another comes back to zero.

using Guatiao.FFI: GAlloc, binding_alloc

const REPO = normpath(joinpath(@__DIR__, "..", "..", "..", ".."))

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

"The core library's default allocator, wrapped to count live blocks."
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
