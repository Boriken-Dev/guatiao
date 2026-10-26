# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Documents to plain Julia data and back with no tree in between. Reading
# takes a C visitor's events as @cfunction callbacks, which a C call makes
# without leaving Julia's thread; writing builds the wire bytes in Julia,
# one crossing for the whole document. Each measured faster than its other
# way and than a Value (benchmarks/README.md).

"`guatiao_visitor`: the event callbacks a walk calls, each answering a status."
struct GVisitor
    struct_size::UInt32
    ctx::Ptr{Cvoid}
    absent::Ptr{Cvoid}
    null::Ptr{Cvoid}
    boolean::Ptr{Cvoid}
    number::Ptr{Cvoid}
    string::Ptr{Cvoid}
    bytes::Ptr{Cvoid}
    list_begin::Ptr{Cvoid}
    list_end::Ptr{Cvoid}
    map_begin::Ptr{Cvoid}
    key::Ptr{Cvoid}
    map_end::Ptr{Cvoid}
end

# One value's events collected as plain data: what `plain` answers for the
# tree the same events would build.
mutable struct PlainReader
    stack::Vector{Any}
    keys::Vector{String}
    key::String
    root::Any
    failure::Any
end
PlainReader() = PlainReader(Any[], String[], "", nothing, nothing)

reader_of(ctx::Ptr{Cvoid}) = unsafe_pointer_to_objref(ctx)::PlainReader
str_of(s::GStr) = s.len == 0 ? "" : unsafe_string(s.ptr, s.len)

function plain_put!(r::PlainReader, v)
    if isempty(r.stack)
        r.root = v
    else
        top = r.stack[end]
        if top isa Dict{String,Any}
            top[r.key] = v
        else
            push!(top::Vector{Any}, v)
        end
    end
    return nothing
end

function opened!(r::PlainReader, c)
    length(r.stack) >= MAX_DEPTH && throw(GuatiaoError(BadValue, "a tree deeper than $MAX_DEPTH containers"))
    push!(r.keys, r.key)
    push!(r.stack, c)
    return nothing
end

function closed!(r::PlainReader)
    c = pop!(r.stack)
    r.key = pop!(r.keys)
    plain_put!(r, c)
end

# A callback never throws into C: a failure is kept, and a status stops the walk.
macro event(ctx, body)
    r = esc(:r)
    quote
        $r = reader_of($(esc(ctx)))
        try
            $(esc(body))
            UInt32(0)
        catch e
            $r.failure = e
            UInt32(BadValue)
        end
    end
end

ev_absent(ctx::Ptr{Cvoid}) = @event ctx plain_put!(r, absent)
ev_null(ctx::Ptr{Cvoid}) = @event ctx plain_put!(r, nothing)
ev_bool(ctx::Ptr{Cvoid}, b::Bool) = @event ctx plain_put!(r, b)
ev_number(ctx::Ptr{Cvoid}, s::GStr) = @event ctx plain_put!(r, auto_number(str_of(s)))
ev_string(ctx::Ptr{Cvoid}, s::GStr) = @event ctx plain_put!(r, str_of(s))
ev_bytes(ctx::Ptr{Cvoid}, b::GBytes) = @event ctx plain_put!(r, b.len == 0 ? UInt8[] : copy(unsafe_wrap(Vector{UInt8}, b.ptr, b.len)))
ev_list(ctx::Ptr{Cvoid}, ::UInt64) = @event ctx opened!(r, Any[])
ev_map(ctx::Ptr{Cvoid}, n::UInt64) = @event ctx opened!(r, n < 1 << 16 ? sizehint!(Dict{String,Any}(), Int(n)) : Dict{String,Any}())
ev_key(ctx::Ptr{Cvoid}, s::GStr) = @event ctx (r.key = str_of(s))
ev_end(ctx::Ptr{Cvoid}) = @event ctx closed!(r)

