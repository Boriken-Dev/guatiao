# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A host's registry: the libraries it loaded and the providers they offer,
# over guatiao_registry_*. Every call that can reach a library is made in
# a GC-safe region (native.jl), so a library calling back from a thread of
# its own never waits on this one.

"""
    Registry(id, version; alloc) -> Registry

A host's registry of libraries and providers. `close(reg)` frees it;
`Registry(f, id, version)` closes after the block. Answers that are data
are plain Julia data, already freed, built in `alloc` (the library's own
by default). Not safe for concurrent use: a host sharing one between tasks
guards it.
"""
mutable struct Registry
    handle::Ptr{Cvoid}
    alloc::Ptr{GAlloc}
    libraries::Vector{Any}   # Julia libraries registered here, kept as long as it lives
    function Registry(id::AbstractString, version::AbstractString; alloc::Ptr{GAlloc} = binding_alloc())
        i, v = String(id), String(version)
        handle = GC.@preserve i v guatiao_registry_new(gstr(i), gstr(v), alloc)
        handle == C_NULL && throw(GuatiaoError(BadValue, "guatiao_registry_new refused the id, the version or the allocator"))
        reg = new(handle, alloc, Any[])
        finalizer(free_registry!, reg)
        return reg
    end
end

function Registry(f::Function, id::AbstractString, version::AbstractString; kw...)
    reg = Registry(id, version; kw...)
    try
        return f(reg)
    finally
        close(reg)
    end
end

function free_registry!(reg::Registry)
    h = reg.handle
    h == C_NULL && return nothing
    reg.handle = C_NULL
    guatiao_registry_free(h)
    return nothing
end
Base.close(reg::Registry) = free_registry!(reg)
Base.isopen(reg::Registry) = reg.handle != C_NULL
Base.show(io::IO, reg::Registry) = print(io, isopen(reg) ? "Guatiao.Registry(open)" : "Guatiao.Registry(closed)")

handle(reg::Registry) = (h = reg.handle; h == C_NULL && throw(closed_error("registry")); h)

# One `(reg, ..., alloc, out) -> status` answer, as plain data, freed.
function answer(f, reg::Registry, args...)
    out = new_root()
    try
        status = GC.@preserve reg f(handle(reg), args..., reg.alloc, out)
        check(status)
        return plain(out)
    finally
        free_root(out)
    end
end

with_text(f, s::AbstractString) = (t = String(s); GC.@preserve t f(gstr(t)))

"`load_file(reg, path)`: loads one library file; the report of what loaded and what was skipped."
load_file(reg::Registry, path::AbstractString) = with_text(p -> answer(guatiao_registry_load_file, reg, p), string(path))

"""
    register_entry(reg, name, entry::Ptr{Cvoid})

Registers a library from its `guatiao_library_entry` (a C function
pointer: a library linked into the host, or one written in Julia).
"""
register_entry(reg::Registry, name::AbstractString, entry::Ptr{Cvoid}) =
    with_text(n -> answer(guatiao_registry_register_entry, reg, n, entry), name)

"""
    scan_dir(reg, dir; rules = nothing, descending = false)

Loads every library file in `dir` whose declarations pass `rules` (one
`[!]KEY=VALUE` per line, matched against what a library declares before
it is mapped). The report of what loaded and what was skipped.
"""
function scan_dir(reg::Registry, dir::AbstractString; rules = nothing, descending::Bool = false)
    return with_text(string(dir)) do d
        rules === nothing ? answer(guatiao_registry_scan_dir, reg, d, descending) :
        with_text(r -> answer(guatiao_registry_scan_dir_rules, reg, d, descending, r), rules)
    end
end

"`scan_path(reg, spec; rules, descending)`: `scan_dir` over every directory of a search path."
scan_path(reg::Registry, spec::AbstractString; rules = "", descending::Bool = false) =
    with_text(s -> with_text(r -> answer(guatiao_registry_scan_path, reg, s, descending, r), something(rules, "")), spec)

"`libraries(reg)`: every library loaded, as plain data."
libraries(reg::Registry) = answer(guatiao_registry_libraries, reg)
"`providers(reg, kind = \"\")`: every provider of `kind` (or of any), best first."
providers(reg::Registry, kind::AbstractString = "") = with_text(k -> answer(guatiao_registry_providers, reg, k), kind)
"`available(reg, kind = \"\")`: the providers of `kind` that can run here."
available(reg::Registry, kind::AbstractString = "") = with_text(k -> answer(guatiao_registry_available, reg, k), kind)
"`why_not(reg, kind)`: why nothing of `kind` is available, or what is."
why_not(reg::Registry, kind::AbstractString) = with_text(k -> answer(guatiao_registry_why_not, reg, k), kind)
"`provider(reg, key)`: one provider, as plain data."
provider(reg::Registry, key::AbstractString) = with_text(k -> answer(guatiao_registry_provider, reg, k), key)
"`best(reg, kind)`: the best provider of `kind`."
best(reg::Registry, kind::AbstractString) = with_text(k -> answer(guatiao_registry_best, reg, k), kind)

