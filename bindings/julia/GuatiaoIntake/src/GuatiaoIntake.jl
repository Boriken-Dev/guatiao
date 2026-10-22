# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    GuatiaoIntake

Taking a guatiao value in from a person, through the `guatiao_intake`
library: naming a place inside a value (`at`), checking a form against a
schema, its layout, field visibility, the form a schema implies, and the
flat `key => text` projection. Every schema, form and value argument is a
Guatiao `Value`, a view, or plain data. Built on Guatiao.
"""
module GuatiaoIntake

using Guatiao: Guatiao, Value, GuatiaoError, WrongKind, BadValue
using Guatiao.FFI: FFI, @cdef, Node, GValue, GAlloc, GStr, TAG_MAP, TAG_STRING, TAG_ABSENT, load_tag, gstr,
    binding_alloc, new_root, free_root, adopt, node_ptr, require_node, plain, map_keys, map_find, with_nodes

const PV = Ptr{GValue}
const PA = Ptr{GAlloc}

@cdef "guatiao_intake" "" :plain guatiao_intake_check(schema::PV, form::PV, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_layout(schema::PV, form::PV, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_for_schema(schema::PV, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_form_for(form::PV, schema::PV, path::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_is_visible(schema::PV, form::PV, key::GStr, values::PV, out::Ptr{Bool})::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_path_get(v::PV, path::GStr)::PV
@cdef "guatiao_intake" "" :plain guatiao_intake_resolve(schema::PV, key::GStr)::PV
@cdef "guatiao_intake" "" :plain guatiao_intake_flat_keys(schema::PV, key::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_flatten(schema::PV, key::GStr, v::PV, a::PA, out::PV)::UInt32
@cdef "guatiao_intake" "" :plain guatiao_intake_unflatten(schema::PV, key::GStr, flat::PV, a::PA, out::PV)::UInt32

"""
    use_library(path)

Loads `guatiao_intake` from `path` (a file exporting its functions,
whatever it is called, or a directory holding it under its platform file
name) instead of looking for one: before the first call. Without it the
library is found through `GUATIAO_INTAKE_LIBRARY`, then
`GUATIAO_LIBRARY` (a directory, or a file exporting it, else the file
beside it), then the platform's search.
"""
use_library(path::AbstractString) = Guatiao.use_library("guatiao_intake", path)

function answer(f)
    out = new_root()
    try
        status = f(out)
        return status, out
    catch
        free_root(out)
        rethrow()
    end
end

function plain_answer(f)
    status, out = answer(f)
    try
        FFI.check(status)
        return plain(out)
    finally
        free_root(out)
    end
end

"""
    check(schema, form) -> Union{Nothing,Dict}

`nothing` when `form` fits `schema`; otherwise the error map (`kind`,
`at`, and whichever of `path`/`id`/`field`/`expected`/`message` apply).
"""
function check(schema, form)
    with_nodes(schema, form) do s, f
        status, out = answer(o -> guatiao_intake_check(s, f, binding_alloc(), o))
        try
            status == 0 && return nothing
            status == UInt32(BadValue) || FFI.check(status)
            return plain(out)
        finally
            free_root(out)
        end
    end
end

"""
    layout(schema, form) -> Vector

The schema's fields grouped into sections, in order:
`[Dict("section" => map or nothing, "fields" => [key, ...]), ...]`.
"""
layout(schema, form) = with_nodes(schema, form) do s, f
    plain_answer(o -> guatiao_intake_layout(s, f, binding_alloc(), o))
end

"Whether the field under `key` is shown, given the `values` entered so far."
function isvisible(schema, form, key::AbstractString, values)
    k = String(key)
    with_nodes(schema, form, values) do s, f, v
        out = Ref(false)
        FFI.check(GC.@preserve k out guatiao_intake_is_visible(s, f, gstr(k), v, out))
        return out[]
    end
end

function borrowed(source::Node, lookup, text::String)
    find = () -> GC.@preserve text lookup(node_ptr(source), gstr(text))
    find() == C_NULL && return nothing
    return FFI.borrow(source, find)
end

"""
    at(value::Node, path) -> Union{ValueRef,Nothing}

