# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A value tree: `Value` owns a root node; `ValueRef`, `MapRef` and
# `ListRef` address a node inside one by a path of keys and positions,
# walked again on every access: a write beside a node can move the array
# it sits in, so no address is ever cached.

"The answer to a lookup that found nothing: distinct from `nothing`, the stored null."
struct Absent end
"The absent marker, written as a value's eighth kind."
const absent = Absent()
Base.show(io::IO, ::Absent) = print(io, "Guatiao.absent")

"""
    NumberText(text)

A number's exact text, as a value keeps it: `NumberText("1.10")` stays
`1.10`, and an integer of any size survives. `Float64(n)`, `BigInt(n)`,
`Int64(n)` and `parse(T, n)` read it.
"""
struct NumberText
    text::String
end
Base.string(n::NumberText) = n.text
Base.show(io::IO, n::NumberText) = print(io, "NumberText(", repr(n.text), ")")
Base.parse(::Type{T}, n::NumberText) where {T<:Real} = parse(T, n.text)
(::Type{T})(n::NumberText) where {T<:AbstractFloat} = parse(T, n.text)
(::Type{T})(n::NumberText) where {T<:Integer} = parse(T, n.text)

abstract type AbstractOwner end

"""
    Value(x = absent; alloc)

Owns a root node, built from the Julia data `x` (see the conversions in
this package's API header). `close(v)` frees it; one dropped unclosed is
freed when the collector finds it. `Value(f, x)` hands the value to `f`
and closes it afterwards (a `do` block). `alloc` names the
`guatiao_alloc` to build in (kept alive by the caller); the default is
the library's own.
"""
mutable struct Value <: AbstractOwner
    root::Ptr{GValue}
    alloc::Ptr{GAlloc}
    function Value(root::Ptr{GValue}, alloc::Ptr{GAlloc}, ::Val{:adopt})
        v = new(root, alloc)
        finalizer(release!, v)
        return v
    end
end

new_root() = Ptr{GValue}(Libc.calloc(1, sizeof(GValue)))

function free_root(root::Ptr{GValue})
    root == C_NULL && return nothing
    guatiao_value_free(root)
    Libc.free(root)
    return nothing
end

function release!(v::Value)
    root = v.root
    root == C_NULL && return nothing
    v.root = C_NULL
    free_root(root)
    return nothing
end

"Takes over `root` (a node in C memory from `Libc.malloc`), which the Value then frees."
adopt(root::Ptr{GValue}, alloc::Ptr{GAlloc} = binding_alloc()) = Value(root, alloc, Val(:adopt))

function Value(x = absent; alloc::Ptr{GAlloc} = binding_alloc())
    root = new_root()
    try
        encode!(root, x, alloc)
    catch
        free_root(root)
        rethrow()
    end
    return adopt(root, alloc)
end

function Value(f::Function, x = absent; alloc::Ptr{GAlloc} = binding_alloc())
    v = Value(x; alloc)
    try
        return f(v)
    finally
        close(v)
    end
end

"""
    Value(root::Ptr{GValue})

Takes over a root node a C function answered (a `guatiao_value` in
`Libc.malloc` memory, whose tree the library's allocator can free).
"""
Value(root::Ptr{GValue}) = adopt(root)

Base.close(v::Value) = release!(v)
Base.isopen(v::Value) = v.root != C_NULL

"""
A read-only (or, lent for a call, writable) owner for memory this binding
does not free: found again by `find` on every access when one is given,
kept valid while `keep` is open and, for a call's view, while `scope` is.
"""
mutable struct View <: AbstractOwner
    node::Ptr{GValue}
    find::Any
    keep::Any
    scope::Any
    alloc::Ptr{GAlloc}
end
View(node::Ptr{GValue}; find = nothing, keep = nothing, scope = nothing, alloc::Ptr{GAlloc} = Ptr{GAlloc}(C_NULL)) =
    View(node, find, keep, scope, alloc)

root_node(v::Value) = (r = v.root; r == C_NULL && throw(closed_error()); r)
function root_node(v::View)
    v.keep === nothing || root_node(v.keep)
    v.scope === nothing || check_scope(v.scope)
    node = v.find === nothing ? v.node : v.find()::Ptr{GValue}
    node == C_NULL && throw(InvalidStateException("the borrowed value is gone", :gone))
    return node