"""
    provider_available(reg, key) -> (Bool, Union{Nothing,String})

Whether the provider can run here, and why not when it cannot.
"""
function provider_available(reg::Registry, key::AbstractString)
    reason = Ref(GStr(C_NULL, 0))
    ok = GC.@preserve reg reason with_text(k -> guatiao_registry_provider_available(handle(reg), k, reason), key)
    return ok ? (true, nothing) : (false, text_of(reason[]))
end

"`keyed_by!(reg, template)`: files providers under `template` (`%id` by default; `%name`, `%library`, `%version`)."
keyed_by!(reg::Registry, template::AbstractString) =
    (GC.@preserve reg with_text(t -> check(guatiao_registry_keyed_by(handle(reg), t)), template); reg)
"`libraries_keyed_by!(reg, template)`: files libraries under `template` (`%id` by default)."
libraries_keyed_by!(reg::Registry, template::AbstractString) =
    (GC.@preserve reg with_text(t -> check(guatiao_registry_libraries_keyed_by(handle(reg), t)), template); reg)
"`set_priority!(reg, id, priority)`: ranks a provider among those of its kinds."
set_priority!(reg::Registry, id::AbstractString, priority::Integer) =
    (GC.@preserve reg with_text(i -> check(guatiao_registry_set_priority(handle(reg), i, Int32(priority))), id); reg)
"`priority(reg, id)`: a provider's priority."
priority(reg::Registry, id::AbstractString) = GC.@preserve reg with_text(i -> Int(guatiao_registry_priority(handle(reg), i)), id)

"""
    provider_table(reg, key; kind = nothing) -> (table, size, ctx)

The table the provider filed under `key` speaks `kind` through (without
`kind`, its single table); `table` is `C_NULL` when there is none.
"""
function provider_table(reg::Registry, key::AbstractString; kind = nothing)
    size = Ref{Csize_t}(0)
    return GC.@preserve reg size with_text(key) do k
        table = kind === nothing ? guatiao_registry_provider_vtable(handle(reg), k, size) :
                with_text(kn -> guatiao_registry_provider_table(handle(reg), k, kn, size), kind)
        ctx = guatiao_registry_provider_ctx(handle(reg), k)
        (table, Int(size[]), ctx)
    end
end

"""
    bind(reg, key, K)

The provider filed under `key`, as the kind `K`: its remote, calling the
provider's table with its own ctx (the provider's one instance).
`NotFound` when it serves no table of the kind; `FloorMismatch` when the
table is not the one `K` describes. Valid while the registry is open and
the library loaded.
"""
function bind(reg::Registry, key::AbstractString, ::Type{K}) where {K<:Kind}
    spec = kind(K)
    table, size, ctx = provider_table(reg, key; kind = spec.name)
    table == C_NULL && throw(GuatiaoError(NotFound, "`$key` serves no $(spec.name) table"))
    return bind(K, table, size, ctx; keep = reg)
end

"""
    provider_config(reg, key) -> Union{ValueRef,Nothing}

A read-only view of the provider's configuration schema, borrowed from
the registry, or `nothing` when it declares none.
"""
function provider_config(reg::Registry, key::AbstractString)
    find = () -> GC.@preserve reg with_text(k -> guatiao_registry_provider_config(handle(reg), k), key)
    find() == C_NULL && return nothing
    return ValueRef(View(Ptr{GValue}(C_NULL); find, keep = reg), Key[])
end

"""
An instance `create` built: `close(inst)` runs its provider's `destroy`,
even after a retire or a refresh loaded a new build under the key.
"""
mutable struct Instance
    registry::Registry
    key::String
    ctx::Ptr{Cvoid}
    open::Bool
end
Base.isopen(inst::Instance) = inst.open && isopen(inst.registry)
function Base.close(inst::Instance)
    inst.open || return nothing
    inst.open = false
    reg = inst.registry
    isopen(reg) || return nothing
    GC.@preserve reg with_text(k -> guatiao_registry_provider_destroy(handle(reg), k, inst.ctx), inst.key)
    return nothing