The node at `path` (`agent[1].name[home].host`: a dot is a member, a
bracket a list position from 0 or a map key, decided by what it is
applied to), or `nothing`. It borrows from `value`: read-only, found again
on every access, an `InvalidStateException` once `value` is closed;
`copy` it to keep it.
"""
at(value::Node, path::AbstractString) = borrowed(value, guatiao_intake_path_get, String(path))

"""
    resolve(schema::Node, key) -> Union{ValueRef,Nothing}

The schema field governing the flat `key` (one level of projection
followed: `auth.password` is the field inside the arm), borrowed from
`schema` as `at` borrows.
"""
resolve(schema::Node, key::AbstractString) = borrowed(schema, guatiao_intake_resolve, String(key))

"""
    for_schema(schema) -> Value

The form `schema` implies: one section per distinct `x-section`, ids
only, plus each member's own form where there is something in it.
"""
function for_schema(schema)
    with_nodes(schema) do s
        status, out = answer(o -> guatiao_intake_for_schema(s, binding_alloc(), o))
        status == 0 || (free_root(out); FFI.check(status))
        return adopt(out)
    end
end

"""
    form_for(form, schema, path) -> Union{Value,Nothing}

The form to show the field at `path` with: the one `form` assigns, or
the one its schema implies; `nothing` for a path naming no field, or a
field with no members and no form assigned.
"""
function form_for(form, schema, path::AbstractString)
    p = String(path)
    with_nodes(form, schema) do f, s
        status, out = answer(o -> GC.@preserve p guatiao_intake_form_for(f, s, gstr(p), binding_alloc(), o))
        status == 0 || (free_root(out); FFI.check(status))
        load_tag(out) == TAG_ABSENT && (free_root(out); return nothing)
        return adopt(out)
    end
end

"""
    flat_keys(schema, key) -> Vector{String}

The flat keys the field `key` projects onto: its own, then one per field
of every arm. `WrongKind` for a key naming no field.
"""
function flat_keys(schema, key::AbstractString)
    k = String(key)
    with_nodes(schema) do s
        String[x for x in plain_answer(o -> GC.@preserve k guatiao_intake_flat_keys(s, gstr(k), binding_alloc(), o))]
    end
end

"""
    flatten(schema, key, value) -> Dict{String,String}

`value` as the flat store the field `key` keeps: one entry per scalar
leaf (`agent[0].name`, `env[PATH]`, a tagged field's member as
`key.member`). `WrongKind` when the value's shape contradicts the field.
"""
function flatten(schema, key::AbstractString, value)
    k = String(key)
    with_nodes(schema, value) do s, v
        Dict{String,String}(a => b for (a, b) in plain_answer(o -> GC.@preserve k guatiao_intake_flatten(s, gstr(k), v, binding_alloc(), o)))
    end
end

function text_store(p::Ptr{GValue})
    load_tag(p) == TAG_MAP || return false
    return all(k -> load_tag(map_find(p, k)) == TAG_STRING, map_keys(p))
end

"""
    unflatten(schema, key, flat) -> Union{Value,Nothing}

The value the flat store `flat` (text to text: a `Dict`, a `Value` or a
view) holds for the field `key`, the reverse of `flatten`. `nothing` when
nothing is stored under it, the key names no field, or the stored shape
cannot be its kind; a store that is not text to text is an
`ArgumentError`.
"""
function unflatten(schema, key::AbstractString, flat)
    if flat isa Node
        text_store(require_node(flat)) || throw(ArgumentError("a flat store maps text to text"))
    else
        (flat isa AbstractDict && all(kv -> kv[1] isa AbstractString && kv[2] isa AbstractString, flat)) ||
            throw(ArgumentError("a flat store maps text to text"))
    end
    k = String(key)
    with_nodes(schema, flat) do s, f
        status, out = answer(o -> GC.@preserve k guatiao_intake_unflatten(s, gstr(k), f, binding_alloc(), o))
        if status == UInt32(WrongKind)
            free_root(out)
            return nothing
        end
        status == 0 || (free_root(out); FFI.check(status))
        load_tag(out) == TAG_ABSENT && (free_root(out); return nothing)
        return adopt(out)
    end
end

end # module GuatiaoIntake