end
writable_alloc(v::Value) = (root_node(v); v.alloc)
function writable_alloc(v::View)
    root_node(v)
    v.alloc == C_NULL && throw(read_only_error())
    return v.alloc
end
clone_alloc(v::Value) = v.alloc
clone_alloc(::View) = binding_alloc()

"A key (a map's) or a 0-based position (a list's) on a path from the root."
const Key = Union{String,Int}

"""
    ValueRef

A node inside a tree: its owner and a path of keys and positions, found
again on every access. `n[]` is its plain Julia data; `n["key"]` and
`n[i]` (from 1) extend the path.
"""
struct ValueRef
    owner::AbstractOwner
    path::Vector{Key}
end

"""
    MapRef

A `ValueRef` onto a map, and an `AbstractDict{String,ValueRef}`: keys in
insertion order, `m[k] = x` replaces in place, `delete!`, `empty!`,
`merge!(m, src)`.
"""
struct MapRef <: AbstractDict{String,ValueRef}
    owner::AbstractOwner
    path::Vector{Key}
end

"""
    ListRef

A `ValueRef` onto a list, and an `AbstractVector{ValueRef}` (from 1):
`push!`, `deleteat!`, `empty!`; `l[i] = x` and `insert!` rebuild the
list, since the C surface only appends and removes.
"""
struct ListRef <: AbstractVector{ValueRef}
    owner::AbstractOwner
    path::Vector{Key}
end

"Anything that reads as a node: a `Value` or a view into one."
const Node = Union{Value,ValueRef,MapRef,ListRef}

const ROOT_PATH = Key[]

# A node pointer is valid only while its owner is reachable: `@held n
# expr` roots `n`'s owner (a view roots what it keeps) for `expr`, since a
# Value whose last use has passed may be freed by its finalizer.
macro held(n, ex)
    return esc(quote
        let __owner = $(@__MODULE__).owner_of($n)
            GC.@preserve __owner $ex
        end
    end)
end
owner_of(v::Value) = v
owner_of(r::Union{ValueRef,MapRef,ListRef}) = r.owner
path_of(::Value) = ROOT_PATH
path_of(r::Union{ValueRef,MapRef,ListRef}) = r.path

# ---- reading, through the header's layout ------------------------------------

@inline function entry_key_equals(e::Ptr{GEntry}, key::String)
    k = unsafe_load(Ptr{GString}(e))
    k.len == sizeof(key) || return false
    k.len == 0 && return true
    return GC.@preserve key ccall(:memcmp, Cint, (Ptr{UInt8}, Ptr{UInt8}, Csize_t), k.ptr, pointer(key), k.len) == 0
end

"The value under `key` in the map `p`, or C_NULL."
function map_find(p::Ptr{GValue}, key::String)
    load_tag(p) == TAG_MAP || return Ptr{GValue}(C_NULL)
    m = unsafe_load(payload_ptr(GMap, p))
    for i in 0:Int(m.len)-1
        e = m.ptr + i * sizeof(GEntry)
        entry_key_equals(e, key) && return Ptr{GValue}(e + ENTRY_VALUE)
    end
    return Ptr{GValue}(C_NULL)
end

"The element at the 0-based `i` of the list `p`, or C_NULL."
function list_at(p::Ptr{GValue}, i::Int)
    load_tag(p) == TAG_LIST || return Ptr{GValue}(C_NULL)
    l = unsafe_load(payload_ptr(GList, p))
    0 <= i < l.len || return Ptr{GValue}(C_NULL)
    return l.ptr + i * sizeof(GValue)
end

map_len(p::Ptr{GValue}) = load_tag(p) == TAG_MAP ? Int(unsafe_load(payload_ptr(GMap, p)).len) : 0
list_len(p::Ptr{GValue}) = load_tag(p) == TAG_LIST ? Int(unsafe_load(payload_ptr(GList, p)).len) : 0

function map_keys(p::Ptr{GValue})
    load_tag(p) == TAG_MAP || return String[]
    m = unsafe_load(payload_ptr(GMap, p))
    return [text_of(unsafe_load(Ptr{GString}(m.ptr + i * sizeof(GEntry)))) for i in 0:Int(m.len)-1]
