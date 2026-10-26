# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A document to plain Julia data and back, three ways: through a Value
# (`GuatiaoSerde.parse(text)[]`, `GuatiaoSerde.emit(Value(data))`), through
# the wire bytes as a tape read and written in Julia, and through
# @cfunction callbacks (a visitor the library calls per event, a walker
# calling the library's visitor per event). The package reads through the
# callbacks (`GuatiaoSerde.parse(Any, text)`) and writes through the tape
# (`GuatiaoSerde.emit(data)`); the other two ways are written here.
#
#     GUATIAO_LIBRARY=<target>/release julia --project=<env> benchmarks/julia/documents.jl [--save NAME]
#
# in an environment holding Guatiao and GuatiaoSerde from the checkout.
# Writes benchmarks/results/NAME.json in the runner's shape.

using Guatiao, GuatiaoSerde
using Guatiao.FFI: FFI, @cdef, GValue, GAlloc, GStr, GBytes, gstr, binding_alloc, new_root, free_root, check,
    load_tag, payload_ptr, text_of, TAG_STRING, TAG_BYTES
using Printf

const PV = Ptr{GValue}
const PA = Ptr{GAlloc}

@cdef "guatiao_serde" "" :plain guatiao_json_to_wire(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{Cvoid})::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit_walker(w::Ptr{Cvoid}, how::UInt32, a::PA, out::PV)::UInt32

# ---- the tape --------------------------------------------------------------------

mutable struct Tape
    data::Vector{UInt8}
    at::Int
end

@inline function leb!(t::Tape)
    n = UInt(0)
    shift = 0
    while true
        b = t.data[t.at]
        t.at += 1
        n |= UInt(b & 0x7f) << shift
        b < 0x80 && return Int(n)
        shift += 7
    end
end

@inline function body!(t::Tape)
    n = leb!(t)
    s = t.at
    t.at += n
    return s, n
end

function read_node!(t::Tape)
    tag = t.data[t.at]
    t.at += 1
    if tag == 0x04
        s, n = body!(t)
        return String(view(t.data, s:s+n-1))
    elseif tag == 0x03
        s, n = body!(t)
        return Guatiao.auto_number(String(view(t.data, s:s+n-1)))
    elseif tag == 0x07
        n = leb!(t)
        d = Dict{String,Any}()
        sizehint!(d, n)
        for _ in 1:n
            s, k = body!(t)
            key = String(view(t.data, s:s+k-1))
            d[key] = read_node!(t)
        end
        return d
    elseif tag == 0x06
        n = leb!(t)
        l = Vector{Any}(undef, n)
        for i in 1:n
            l[i] = read_node!(t)
        end
        return l
    elseif tag == 0x01
        return nothing
    elseif tag == 0x02
        b = t.data[t.at]
        t.at += 1
        return b == 0x01
    elseif tag == 0x05
        s, n = body!(t)
        return t.data[s:s+n-1]
    elseif tag == 0x00
        return Guatiao.absent
    end
    error("unknown tag $tag")
end

wire_to_julia(data::Vector{UInt8}) = read_node!(Tape(data, 1))

function parse_tape(text::String)
    out = new_root()
    try
        st = GC.@preserve text guatiao_json_to_wire(gstr(text), UInt32(0), binding_alloc(), out, C_NULL)
        check(st, "")
        b = unsafe_load(payload_ptr(Guatiao.GBuffer, out))
        data = unsafe_wrap(Vector{UInt8}, b.ptr, b.len)
        return wire_to_julia(data)
    finally
        free_root(out)
    end
end

const GString = Guatiao.GString

render_tape(x) = GuatiaoSerde.emit(x)
parse_callbacks(text::String) = GuatiaoSerde.parse(Any, text)

# ---- callbacks -------------------------------------------------------------------

const CVisitor = FFI.GVisitor

struct CWalker
    struct_size::UInt32
    ctx::Ptr{Cvoid}
    walk::Ptr{Cvoid}
end

const WALKING = Ref{Any}(nothing)

function go(into::Ptr{CVisitor}, x)
    v = unsafe_load(into)
    if x isa String
        GC.@preserve x ccall(v.string, UInt32, (Ptr{Cvoid}, GStr), v.ctx, gstr(x))
    elseif x isa Bool
        ccall(v.boolean, UInt32, (Ptr{Cvoid}, Bool), v.ctx, x)
    elseif x isa Integer
        s = string(x); GC.@preserve s ccall(v.number, UInt32, (Ptr{Cvoid}, GStr), v.ctx, gstr(s))
    elseif x isa AbstractFloat
        s = Guatiao.float_text(x); GC.@preserve s ccall(v.number, UInt32, (Ptr{Cvoid}, GStr), v.ctx, gstr(s))
    elseif x === nothing
        ccall(v.null, UInt32, (Ptr{Cvoid},), v.ctx)
    elseif x isa AbstractDict
        ccall(v.map_begin, UInt32, (Ptr{Cvoid}, UInt64), v.ctx, length(x))
        for (k, val) in x
            ks = String(k); GC.@preserve ks ccall(v.key, UInt32, (Ptr{Cvoid}, GStr), v.ctx, gstr(ks))
            go(into, val)
        end
        ccall(v.map_end, UInt32, (Ptr{Cvoid},), v.ctx)
    else
        ccall(v.list_begin, UInt32, (Ptr{Cvoid}, UInt64), v.ctx, length(x))
        for val in x
            go(into, val)
        end
        ccall(v.list_end, UInt32, (Ptr{Cvoid},), v.ctx)
    end
    return nothing
