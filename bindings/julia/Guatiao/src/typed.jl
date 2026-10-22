# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Trees read into Julia types, and the JSON Schema of what a type writes.

"""
    decode(T, n)

The node `n` read as a `T`: `Bool`, every integer type (exactly, range
checked: a fraction, an exponent or a value out of range is refused, never
rounded), floats (nearest), `NumberText`, `String`, `Symbol`, `Char`,
`Vector{UInt8}` (bytes), vectors, tuples, sets, `Dict`s (text, symbol,
integer or `Bool` keys), `NamedTuple`s, `Union{Nothing,T}` (null or
nothing there is `nothing`), enums by name, the dates and times, `Value`
(a copy), `Any` (plain data), and structs, field by field (a field whose
key is missing is `nothing` when its type allows it, else its `@kwdef`
default, else refused). Keys a struct does not declare are ignored. A
refusal is a `GuatiaoError` `BadValue` naming the path.
"""
decode(::Type{T}, n::Node) where {T} = @held n decode_at(T, require_node(n), copy(path_of(n)))

refuse(path::Vector{Key}, what::AbstractString) =
    throw(GuatiaoError(BadValue, "at $(path_text(path)): $what"))

tag_name(p::Ptr{GValue}) = (t = load_tag(p); t < length(TAG_NAMES) ? string(TAG_NAMES[t+1]) : "an unknown kind")

function strip_nothing(T)
    T isa Union || return T
    a = T.a === Nothing ? Union{} : strip_nothing(T.a)
    b = T.b === Nothing ? Union{} : strip_nothing(T.b)
    return Union{a,b}
end
nullable(T) = T !== Any && T !== Nothing && Nothing <: T

decode_special(::Type, p, path) = nothing   # time.jl adds the dates, times and lengths

function decode_at(::Type{T}, p::Ptr{GValue}, path::Vector{Key}) where {T}
    length(path) > MAX_DEPTH && refuse(path, "deeper than $MAX_DEPTH containers")
    t = load_tag(p)
    T === Any && return plain(p)
    if nullable(T)
        (t == TAG_NULL || t == TAG_ABSENT) && return nothing
        return decode_at(strip_nothing(T), p, path)
    end
    T === Nothing && ((t == TAG_NULL || t == TAG_ABSENT) ? (return nothing) : refuse(path, "want null, got $(tag_name(p))"))
    if T === Value
        root = new_root()
        status = guatiao_value_clone(binding_alloc(), p, root)
        status == 0 || (Libc.free(root); check(status))
        return adopt(root)
    end
    special = decode_special(T, p, path)
    special === nothing || return something(special)
    if T === Bool
        t == TAG_BOOL || refuse(path, "want a boolean, got $(tag_name(p))")
        return unsafe_load(Ptr{Bool}(p + PAYLOAD))
    elseif T <: Integer
        text = number_text(p)
        text === nothing && refuse(path, "want an integer, got $(tag_name(p))")
        any(c -> c == '.' || c == 'e' || c == 'E', text) && refuse(path, "want an integer, got $text")
        n = tryparse(T, text)
        n === nothing && refuse(path, "$text does not fit $(T)")
        return n
    elseif T <: AbstractFloat
        text = number_text(p)
        text === nothing && refuse(path, "want a number, got $(tag_name(p))")
        x = parse(T, text)
        isfinite(x) || refuse(path, "$text does not fit $(T)")
        return x
    elseif T === NumberText
        text = number_text(p)
        text === nothing && refuse(path, "want a number, got $(tag_name(p))")
        return NumberText(text)
    elseif T <: AbstractString || T === Symbol || T <: AbstractChar
        t == TAG_STRING || refuse(path, "want a string, got $(tag_name(p))")
        s = text_of(unsafe_load(payload_ptr(GString, p)))
        T === Symbol && return Symbol(s)
        if T <: AbstractChar
            length(s) == 1 || refuse(path, "want one character, got $(length(s))")
            return T(first(s))
        end
        return convert(T, s)
    elseif T === Vector{UInt8}
        t == TAG_BYTES || refuse(path, "want bytes, got $(tag_name(p))")
        b = unsafe_load(payload_ptr(GBuffer, p))
        return b.len == 0 ? UInt8[] : copy(unsafe_wrap(Vector{UInt8}, b.ptr, b.len))
    elseif T <: Base.Enum
        t == TAG_STRING || refuse(path, "want one of $(join(string.(Symbol.(instances(T))), ", ")), got $(tag_name(p))")
        s = text_of(unsafe_load(payload_ptr(GString, p)))
        for x in instances(T)
            string(Symbol(x)) == s && return x
        end
        refuse(path, "want one of $(join(string.(Symbol.(instances(T))), ", ")), got $(repr(s))")
    elseif T <: Tuple
        t == TAG_LIST || refuse(path, "want a list, got $(tag_name(p))")
        types = fieldtypes(T)
        list_len(p) == length(types) || refuse(path, "want $(length(types)) elements, got $(list_len(p))")
        return T(Tuple(decode_at(types[i], list_at(p, i - 1), Key[path; i - 1]) for i in eachindex(types)))
    elseif T <: AbstractSet
        t == TAG_LIST || refuse(path, "want a list, got $(tag_name(p))")
        E = eltype(T)
        out = (isconcretetype(T) ? T : Set{E})()
        for i in 0:list_len(p)-1
            x = decode_at(E, list_at(p, i), Key[path; i])
            x in out && refuse(Key[path; i], "repeats an earlier element")
            push!(out, x)
        end
        return out
    elseif T <: AbstractVector
        t == TAG_LIST || refuse(path, "want a list, got $(tag_name(p))")
        E = eltype(T)
        return (isconcretetype(T) ? T : Vector{E})([decode_at(E, list_at(p, i), Key[path; i]) for i in 0:list_len(p)-1])
    elseif T <: AbstractDict
        t == TAG_MAP || refuse(path, "want a map, got $(tag_name(p))")
        K, V = keytype(T), valtype(T)
        out = (isconcretetype(T) ? T : Dict{K,V})()
        for k in map_keys(p)
            key = decode_key(K, k, path)
            haskey(out, key) && refuse(Key[path; k], "reads as a key already read")
            out[key] = decode_at(V, map_find(p, k), Key[path; k])
        end
        return out
    elseif T <: NamedTuple
        t == TAG_MAP || refuse(path, "want a map, got $(tag_name(p))")
        names = fieldnames(T)
        types = fieldtypes(T)
        vals = map(eachindex(names)) do i
            c = map_find(p, string(names[i]))
            if c == C_NULL || load_tag(c) == TAG_ABSENT
                nullable(types[i]) || types[i] === Any || refuse(path, "missing key $(names[i])")
                nothing
            else
                decode_at(types[i], c, Key[path; string(names[i])])
            end
        end
        return T(Tuple(vals))
    elseif isstructtype(T) && isconcretetype(T)
        return decode_struct(T, p, path)
    end
    refuse(path, "cannot read a $(T)")