end

function walk(root::Ptr{GValue}, path::Vector{Key})
    node = root
    for k in path
        node = k isa String ? map_find(node, k) : list_at(node, k)
        node == C_NULL && return node
    end
    return node
end

"The node `n` names now, or C_NULL when its path names nothing."
node_ptr(n::Node) = walk(root_node(owner_of(n)), path_of(n))

function require_node(n::Node)
    p = node_ptr(n)
    p == C_NULL && throw(GuatiaoError(NotFound, "no value at $(path_text(path_of(n)))"))
    return p
end

"A path in the intake grammar: `agent[1].name`, from 0."
function path_text(path::Vector{Key})
    isempty(path) && return "the root"
    io = IOBuffer()
    for (i, k) in enumerate(path)
        if k isa Int
            print(io, "[", k, "]")
        elseif occursin(r"^[A-Za-z_][A-Za-z0-9_-]*$", k)
            i > 1 && print(io, ".")
            print(io, k)
        else
            print(io, "[", k, "]")
        end
    end
    return String(take!(io))
end

"""
    path(n) -> String

Where `n` is, in the intake path grammar (positions from 0).
"""
path(n::Node) = path_text(path_of(n))

const TAG_NAMES = (:absent, :null, :bool, :number, :string, :bytes, :list, :map)

"""
    tag(n) -> Symbol

The node's kind: `:absent`, `:null`, `:bool`, `:number`, `:string`,
`:bytes`, `:list` or `:map` (`:absent` too for a path naming nothing).
"""
function tag(n::Node)
    t = @held n load_tag(node_ptr(n))
    return t < length(TAG_NAMES) ? TAG_NAMES[t+1] : :unknown
end
isabsent(n::Node) = tag(n) === :absent
isnull(n::Node) = tag(n) === :null

const MAX_DEPTH = 128

function number_text(p::Ptr{GValue})
    load_tag(p) == TAG_NUMBER || return nothing
    return text_of(unsafe_load(payload_ptr(GString, p)))
end

function auto_number(text::String)
    if any(c -> c == '.' || c == 'e' || c == 'E', text)
        return parse(Float64, text)
    end
    n = tryparse(Int64, text)
    return n === nothing ? parse(BigInt, text) : n
end

"The plain Julia data under `p`: `nothing`, `Bool`, numbers, `String`, `Vector{UInt8}`, `Vector{Any}`, `Dict{String,Any}`."
function plain(p::Ptr{GValue}, depth::Int = 0)
    depth > MAX_DEPTH && throw(GuatiaoError(BadValue, "a tree deeper than $MAX_DEPTH containers"))
    t = load_tag(p)
    t == TAG_ABSENT && return absent
    t == TAG_NULL && return nothing
    t == TAG_BOOL && return unsafe_load(Ptr{Bool}(p + PAYLOAD))
    t == TAG_NUMBER && return auto_number(text_of(unsafe_load(payload_ptr(GString, p))))
    t == TAG_STRING && return text_of(unsafe_load(payload_ptr(GString, p)))
    if t == TAG_BYTES
        b = unsafe_load(payload_ptr(GBuffer, p))
        return b.len == 0 ? UInt8[] : copy(unsafe_wrap(Vector{UInt8}, b.ptr, b.len))
    end
    if t == TAG_LIST
        l = unsafe_load(payload_ptr(GList, p))
        return Any[plain(l.ptr + i * sizeof(GValue), depth + 1) for i in 0:Int(l.len)-1]
    end
    if t == TAG_MAP
        m = unsafe_load(payload_ptr(GMap, p))
        d = Dict{String,Any}()
        for i in 0:Int(m.len)-1
            e = m.ptr + i * sizeof(GEntry)
            d[text_of(unsafe_load(Ptr{GString}(e)))] = plain(Ptr{GValue}(e + ENTRY_VALUE), depth + 1)
        end
        return d
    end
    return absent   # a tag this binding does not know: skipped, as the header asks
end

"`n[]`: the node's plain Julia data (`Guatiao.absent` for nothing there)."
Base.getindex(n::Node) = @held n plain(node_ptr(n))

# ---- navigating ------------------------------------------------------------------