end

walk_cb(::Ptr{Cvoid}, into::Ptr{CVisitor}) = (go(into, WALKING[]); UInt32(0))

function render_callbacks(x)
    WALKING[] = x
    w = Ref(CWalker(sizeof(CWalker), C_NULL, @cfunction(walk_cb, UInt32, (Ptr{Cvoid}, Ptr{CVisitor}))))
    out = new_root()
    try
        st = GC.@preserve w guatiao_json_emit_walker(Ptr{Cvoid}(Base.unsafe_convert(Ptr{CWalker}, w)), UInt32(0),
            binding_alloc(), out)
        check(st, "")
        return text_of(unsafe_load(payload_ptr(GString, out)))
    finally
        free_root(out)
    end
end

# ---- the value path ----------------------------------------------------------------

function parse_value(text::String)
    v = GuatiaoSerde.parse(text)
    try
        return v[]
    finally
        close(v)
    end
end

function render_value(x)
    v = Value(x)
    try
        return GuatiaoSerde.emit(v)
    finally
        close(v)
    end
end

# ---- the cases and the clock --------------------------------------------------------

function config()
    servers = Any[]
    for i in 0:49
        push!(servers, Dict{String,Any}("host" => "node-$i.example", "port" => 8000 + i, "weight" => i / 7,
            "tags" => Any["edge", "eu", string(i)], "enabled" => i % 3 != 0,
            "limits" => Dict{String,Any}("rps" => 1000 + i, "burst" => 2.5, "note" => nothing)))
    end
    return Dict{String,Any}("name" => "fleet", "servers" => servers)
end

cases() = [("config", config()), ("floats_10k", Any[i * 1.25 + 0.5 for i in 0:9999]),
    ("text_1mb", Dict{String,Any}("blob" => "x"^(1 << 20)))]

function timeit(f, target, samples)
    f()
    n = 1
    while true
        t = time_ns()
        for _ in 1:n
            f()
        end
        (time_ns() - t) / 1e9 >= target && break
        n *= 2
    end
    out = Float64[]
    for _ in 1:samples
        t = time_ns()
        for _ in 1:n
            f()
        end
        push!(out, (time_ns() - t) / n / 1e6)
    end
    return out
end

median(xs) = (s = sort(xs); n = length(s); isodd(n) ? s[(n+1)÷2] : (s[n÷2] + s[n÷2+1]) / 2)

function main(args)
    save = (i = findfirst(==("--save"), args)) === nothing ? nothing : args[i+1]
    samples = (i = findfirst(==("--samples"), args)) === nothing ? 15 : parse(Int, args[i+1])
    only = get(ENV, "BENCH_ONLY", "")
    metrics = Pair{String,Function}[]
    for (name, data) in cases()
        text = render_tape(data)
        want = parse_value(text)
        @assert parse_tape(text) == want "$name: the tape reads differently"
        @assert parse_callbacks(text) == want "$name: the callbacks read differently"
        @assert render_value(data) == text && render_callbacks(data) == text "$name: a path writes differently"
        push!(metrics, "$(name)_parse_value" => () -> parse_value(text), "$(name)_parse_tape" => () -> parse_tape(text),
            "$(name)_parse_callbacks" => () -> parse_callbacks(text), "$(name)_render_value" => () -> render_value(data),
            "$(name)_render_tape" => () -> render_tape(data), "$(name)_render_callbacks" => () -> render_callbacks(data))
    end
    results = Pair{String,Vector{Float64}}[]
    for (name, f) in metrics
        isempty(only) || occursin(only, name) || continue
        xs = timeit(f, 0.02, samples)
        push!(results, name => xs)
        @printf("%-32s %12.1f us\n", name, median(xs) * 1000)
    end
    if save !== nothing
        path = joinpath(@__DIR__, "..", "results", save * ".json")
        open(path, "w") do io
            println(io, "{\n  \"name\": \"", save, "\",\n  \"metrics\": {")
            for (i, (name, xs)) in enumerate(results)
                @printf(io, "    \"%s\": {\"min_ms\": %.6f, \"median_ms\": %.6f, \"max_ms\": %.6f}%s\n", name, minimum(xs),
                    median(xs), maximum(xs), i == length(results) ? "" : ",")
            end
            println(io, "  },\n  \"raw\": {\"processor\": \"", Sys.ARCH, "\", \"os\": \"", Sys.KERNEL, "\", \"julia\": \"",
                VERSION, "\"}\n}")
        end
        println(stderr, "saved ", path)
    end
end

main(ARGS)
