# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    redact(schema, value; placeholder = "[redacted]") -> Value

A copy of `value` with every field its schema marks secret
(`x-sensitive` or `writeOnly`) replaced by `placeholder`, at any depth:
in a nested object, every element of a list, a variant's arm. Where the
schema stops describing the value (a key it does not declare), the value
is hidden too. Local `\$ref`s are followed. `schema` and `value` are
`Value`s, views, or plain data. `WrongKind` for a schema that is not a
map.
"""
function redact(schema, value; placeholder::AbstractString = "[redacted]", alloc::Ptr{GAlloc} = binding_alloc())
    owned = Value[]
    keep = Any[]
    try
        ptrs = map((schema, value)) do a
            a isa Node && (push!(keep, owner_of(a)); return require_node(a))
            v = Value(a)
            push!(owned, v)
            require_node(v)
        end
        text = String(placeholder)
        out = new_root()
        status = GC.@preserve text keep owned guatiao_schema_redact(ptrs[1], ptrs[2], gstr(text), alloc, out)
        status == 0 || (free_root(out); check(status))
        return adopt(out, alloc)
    finally
        foreach(close, owned)
    end
end