function child(n::Node, key::AbstractString)
    k = String(key)
    @held n begin
        p = require_node(n)
        load_tag(p) == TAG_MAP || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a map"))
        map_find(p, k) == C_NULL && throw(KeyError(k))
    end
    return ValueRef(owner_of(n), Key[path_of(n); k])
end

function child(n::Node, i::Integer)
    len = @held n begin
        p = require_node(n)
        load_tag(p) == TAG_LIST || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a list"))
        list_len(p)
    end
    1 <= i <= len || throw(BoundsError(n, i))
    return ValueRef(owner_of(n), Key[path_of(n); Int(i) - 1])
end

Base.getindex(n::Union{Value,ValueRef}, key::AbstractString) = child(n, key)
Base.getindex(n::Union{Value,ValueRef}, i::Integer) = child(n, i)
Base.getindex(m::MapRef, key::AbstractString) = child(m, key)
Base.getindex(l::ListRef, i::Int) = child(l, i)

function MapRef(n::Node)
    @held(n, load_tag(require_node(n))) == TAG_MAP || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a map"))
    return MapRef(owner_of(n), path_of(n))
end
function ListRef(n::Node)
    @held(n, load_tag(require_node(n))) == TAG_LIST || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a list"))
    return ListRef(owner_of(n), path_of(n))
end
ValueRef(n::Node) = ValueRef(owner_of(n), path_of(n))
ValueRef(r::ValueRef) = r

Base.length(m::MapRef) = @held m map_len(node_ptr(m))
Base.haskey(m::MapRef, key::AbstractString) = (k = String(key); @held m map_find(node_ptr(m), k) != C_NULL)
Base.get(m::MapRef, key::AbstractString, default) =
    haskey(m, key) ? ValueRef(m.owner, Key[m.path; String(key)]) : default
Base.keys(m::MapRef) = @held m map_keys(require_node(m))
Base.values(m::MapRef) = [m[k] for k in keys(m)]
function Base.iterate(m::MapRef, state = (keys(m), 1))
    ks, i = state
    i > length(ks) && return nothing
    return (ks[i] => ValueRef(m.owner, Key[m.path; ks[i]]), (ks, i + 1))
end

Base.size(l::ListRef) = (@held(l, list_len(node_ptr(l))),)
Base.IndexStyle(::Type{ListRef}) = IndexLinear()

# ---- writing -------------------------------------------------------------------------

"Builds the Julia data `x` into `raw` (absent or null); on failure `raw` is left freed."
function encode!(raw::Ptr{GValue}, x, alloc::Ptr{GAlloc})
    try
        encode_node!(raw, x, alloc, 0)
    catch
        guatiao_value_free(raw)
        rethrow()
    end
    return nothing
end

"""
    lower(x)

What `x` is written as when it is not data this package knows: define
`Guatiao.lower(x::MyType)` to answer plain data (a `Dict`, a `String`,
...). The default answers `x` itself.
"""
lower(x) = x

function float_text(x::AbstractFloat)
    isfinite(x) || throw(GuatiaoError(BadValue, "$x is not a finite number"))
    x isa Float64 && return string(x)
    x isa BigFloat && return string(x)
    s = string(Float32(x))
    s = replace(s, "f" => "e")
    return endswith(s, "e0") ? s[1:end-2] : s
end

function build_text!(raw, alloc, s::AbstractString, number::Bool)
    str = String(s)
    status = GC.@preserve str number ? guatiao_value_number(alloc, gstr(str), raw) : guatiao_value_string(alloc, gstr(str), raw)
    check(status, number ? "not a number: the JSON number grammar" : "not UTF-8 text")
end

function build_child_into!(push::Function, x, alloc, depth)
    tmp = new_root()
    try
        encode_node!(tmp, x, alloc, depth + 1)
        push(tmp)
    finally
        guatiao_value_free(tmp)
        Libc.free(tmp)
    end
end

map_key_text(k::AbstractString) = String(k)
map_key_text(k::Symbol) = String(k)
map_key_text(k::Bool) = k ? "true" : "false"
map_key_text(k::Integer) = string(k)
map_key_text(k::AbstractChar) = string(k)
map_key_text(k) = throw(ArgumentError("a map key is text, a symbol, an integer, a Bool or a Char, not $(typeof(k))"))