end
Base.show(io::IO, inst::Instance) = print(io, "Guatiao.Instance(", inst.key, isopen(inst) ? ")" : ", closed)")

"""
    create(reg, key, config) -> Instance

Builds an instance of the provider filed under `key` from `config` (a
Value, a view or plain data), checked against the provider's schema
first (`BadValue` naming the field).
"""
function create(reg::Registry, key::AbstractString, config)
    owned = config isa Node ? nothing : Value(config; alloc = reg.alloc)
    src = owned === nothing ? config : owned
    keep = owner_of(src)
    out = Ref{Ptr{Cvoid}}(C_NULL)
    err = Ref(GProviderError(0, NO_STRING))
    try
        status = GC.@preserve reg keep out err with_text(k -> guatiao_registry_provider_create(handle(reg), k, require_node(src), out, err), key)
        finish_call(status, err, true, "create")
    finally
        owned === nothing || close(owned)
    end
    inst = Instance(reg, String(key), out[], true)
    finalizer(close, inst)
    return inst
end

"""
    bind(inst, K)

The instance as the kind `K`: a remote whose calls go to this instance,
valid until it is closed.
"""
function bind(inst::Instance, ::Type{K}) where {K<:Kind}
    spec = kind(K)
    table, size, _ = provider_table(inst.registry, inst.key; kind = spec.name)
    table == C_NULL && throw(GuatiaoError(NotFound, "`$(inst.key)` serves no $(spec.name) table"))
    return bind(K, table, size, inst.ctx; keep = inst)
end

key_call(f, reg::Registry, key::AbstractString) = GC.@preserve reg with_text(k -> check(f(handle(reg), k)), key)

"""
    retire!(reg, key)

Takes the library `key` names (the LIBRARY key, `%id` by default) out of
the registry and leaves it mapped until nothing uses it. `NotFound` for
none.
"""
retire!(reg::Registry, key::AbstractString) = (key_call(guatiao_registry_retire, reg, key); nothing)

"""
    collect!(reg) -> Vector

Closes every retired library nothing uses any more and whose own `unload`
agrees, and answers their keys.
"""
collect!(reg::Registry) = answer(guatiao_registry_collect, reg)

"""
    unload!(reg, key; unchecked = false)

Unmaps a library (by its LIBRARY key) when it agrees: `Busy` while
something it handed out lives, `Unsupported` with no `unload` function,
`WrongKind` for a library linked rather than mapped, `NotFound` for none.
`unchecked = true` insists past a missing `unload` function. No table,
ctx or borrowed schema taken from it is used again.
"""
unload!(reg::Registry, key::AbstractString; unchecked::Bool = false) =
    (key_call(unchecked ? guatiao_registry_unload_unchecked : guatiao_registry_unload, reg, key); nothing)

"""
    add_source_file!(reg, path, config = Dict()) -> String

Maps a file exporting `guatiao_source` (a bridge to another language) and
adds the source it answers for `config`, refusing with the source's own
words. Nothing it serves loads until `refresh!`. Answers the source's
name, its path.
"""
function add_source_file!(reg::Registry, path::AbstractString, config = Dict{String,Any}())
    owned = config isa Node ? nothing : Value(config; alloc = reg.alloc)
    src = owned === nothing ? config : owned
    keep = owner_of(src)
    err = Ref(GProviderError(0, NO_STRING))
    try
        status = GC.@preserve reg keep err with_text(p -> guatiao_registry_add_source_file(handle(reg), p, require_node(src), err), string(path))
        finish_call(status, err, true, "add_source_file")
    finally
        owned === nothing || close(owned)
    end
    return string(path)
end

"""
    watch_dir!(reg, dir; rules = nothing)

Watches a directory: each `refresh!` loads a library file that appeared
there, retires one that vanished, and reloads one that changed.
"""
watch_dir!(reg::Registry, dir::AbstractString; rules = nothing) =
    (GC.@preserve reg with_text(d -> with_text(r -> check(guatiao_registry_watch_dir(handle(reg), d, r)), something(rules, "")), string(dir)); nothing)

"""
    refresh!(reg) -> Dict

Asks every source and watched directory what it serves now and makes the
registry agree: `Dict("loaded" => [...], "retired" => [...], "skipped" =>
[...], "refused" => [...])`. Nothing refreshes on its own.
"""
refresh!(reg::Registry) = answer(guatiao_registry_refresh, reg)

"""
    host(reg) -> Ptr{GHostInfo}

The `guatiao_host_info *` the registry hands a library; it lives as long
as the process.
"""
host(reg::Registry) = GC.@preserve reg guatiao_registry_host(handle(reg))