"""
    read_plain(call) -> (data, status)

Calls `call(visitor::Ptr{Cvoid})`, which hands a `guatiao_visitor` to a C
walk (a `*_visit` export) and answers its status, and answers the plain
Julia data its events make, as `plain` reads the tree they would build,
with that status. A failure inside the events is rethrown.
"""
function read_plain(call)
    r = PlainReader()
    v = Ref(GVisitor(sizeof(GVisitor), pointer_from_objref(r),
        @cfunction(ev_absent, UInt32, (Ptr{Cvoid},)), @cfunction(ev_null, UInt32, (Ptr{Cvoid},)),
        @cfunction(ev_bool, UInt32, (Ptr{Cvoid}, Bool)), @cfunction(ev_number, UInt32, (Ptr{Cvoid}, GStr)),
        @cfunction(ev_string, UInt32, (Ptr{Cvoid}, GStr)), @cfunction(ev_bytes, UInt32, (Ptr{Cvoid}, GBytes)),
        @cfunction(ev_list, UInt32, (Ptr{Cvoid}, UInt64)), @cfunction(ev_end, UInt32, (Ptr{Cvoid},)),
        @cfunction(ev_map, UInt32, (Ptr{Cvoid}, UInt64)), @cfunction(ev_key, UInt32, (Ptr{Cvoid}, GStr)),
        @cfunction(ev_end, UInt32, (Ptr{Cvoid},))))
    status = GC.@preserve r v UInt32(call(Ptr{Cvoid}(Base.unsafe_convert(Ptr{GVisitor}, v))))
    r.failure === nothing || throw(r.failure)
    return (r.root, status)
end

# ---- writing wire bytes ------------------------------------------------------------

const WIRE_ABSENT = 0x00
const WIRE_NULL = 0x01
const WIRE_BOOL = 0x02
const WIRE_NUMBER = 0x03
const WIRE_STRING = 0x04
const WIRE_BYTES = 0x05
const WIRE_LIST = 0x06
const WIRE_MAP = 0x07

function leb!(io::Vector{UInt8}, n::Integer)
    n = UInt(n)
    while n >= 0x80
        push!(io, UInt8(n & 0x7f) | 0x80)
        n >>= 7
    end
    push!(io, UInt8(n))
    return nothing
end

# `append!` of `codeunits(s)` copies a byte at a time, about ten times
# slower than one block copy on a large string.
function put_bytes!(io::Vector{UInt8}, data::Union{String,Vector{UInt8}})
    n = sizeof(data)
    at = length(io)
    resize!(io, at + n)
    GC.@preserve io data unsafe_copyto!(pointer(io, at + 1), pointer(data), n)
    return nothing
end
put_bytes!(io::Vector{UInt8}, data::Base.CodeUnits{UInt8,String}) = put_bytes!(io, data.s)
put_bytes!(io::Vector{UInt8}, data) = (append!(io, data); nothing)

function body!(io::Vector{UInt8}, tag::UInt8, s::String)
    push!(io, tag)
    leb!(io, ncodeunits(s))
    put_bytes!(io, s)
    return nothing
end

"Whether `s` is in the JSON number grammar (RFC 8259, section 6)."
function is_number_text(s::String)
    b = codeunits(s)
    i, n = 1, length(b)
    i <= n && b[i] == UInt8('-') && (i += 1)
    digit(c) = UInt8('0') <= c <= UInt8('9')
    if i <= n && b[i] == UInt8('0')
        i += 1
    elseif i <= n && UInt8('1') <= b[i] <= UInt8('9')
        while i <= n && digit(b[i]); i += 1; end
    else
        return false
    end
    if i <= n && b[i] == UInt8('.')
        i += 1
        start = i
        while i <= n && digit(b[i]); i += 1; end
        i == start && return false
    end
    if i <= n && (b[i] == UInt8('e') || b[i] == UInt8('E'))
        i += 1
        i <= n && (b[i] == UInt8('+') || b[i] == UInt8('-')) && (i += 1)
        start = i
        while i <= n && digit(b[i]); i += 1; end
        i == start && return false
    end
    return i > n
end

function wire_text!(io, s::String, number::Bool)
    if number
        is_number_text(s) || throw(GuatiaoError(BadValue, "not a number: the JSON number grammar"))
        body!(io, WIRE_NUMBER, s)
    else
        isvalid(s) || throw(GuatiaoError(BadValue, "not UTF-8 text"))
        body!(io, WIRE_STRING, s)
    end
end