function build_map!(raw, pairs, alloc, depth)
    check(guatiao_value_map(alloc, raw))
    for (k, v) in pairs
        key = map_key_text(k)
        build_child_into!(v, alloc, depth) do tmp
            GC.@preserve key check(guatiao_map_set(alloc, raw, gstr(key), tmp))
        end
    end
end

function build_list!(raw, items, alloc, depth)
    check(guatiao_value_list(alloc, raw))
    for v in items
        build_child_into!(v, alloc, depth) do tmp
            check(guatiao_list_push(alloc, raw, tmp))
        end
    end
end

function encode_node!(raw::Ptr{GValue}, x, alloc::Ptr{GAlloc}, depth::Int)
    depth > MAX_DEPTH && throw(GuatiaoError(BadValue, "a tree deeper than $MAX_DEPTH containers"))
    if x === nothing
        check(guatiao_value_null(raw))
    elseif x isa Absent
        check(guatiao_value_absent(raw))
    elseif x isa Bool
        check(guatiao_value_bool(x, raw))
    elseif x isa Integer
        build_text!(raw, alloc, string(x), true)
    elseif x isa AbstractFloat
        build_text!(raw, alloc, float_text(x), true)
    elseif x isa NumberText
        build_text!(raw, alloc, x.text, true)
    elseif x isa AbstractString || x isa Symbol || x isa AbstractChar
        build_text!(raw, alloc, string(x), false)
    elseif x isa Union{Vector{UInt8},Base.CodeUnits{UInt8}}
        b = Vector{UInt8}(x)
        GC.@preserve b check(guatiao_value_bytes(alloc, gbytes(b), raw))
    elseif x isa Node
        @held x check(guatiao_value_clone(alloc, require_node(x), raw))
    elseif x isa AbstractDict
        build_map!(raw, x, alloc, depth)
    elseif x isa NamedTuple
        build_map!(raw, pairs(x), alloc, depth)
    elseif x isa Union{AbstractVector,Tuple,AbstractSet}
        build_list!(raw, x, alloc, depth)
    elseif x isa Base.Enum
        build_text!(raw, alloc, string(Symbol(x)), false)
    elseif encode_special!(raw, x, alloc)
        nothing
    else
        y = lower(x)
        if y !== x
            encode_node!(raw, y, alloc, depth)
        elseif isstructtype(typeof(x)) && !(x isa Function) && !(x isa Ptr)
            T = typeof(x)
            if fieldcount(T) == 0
                check(guatiao_value_null(raw))
            else
                fields = Pair{Symbol,Any}[]
                for (i, name) in enumerate(fieldnames(T))
                    v = getfield(x, i)
                    v === nothing && Nothing <: fieldtype(T, i) && continue
                    push!(fields, name => v)
                end
                build_map!(raw, fields, alloc, depth)
            end
        else
            throw(ArgumentError("cannot write a $(typeof(x)) as a guatiao value"))
        end
    end
    return nothing
end

# Dates, times, spans and exact lengths add methods (time.jl).
encode_special!(raw, x, alloc) = false

function as_map_node(n::Node)
    p = require_node(n)
    load_tag(p) == TAG_MAP || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a map"))
    return p
end
function as_list_node(n::Node)
    p = require_node(n)
    load_tag(p) == TAG_LIST || throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a list"))
    return p
end

function Base.setindex!(n::Union{Value,ValueRef,MapRef}, x, key::AbstractString)
    alloc = writable_alloc(owner_of(n))
    k = String(key)
    # Built before the node is found: building can run code that changes the tree.
    build_child_into!(x, alloc, 0) do tmp
        @held n GC.@preserve k check(guatiao_map_set(alloc, as_map_node(n), gstr(k), tmp))
    end
    return n
end

function Base.delete!(n::Union{Value,ValueRef,MapRef}, key::AbstractString)
    writable_alloc(owner_of(n))
    k = String(key)
    status = @held n GC.@preserve k guatiao_map_discard(as_map_node(n), gstr(k))
    status == UInt32(NotFound) || check(status)
    return n
end

