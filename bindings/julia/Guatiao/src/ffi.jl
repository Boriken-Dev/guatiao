# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    Guatiao.FFI

What a package over another guatiao C library builds on: GuatiaoSerde and
GuatiaoIntake call their own libraries and pass trees to and from this
package through what is here, as may any package binding a C library that
takes and answers guatiao values. Supported API, versioned with the
package; nothing here is exported, so name it `FFI.adopt` and so on.

- `@cdef base feature mode name(arg::CType, ...)::CRet` declares an export
  of the library carrying surface `base`, found on first use: what
  `use_library(base, path)` named, its own variable (`surface!`), then
  `GUATIAO_LIBRARY` (a directory; or a file, used when it exports the
  surface's probe, else the file beside it), then the platform's search.
- `surface!(base; variable, probe)` declares a surface's own variable and
  a symbol only it exports; `guatiao_serde` and `guatiao_intake` are known.
  `find_library(base)` is where that order lands now.
- Trees cross as their root node, a `Ptr{GValue}`: `new_root()` and
  `free_root(p)` for a root a call writes, `adopt(root, alloc)` for a
  `Value` taking it over, `with_nodes(f, args...)` lending `Value`s, views
  or plain data to a call, `require_node(x)` / `node_ptr(x)` the node a
  `Value` or view names now, `owner_of(x)` what to root with
  `GC.@preserve` while it is used, and `borrow(source, find)` a read-only
  view of what another call found. Built by one library, a tree is read
  and freed by any other, since a value records its own allocator; the
  library that built it stays loaded while it lives.
- Reading: `load_tag`, `payload_ptr`, `text_of`, `plain`, `map_keys`,
  `map_find`, the `TAG_*` constants, `ZERO_VALUE`; writing: `gstr` (a
  `GStr` over a `String` kept alive by `GC.@preserve`), `binding_alloc()`
  (the core library's default allocator); `check(status, message)` throws
  the status's `GuatiaoError`; `guatiao_value_free(p)` frees what a node
  owns.
- A whole document with no tree: `read_plain(call)` hands `call` a
  `guatiao_visitor` (a `Ptr{Cvoid}`) for a C walk and answers the plain
  data its events make, as `plain` reads a tree, with the walk's status;
  `julia_to_wire(x)` writes what `Value(x)` builds as wire bytes, in Julia.
- The C mirrors: `GValue`, `GAlloc`, `GStr`, `GString`, `GBytes`, `GVisitor`.
"""
module FFI

using ..Guatiao: Guatiao, @cdef, use_library, GValue, GAlloc, GStr, GString, GBytes, Node, Value, ValueRef, View, Key,
    ZERO_VALUE, TAG_ABSENT, TAG_NULL, TAG_BOOL, TAG_NUMBER, TAG_STRING, TAG_BYTES, TAG_LIST, TAG_MAP,
    load_tag, payload_ptr, text_of, gstr, binding_alloc, new_root, free_root, adopt, require_node, node_ptr,
    owner_of, plain, map_keys, map_find, check, guatiao_value_free, SURFACES, LOAD_LOCK, GVisitor, read_plain,
    julia_to_wire

"""
    surface!(basename; variable = "", probe = "")

Declares the surface `basename`'s own environment variable, consulted
before `GUATIAO_LIBRARY`, and `probe`, a symbol only it exports, which a
`GUATIAO_LIBRARY` file is checked for. Before the first call into it.
"""
function surface!(basename::AbstractString; variable::AbstractString = "", probe::AbstractString = "")
    lock(LOAD_LOCK) do
        SURFACES[String(basename)] = (variable = String(variable), probe = String(probe))
    end
    return nothing
end

"""
    with_nodes(f, args...)

Calls `f` with a `Ptr{GValue}` for each argument: a `Value` or view as it
is (its owner rooted for the call), plain data built into a `Value` for
the call and closed after it.
"""
function with_nodes(f, args...)
    owned = Value[]
    keep = Any[]
    try
        ptrs = map(args) do a
            if a isa Node
                push!(keep, owner_of(a))
                require_node(a)
            else
                v = Value(a)
                push!(owned, v)
                require_node(v)
            end
        end
        return GC.@preserve keep owned f(ptrs...)
    finally
        foreach(close, owned)
    end
end

"""
    borrow(source, find) -> ValueRef

A read-only view of the node `find()` answers, asked again on every
access, which holds `source` (a `Value` or view) and refuses to read once
it is closed. `find` answers `C_NULL` once the node is gone.
"""
borrow(source::Node, find) = ValueRef(View(Ptr{GValue}(C_NULL); find, keep = owner_of(source)), Key[])

"""
    find_library(basename) -> String

Where the library carrying surface `basename` is found now, by the order
`@cdef` uses; a `LibraryNotFound` naming every place tried.
"""
find_library(basename::AbstractString) = Guatiao.resolve_path(String(basename))

end # module FFI