end

function decode_key(::Type{K}, k::String, path) where {K}
    (K === Any || K <: AbstractString) && return k
    K === Symbol && return Symbol(k)
    if K === Bool
        k == "true" && return true
        k == "false" && return false
        refuse(Key[path; k], "a Bool key is true or false")
    elseif K <: Integer
        ok = occursin(K <: Signed ? r"^-?(0|[1-9][0-9]*)$" : r"^(0|[1-9][0-9]*)$", k)
        n = ok ? tryparse(K, k) : nothing
        n === nothing && refuse(Key[path; k], "not a $(K) key")
        return n
    elseif K <: AbstractChar
        length(k) == 1 || refuse(Key[path; k], "a Char key is one character")
        return K(first(k))
    end
    refuse(path, "cannot read a key as $(K)")
end

"Whether `T` has a keyword constructor naming every field (a `@kwdef` struct)."
has_kw_constructor(T) = hasmethod(T, Tuple{}, fieldnames(T))

function decode_struct(::Type{T}, p::Ptr{GValue}, path::Vector{Key}) where {T}
    load_tag(p) == TAG_MAP || refuse(path, "want a map, got $(tag_name(p))")
    names = fieldnames(T)
    types = fieldtypes(T)
    fieldcount(T) == 0 && return T()
    vals = Vector{Any}(undef, length(names))
    present = falses(length(names))
    for i in eachindex(names)
        c = map_find(p, string(names[i]))
        if c != C_NULL && load_tag(c) != TAG_ABSENT
            vals[i] = decode_at(types[i], c, Key[path; string(names[i])])
            present[i] = true
        end
    end
    all(present) && return T(vals...)
    if has_kw_constructor(T)
        kw = Dict{Symbol,Any}(names[i] => vals[i] for i in eachindex(names) if present[i])
        while true
            try
                return T(; kw...)
            catch e
                e isa UndefKeywordError || rethrow()
                i = findfirst(==(e.var), names)
                (i === nothing || !nullable(types[i])) && refuse(path, "missing key $(e.var)")
                kw[e.var] = nothing
            end
        end
    end
    for i in eachindex(names)
        present[i] && continue
        nullable(types[i]) || refuse(path, "missing key $(names[i])")
        vals[i] = nothing
    end
    return T(vals...)
end

# ---- configurations: @config, and the schema of a type -------------------------

"""
    fields_info(T) -> NamedTuple

What `@config` recorded about each field of `T`: `(; default, doc)`
where `default` is `nothing` for a field with none (a default of
`nothing` itself is `Some(nothing)`). The default method answers nothing
for every type.
"""
fields_info(::Type) = nothing

"""
    keywords(T) -> NamedTuple

Per-field JSON Schema keywords added to `schema(T)`:
`Guatiao.keywords(::Type{Listen}) = (host = (minLength = 1,),)`.
"""
keywords(::Type) = NamedTuple()