function Base.empty!(n::Union{Value,ValueRef,MapRef,ListRef})
    writable_alloc(owner_of(n))
    @held n begin
        p = require_node(n)
        t = load_tag(p)
        t == TAG_MAP ? check(guatiao_map_clear(p)) :
        t == TAG_LIST ? check(guatiao_list_clear(p)) :
        throw(GuatiaoError(WrongKind, "at $(path_text(path_of(n))): not a map or a list"))
    end
    return n
end

"`merge!(m, src)`: copies every entry of the map `src`, replacing collisions in place."
function Base.merge!(n::Union{Value,ValueRef,MapRef}, src::Node)
    alloc = writable_alloc(owner_of(n))
    @held n @held src check(guatiao_map_copy_from(alloc, as_map_node(n), require_node(src)))
    return n
end

function Base.push!(n::Union{Value,ValueRef,ListRef}, xs...)
    alloc = writable_alloc(owner_of(n))
    for x in xs
        build_child_into!(x, alloc, 0) do tmp
            @held n check(guatiao_list_push(alloc, as_list_node(n), tmp))
        end
    end
    return n
end

function Base.deleteat!(n::Union{Value,ValueRef,ListRef}, i::Integer)
    writable_alloc(owner_of(n))
    @held n begin
        p = as_list_node(n)
        1 <= i <= list_len(p) || throw(BoundsError(n, i))
        check(guatiao_list_discard(p, Csize_t(i - 1)))
    end
    return n
end

# The C surface appends and removes: setting or inserting at a position
# clones every element, clears the list and pushes the new order back.
function rebuild_list!(n, at::Int, x, replace::Bool)
    alloc = writable_alloc(owner_of(n))
    held = Ptr{GValue}[]   # every root made here; a pushed one holds null
    @held n try
        p = as_list_node(n)
        order = Ptr{GValue}[]
        for i in 0:list_len(p)-1
            c = new_root()
            push!(held, c)
            check(guatiao_value_clone(alloc, list_at(p, i), c))
            push!(order, c)
        end
        fresh = new_root()
        push!(held, fresh)
        encode!(fresh, x, alloc)
        p = as_list_node(n)
        replace ? (order[at] = fresh) : insert!(order, at, fresh)
        check(guatiao_list_clear(p))
        for c in order
            check(guatiao_list_push(alloc, as_list_node(n), c))
        end
    finally
        foreach(free_root, held)
    end
    return n
end

function Base.setindex!(l::ListRef, x, i::Int)
    1 <= i <= length(l) || throw(BoundsError(l, i))
    rebuild_list!(l, i, x, true)
    return l
end
function Base.insert!(l::Union{Value,ValueRef,ListRef}, i::Integer, x)
    len = @held l list_len(as_list_node(l))
    1 <= i <= len + 1 || throw(BoundsError(l, i))
    rebuild_list!(l, Int(i), x, false)
    return l
end

"`copy(n)`: a deep, independent copy of the node, as a `Value`."
function Base.copy(n::Node)
    alloc = clone_alloc(owner_of(n))
    root = new_root()
    status = @held n guatiao_value_clone(alloc, require_node(n), root)
    status == 0 || (Libc.free(root); check(status))
    return adopt(root, alloc)
end

function Base.show(io::IO, v::Value)
    if !isopen(v)
        print(io, "Guatiao.Value(closed)")
    else
        print(io, "Guatiao.Value(")
        show(io, try GC.@preserve(v, plain(root_node(v))) catch; "?" end)
        print(io, ")")
    end
end
function Base.show(io::IO, r::ValueRef)
    print(io, "Guatiao.ValueRef(", path(r), ")")
end
Base.show(io::IO, ::MIME"text/plain", r::Union{MapRef,ListRef}) = show(io, r)
Base.show(io::IO, r::MapRef) = print(io, "Guatiao.MapRef(", path(r), ", ", length(r), " keys)")
Base.show(io::IO, r::ListRef) = print(io, "Guatiao.ListRef(", path(r), ", ", length(r), " elements)")

"""
A node moved out of `v` into a fresh root: `v` then holds absent and frees
nothing. For handing a tree over to a callee.
"""
function take_root!(v::Value)
    moved = new_root()
    GC.@preserve v begin
        root = root_node(v)
        unsafe_store!(moved, unsafe_load(root))
        unsafe_store!(root, ZERO_VALUE)
    end
    return moved
end
