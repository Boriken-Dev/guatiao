# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A kind read from its description: the document a kind crate commits
# beside its header (`<crate>.kinds.json`), or the one a provider carries,
# turned into the `@kind` declarations a person would write from it.

@cdef "guatiao" "" :plain guatiao_kind_document_read(text::GStr, a::PA, out::PV, why::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_kind_descriptions(reg::P, key::GStr, a::PA, out::PV)::UInt32

"""
    kind_descriptions(reg, key) -> Union{Value,Nothing}

The document of the kinds the provider filed under `key` serves and of
every object kind their slots take or answer (`{"kinds": [...]}`), as a
`Value` the caller owns, or `nothing` when it carries none:
what `kinds_of` loads a kind from when no file of it is at hand.
`NotFound` when nothing answers to `key`, `BadValue` when what it carries
is not a description.
"""
function kind_descriptions(reg::Registry, key::AbstractString)
    out = new_root()
    status = GC.@preserve reg with_text(k -> guatiao_registry_kind_descriptions(handle(reg), k, reg.alloc, out), key)
    if status != 0
        free_root(out)
        check(status)
    end
    document = adopt(out, reg.alloc)
    isabsent(document) || return document
    close(document)
    return nothing
end

"The document as plain data: read through the library from JSON text, or taken from a `Value` or a `Dict`."
function document_data(document::AbstractString)
    alloc = binding_alloc()
    out, why = new_root(), new_root()
    status = with_text(t -> guatiao_kind_document_read(t, alloc, out, why), document)
    if status != 0
        message = load_tag(why) == TAG_STRING ? text_of(unsafe_load(payload_ptr(GString, why))) : "not a kinds document"
        free_root(out)
        free_root(why)
        throw(GuatiaoError(status, message))
    end
    free_root(why)
    value = adopt(out, alloc)
    try
        return value[]
    finally
        close(value)
    end
end
document_data(document::Value) = document[]
document_data(document::AbstractDict) = document

const DESCRIBED_SCALARS = Dict("bool" => :Bool, "i8" => :Int8, "i16" => :Int16, "i32" => :Int32, "i64" => :Int64,
    "u8" => :UInt8, "u16" => :UInt16, "u32" => :UInt32, "u64" => :UInt64, "isize" => :Int, "usize" => :UInt,
    "f32" => :Float32, "f64" => :Float64)
const DESCRIBED_TREES = Dict("value" => :ValueRef, "map" => :MapRef, "list" => :ListRef)

"`greet_into` as a type name: `GreetInto`."
type_name(kind::AbstractString) = Symbol(join(uppercasefirst.(filter(!isempty, split(kind, '_')))))

function described_arg(a, types, where)
    ty, pass = get(a, "type", nothing), get(a, "pass", nothing)
    haskey(DESCRIBED_SCALARS, ty) && return DESCRIBED_SCALARS[ty]
    ty == "str" && return :String
    ty == "bytes" && return :(Vector{UInt8})
    ty == "bytes_mut" && return :(Lent{Vector{UInt8}})
    if ty == "object"
        pass in ("taken", "lent") || throw(ArgumentError("$where: an object is passed taken or lent, not $(repr(pass))"))
        return Expr(:curly, pass == "taken" ? :Taken : :Lent, types[a["kind"]])
    end
    haskey(DESCRIBED_TREES, ty) || throw(ArgumentError("$where: $(repr(ty)) is no argument type"))
    tree = DESCRIBED_TREES[ty]
    if pass == "borrowed"
        ty == "list" && throw(ArgumentError("$where: a list cannot be passed borrowed"))
        return tree
    end
    pass in ("taken", "lent") || throw(ArgumentError("$where: a $ty is passed borrowed, taken or lent, not $(repr(pass))"))
    return Expr(:curly, pass == "taken" ? :Taken : :Lent, tree)
end

function described_answer(a, types, where)
    a === nothing && return :Nothing
    ty = get(a, "type", nothing)
    haskey(DESCRIBED_SCALARS, ty) && return DESCRIBED_SCALARS[ty]
    ty == "text" && return :String
    haskey(DESCRIBED_TREES, ty) && return DESCRIBED_TREES[ty]
    ty == "object" && return types[a["kind"]]
    throw(ArgumentError("$where: an answer cannot be $(repr(ty))"))
end

"Every kind a slot of `k` names, in order."
named_kinds(k) = [one["kind"] for s in get(k, "slots", []) for one in [get(s, "args", []); something(get(s, "answer", nothing), Dict())]
                  if get(one, "type", nothing) == "object"]

"The kinds with every object kind a slot names before the kind naming it, else in document order."
function ordered_kinds(kinds, outside)
    by_name = Dict{String,Any}()
    for k in kinds
        name = get(k, "name", nothing)
        name isa AbstractString || throw(ArgumentError("a kind without a name"))
        haskey(by_name, name) && throw(ArgumentError("the document describes $name twice"))
        by_name[name] = k
    end
    out, done, making = Any[], Set{String}(), String[]
    function visit(name)
        name in done && return
        name in making && throw(ArgumentError("$name refers to itself through its slots, which a description cannot load here"))
        push!(making, name)
        for named in named_kinds(by_name[name])
            if haskey(by_name, named)
                visit(named)
            elseif !(named in outside)
                throw(ArgumentError("$name: no kind $named in the document or among the known kinds"))
            end
        end
        pop!(making)
        push!(done, name)
        push!(out, by_name[name])
    end
    foreach(k -> visit(k["name"]), kinds)
    return out
end

"""
    kind_declarations(document; known = ()) -> Vector{Expr}

The `@kind` declaration of each kind of a description document (its JSON
text, the `Value` `kind_descriptions` answers, or plain data), each object
kind before the kinds naming it: what a person would write from the
description. `known` are kinds (`@kind` types) a slot may name that the
document does not hold.
"""
function kind_declarations(document; known = ())
    doc = document_data(document)
    haskey(doc, "kinds") || throw(ArgumentError("a kinds document is {\"kinds\": [...]}"))
    kinds = doc["kinds"]
    types = Dict{String,Any}(kind(k).name => k for k in known)
    for k in kinds
        types[k["name"]] = type_name(k["name"])
    end
    decls = Expr[]
    for k in ordered_kinds(kinds, Set(kind(x).name for x in known))
        name = k["name"]
        slots = Expr(:block)
        for s in get(k, "slots", [])
            where = "$name.$(s["name"])"
            args = [Expr(:(::), Symbol(a["name"]), described_arg(a, types, "$where($(a["name"]))")) for a in get(s, "args", [])]
            line = Expr(:(::), Expr(:call, Symbol(s["name"]), args...), described_answer(get(s, "answer", nothing), types, "$where answer"))
            get(s, "fallible", false) === true && (line = Expr(:macrocall, Symbol("@fallible"), LineNumberNode(0), line))
            get(s, "required", false) === true && (line = Expr(:macrocall, Symbol("@required"), LineNumberNode(0), line))
            push!(slots.args, line)
        end
        push!(decls, Expr(:macrocall, GlobalRef(Guatiao, Symbol("@kind")), LineNumberNode(0), type_name(name),
            :(floor_hash = $(UInt32(k["floor_hash"]))), :(name = $name), :(object = $(get(k, "object", false) === true)), slots))
    end
    return decls
end

"""
    kinds_of(document; into = Module(:Kinds), known = ()) -> Dict{String,Type}

Each kind of a description document by name, declared in `into` as
`@kind` would declare it (its abstract type, its functions, its remote):
the document a kind crate commits (`<crate>.kinds.json`, as JSON text),
the `Value` `kind_descriptions` answers, or plain data. JSON text is read
by the library, so no JSON package is needed. A slot's function is
`getfield(into, :greet)`; code compiled before the call reaches it
through `Base.invokelatest`. `ArgumentError` names the kind and the slot
for a type it does not know, a borrowed list, or a cycle.
"""
function kinds_of(document; into::Module = Module(:Kinds), known = ())
    doc = document_data(document)
    for decl in kind_declarations(doc; known = known)
        Core.eval(into, decl)
    end
    return Dict{String,Type}(k["name"] => getfield(into, type_name(k["name"])) for k in doc["kinds"])
end