# What the tree path writes for `x`, as wire bytes: for a value this walk
# does not take apart (a Value or view, a date or a length) and for a map
# whose keys may write one text twice, which the tree keeps once.
function wire_via_tree!(io, x)
    v = Value(x)
    try
        append!(io, to_wire(v))
    finally
        close(v)
    end
end

function wire_map!(io, pairs, n::Int, depth::Int)
    push!(io, WIRE_MAP)
    leb!(io, n)
    for (k, v) in pairs
        key = map_key_text(k)
        isvalid(key) || throw(GuatiaoError(BadValue, ""))
        leb!(io, ncodeunits(key))
        put_bytes!(io, key)
        write_wire!(io, v, depth + 1)
    end
end

function wire_list!(io, items, depth::Int)
    push!(io, WIRE_LIST)
    leb!(io, length(items))
    for v in items
        write_wire!(io, v, depth + 1)
    end
end

# Keys that cannot write one text twice: text and symbols are their text.
unique_key_texts(::AbstractDict{<:Union{AbstractString,Symbol}}) = true
function unique_key_texts(d::AbstractDict)
    seen = Set{String}()
    for k in keys(d)
        t = map_key_text(k)
        t in seen && return false
        push!(seen, t)
    end
    return true
end

"Writes `x` as `encode_node!` builds it, branch for branch, as wire bytes."
function write_wire!(io::Vector{UInt8}, x, depth::Int)
    depth > MAX_DEPTH && throw(GuatiaoError(BadValue, "a tree deeper than $MAX_DEPTH containers"))
    if x === nothing
        push!(io, WIRE_NULL)
    elseif x isa Absent
        push!(io, WIRE_ABSENT)
    elseif x isa Bool
        push!(io, WIRE_BOOL, x ? 0x01 : 0x00)
    elseif x isa Integer
        body!(io, WIRE_NUMBER, string(x))
    elseif x isa AbstractFloat
        body!(io, WIRE_NUMBER, float_text(x))
    elseif x isa NumberText
        wire_text!(io, x.text, true)
    elseif x isa String
        wire_text!(io, x, false)
    elseif x isa AbstractString || x isa Symbol || x isa AbstractChar
        wire_text!(io, string(x), false)
    elseif x isa Union{Vector{UInt8},Base.CodeUnits{UInt8}}
        push!(io, WIRE_BYTES)
        leb!(io, length(x))
        put_bytes!(io, x)
    elseif x isa Node
        append!(io, to_wire(x))
    elseif x isa AbstractDict
        unique_key_texts(x) ? wire_map!(io, x, length(x), depth) : wire_via_tree!(io, x)
    elseif x isa NamedTuple
        wire_map!(io, pairs(x), length(x), depth)
    elseif x isa Union{AbstractVector,Tuple,AbstractSet}
        wire_list!(io, x, depth)
    elseif x isa Base.Enum
        wire_text!(io, string(Symbol(x)), false)
    elseif is_special(x)
        wire_via_tree!(io, x)
    else
        y = lower(x)
        if y !== x
            write_wire!(io, y, depth)
        elseif isstructtype(typeof(x)) && !(x isa Function) && !(x isa Ptr)
            T = typeof(x)
            if fieldcount(T) == 0
                push!(io, WIRE_NULL)
            else
                fields = Pair{Symbol,Any}[]
                for (i, name) in enumerate(fieldnames(T))
                    v = getfield(x, i)
                    v === nothing && Nothing <: fieldtype(T, i) && continue
                    push!(fields, name => v)
                end
                wire_map!(io, fields, length(fields), depth)
            end
        else
            throw(ArgumentError("cannot write a $(typeof(x)) as a guatiao value"))
        end
    end
    return nothing
end

# What encode_special! takes.
is_special(x) = false
is_special(::Union{Date,Time,DateTime,Timestamp,Span,Length,Dates.FixedPeriod,Dates.CompoundPeriod,Dates.OtherPeriod}) = true

"""
    julia_to_wire(x) -> Vector{UInt8}

The wire bytes of what `Value(x)` builds, written in Julia with no tree:
the same data taken, the same errors.
"""
function julia_to_wire(x)
    io = UInt8[]
    sizehint!(io, 256)
    write_wire!(io, x, 0)
    return io
end