"""
    @config struct ... end

`Base.@kwdef`, which also records each field's default and docstring for
`schema`: a field with a default is optional and its schema states the
default; a docstring is its `description`.

```julia
Guatiao.@config struct Listen
    "Where to listen."
    host::String = "0.0.0.0"
    port::UInt16
end
```
"""
macro config(def)
    def isa Expr && def.head === :struct || error("@config takes a struct definition")
    name = def.args[2]
    name = name isa Expr && name.head === :(<:) ? name.args[1] : name
    name isa Expr && name.head === :curly && error("@config takes a struct without type parameters")
    names = Symbol[]
    entries = Expr[]
    doc = nothing
    for ex in def.args[3].args
        ex isa LineNumberNode && continue
        if ex isa String || (ex isa Expr && ex.head === :string)
            doc = ex
            continue
        end
        field, default = if ex isa Expr && ex.head === :(=)
            ex.args[1], ex.args[2]
        else
            ex, nothing
        end
        fname = field isa Expr && field.head === :(::) ? field.args[1] : field
        fname isa Symbol || (doc = nothing; continue)
        d = ex isa Expr && ex.head === :(=) ? :(Some($(esc(default)))) : :nothing
        push!(names, fname)
        push!(entries, :((default = $d, doc = $(doc === nothing ? "" : esc(doc)))))
        doc = nothing
    end
    info = :(NamedTuple{$(Tuple(names))}(($(entries...),)))
    return quote
        $(esc(:(Base.@kwdef $def)))
        $(@__MODULE__).fields_info(::Type{$(esc(name))}) = $info
    end
end

"""
    schema(T) -> Value

The JSON Schema (2020-12) of what a `T` is written as: a struct is an
object whose properties are its fields in order, every key it does not
declare refused, a field required unless its type allows `nothing` (which
may then also be null) or `@config` gave it a default. Integer types state
their bounds; the dates, times, spans and exact lengths their forms.
Define `Guatiao.schema_of(::Type{MyType})` to answer a type's own (plain
data, a `NamedTuple` keeping key order).
"""
schema(::Type{T}) where {T} = Value(schema_of(T))

function schema_of(::Type{T}) where {T}
    T === Any && return NamedTuple()
    T === Value && return NamedTuple()
    if nullable(T)
        return (anyOf = [schema_of(strip_nothing(T)), (type = "null",)],)
    end
    T === Nothing && return (type = "null",)
    T === Bool && return (type = "boolean",)
    T === BigInt && return (type = "integer",)
    if T <: Integer
        return (type = "integer", minimum = typemin(T), maximum = typemax(T))
    end
    (T <: AbstractFloat || T === NumberText) && return (type = "number",)
    T <: AbstractChar && return (type = "string", minLength = 1, maxLength = 1)
    (T <: AbstractString || T === Symbol) && return (type = "string",)
    T === Vector{UInt8} && return (type = "bytes",)
    T <: Base.Enum && return (type = "string", enum = [string(Symbol(x)) for x in instances(T)])
    special = schema_special(T)
    special === nothing || return special
    if T <: Tuple
        types = fieldtypes(T)
        return (type = "array", prefixItems = [schema_of(x) for x in types], items = false,
            minItems = length(types), maxItems = length(types))
    elseif T <: AbstractSet
        return (type = "array", items = schema_of(eltype(T)), uniqueItems = true)
    elseif T <: AbstractVector
        return (type = "array", items = schema_of(eltype(T)))
    elseif T <: AbstractDict
        K = keytype(T)
        s = (type = "object", additionalProperties = schema_of(valtype(T)))
        if K <: Signed
            return merge(s, (propertyNames = (type = "string", pattern = "^-?(0|[1-9][0-9]*)\$"),))
        elseif K <: Unsigned
            return merge(s, (propertyNames = (type = "string", pattern = "^(0|[1-9][0-9]*)\$"),))
        elseif K === Bool
            return merge(s, (propertyNames = (type = "string", enum = ["false", "true"]),))
        end
        return s
    elseif (T <: NamedTuple || isstructtype(T)) && isconcretetype(T)
        return struct_schema(T)
    end
    throw(ArgumentError("no schema for $(T)"))
end

schema_special(::Type) = nothing

function struct_schema(::Type{T}) where {T}
    names = fieldnames(T)
    types = fieldtypes(T)
    info = fields_info(T)
    extra = keywords(T)
    props = Pair{Symbol,Any}[]
    required = String[]
    for i in eachindex(names)
        n = names[i]
        s = schema_of(types[i])
        fi = info === nothing ? nothing : get(info, n, nothing)
        default = fi === nothing ? nothing : fi.default
        if fi !== nothing && !isempty(fi.doc)
            s = merge(s, (description = fi.doc,))
        end
        if default !== nothing
            s = merge(s, (default = something(default),))
        end
        haskey(extra, n) && (s = merge(s, extra[n]))
        push!(props, n => s)
        default === nothing && !nullable(types[i]) && push!(required, string(n))
    end
    properties = NamedTuple{Tuple(first.(props))}(Tuple(last.(props)))
    s = (type = "object", properties = properties, additionalProperties = false)
    isempty(required) || (s = merge(s, (required = required,)))
    return s
end
