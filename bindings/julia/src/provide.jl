# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A library written in Julia: providers are structs with methods for their
# kinds' functions, and the library is the package declaring them. Its
# descriptor is built in C memory on first use and kept for the process,
# since a host may hold it as long as it lives.

using TOML

"""
What a provider is: its Julia type, name, id, display name, version,
meta, the kinds it serves, how instances are built (`config`, a type, or
`config_schema`, plain data), whether its one instance is offered, and
what says whether it can run here.
"""
struct ProviderSpec
    type::Type
    name::String
    id::Union{Nothing,String}
    display_name::String
    version::String
    meta::Any
    kinds::Vector{DataType}
    config::Any
    config_schema::Any
    default::Bool
    available::Any
end

snake(s::AbstractString) = lowercase(replace(s, r"(?<=[a-z0-9])([A-Z])" => s"_\1"))

"The kinds `T` subtypes: what it serves when nothing names them."
function kind_supertypes(T::Type)
    found = DataType[]
    S = supertype(T)
    while S !== Any
        S <: Kind && S !== Kind && isabstracttype(S) && push!(found, S)
        S = supertype(S)
    end
    return found
end

"""
    Provider(T; serves, name, id, display_name, version, meta, config, config_schema, default, available)

The primitive under `@provider`: a provider of the kinds `serves` (default
the kinds `T` subtypes), built from `T`. With `config = C`, an instance per
configuration, `T(c::C)`, after the host checked it against
`Guatiao.schema(C)`; with `config_schema` (plain data) `T(plain)`; with
neither, `T()` is its one instance. `default = true` also offers `T()`
beside a configured provider's instances. `available` is a function
answering `nothing` when the provider can run here and why not otherwise,
asked every time a host asks.
"""
function Provider(T::Type; serves = kind_supertypes(T), name = snake(string(nameof(T))), id = nothing,
    display_name = "", version = "", meta = nothing, config = nothing, config_schema = nothing,
    default = nothing, available = nothing)
    kinds = DataType[serves...]
    isempty(kinds) && throw(ArgumentError("$(T) serves no kind: subtype one, or name them with serves"))
    for K in kinds
        K <: Kind || throw(ArgumentError("$(K) is not a kind"))
        kind(K).object && throw(ArgumentError("$(T): $(K) is an object kind; a provider serves provider kinds"))
    end
    configured = config !== nothing || config_schema !== nothing
    offers_default = default === nothing ? !configured : default
    return ProviderSpec(T, String(name), id === nothing ? nothing : String(id), String(display_name),
        String(version), meta, kinds, config, config_schema, offers_default, available)
end

"""
    @provider T key = value ...

Declares the struct `T` a provider in the module where it is written, in
declaration order (`library_of` collects them): `@provider Hello serves
= (Greeter, Counter) display_name = "Hello, in Julia"`. The keys are
`Provider`'s.
"""
macro provider(T, kw...)
    pairs = map(kw) do a
        a isa Expr && a.head === :(=) || error("@provider options are key = value")
        Expr(:kw, a.args[1], a.args[2])
    end
    list = :__guatiao_providers__
    return esc(quote
        isdefined(@__MODULE__, $(QuoteNode(list))) || global $list = $(Guatiao).ProviderSpec[]
        push!($list, $(Guatiao).Provider($T; $(pairs...)))
        nothing
    end)
end

# ---- the descriptor's slot pools ---------------------------------------------------------

const MAX_LIBRARIES = 64
const MAX_PROVIDERS = 256
const POOL_LOCK = Threads.SpinLock()
const LIBRARY_SLOTS = Vector{Any}(nothing, MAX_LIBRARIES)
const PROVIDER_SLOTS = Vector{Any}(nothing, MAX_PROVIDERS)
const REASONS = Dict{String,Vector{UInt8}}()   # a reason a provider gives, kept for every reader

function library_entry(i::Int, host::Ptr{Cvoid})::Ptr{Cvoid}
    try
        lib = LIBRARY_SLOTS[i]
        lib === nothing && return C_NULL
        lib.closed && return C_NULL
        return Ptr{Cvoid}(lib.info)
    catch
        return C_NULL
    end
end

function library_unload(i::Int)::UInt32
    try
        lib = LIBRARY_SLOTS[i]
        lib === nothing && return UInt32(0)
        return live_instances(lib) > 0 ? UInt32(Busy) : UInt32(0)
    catch
        return UInt32(Internal)
    end
end

function provider_available(j::Int, ctx::Ptr{Cvoid}, reason::Ptr{GStr})::Bool
    why = try
        p = PROVIDER_SLOTS[j]
        p === nothing ? nothing : p.spec.available()
    catch e
        sprint(showerror, e)
    end
    why === nothing && return true
    text = string(why)
    buf = lock(POOL_LOCK) do
        get!(() -> Vector{UInt8}(text), REASONS, text)
    end
    reason == C_NULL || unsafe_store!(reason, GStr(pointer(buf), length(buf)))
    return false
end

function provider_create(j::Int, ctx::Ptr{Cvoid}, config::Ptr{GValue}, out::Ptr{Ptr{Cvoid}}, err::Ptr{GProviderError})::UInt32
    try
        # Refused before anything is built: an instance with no slot to land in would be held for good.
        out == C_NULL && throw(GuatiaoError(NullArgument, "no slot for the instance"))
        unsafe_store!(out, C_NULL)
        config == C_NULL && throw(GuatiaoError(NullArgument, "no configuration"))
        p = PROVIDER_SLOTS[j]
        spec = p.spec
        made = spec.config !== nothing ? decode_at(spec.config, config, Key[]) : plain(config)
        instance = spec.type(made)
        ctx_ = hold(Served(instance, p.library))
        add_instance!(p.library, ctx_)
        unsafe_store!(out, ctx_)
        return UInt32(0)
    catch e
        s, message = status_of(e)
        write_err(err, s == 0 ? UInt32(Internal) : s, message)
        return s == 0 ? UInt32(Internal) : s
    end
end

function provider_destroy(ctx::Ptr{Cvoid}, instance::Ptr{Cvoid})::Cvoid
    try
        obj = release(instance)
        obj isa Served && obj.library !== nothing && remove_instance!(obj.library, instance)
        v = served_value(obj)
        v === nothing || !hasmethod(close, Tuple{typeof(v)}) || close(v)
    catch e
        try
            WARN[]("destroy failed: $(sprint(showerror, e))")
        catch
        end
    end
    return nothing
end

for i in 1:MAX_LIBRARIES
    @eval begin
        $(Symbol("__gq_entry_", i))(host::Ptr{Cvoid})::Ptr{Cvoid} = library_entry($i, host)
        $(Symbol("__gq_unload_", i))()::UInt32 = library_unload($i)
    end
end
for j in 1:MAX_PROVIDERS
    @eval begin
        $(Symbol("__gq_available_", j))(ctx::Ptr{Cvoid}, reason::Ptr{GStr})::Bool = provider_available($j, ctx, reason)
        $(Symbol("__gq_create_", j))(ctx::Ptr{Cvoid}, config::Ptr{GValue}, out::Ptr{Ptr{Cvoid}}, err::Ptr{GProviderError})::UInt32 =
            provider_create($j, ctx, config, out, err)
    end
end
# Each pointer is taken when its slot is first used: a thunk per slot.
const ENTRY_POINTERS = Tuple(@eval(() -> @cfunction($(Symbol("__gq_entry_", i)), Ptr{Cvoid}, (Ptr{Cvoid},))) for i in 1:MAX_LIBRARIES)
const UNLOAD_POINTERS = Tuple(@eval(() -> @cfunction($(Symbol("__gq_unload_", i)), UInt32, ())) for i in 1:MAX_LIBRARIES)
const AVAILABLE_POINTERS = Tuple(@eval(() -> @cfunction($(Symbol("__gq_available_", j)), Bool, (Ptr{Cvoid}, Ptr{GStr}))) for j in 1:MAX_PROVIDERS)
const CREATE_POINTERS = Tuple(@eval(() -> @cfunction($(Symbol("__gq_create_", j)), UInt32,
    (Ptr{Cvoid}, Ptr{GValue}, Ptr{Ptr{Cvoid}}, Ptr{GProviderError}))) for j in 1:MAX_PROVIDERS)
destroy_instance_pointer() = @cfunction(provider_destroy, Cvoid, (Ptr{Cvoid}, Ptr{Cvoid}))

function take_slot!(slots::Vector{Any}, value, what::String)
    lock(POOL_LOCK) do
        i = findfirst(isnothing, slots)
        i === nothing && throw(GuatiaoError(Busy, "a process builds at most $(length(slots)) $what in Julia"))
        slots[i] = value
        return i
    end
end

# ---- a library ---------------------------------------------------------------------------------

"""
    Library(id, version; providers, meta = nothing)

A library of providers (`Provider` specs or types declared with
`@provider`): the primitive under `library_of`, for a test or a library
made at run time. Its descriptor is built on first use and kept for the
process: a process builds at most 64 libraries and 256 providers.
`close(lib)` lets go of each provider's one instance and answers null to
any host asking for the descriptor from then on.
"""
mutable struct Library
    id::String
    version::String
    providers::Vector{ProviderSpec}
    meta::Any
    info::Ptr{GLibraryInfo}
    slot::Int
    keep::Vector{Any}
    instances::Set{UInt}
    defaults::Vector{Ptr{Cvoid}}
    lock::Threads.SpinLock
    closed::Bool
end

function Library(id::AbstractString, version::AbstractString; providers, meta = nothing)
    specs = ProviderSpec[p isa ProviderSpec ? p : Provider(p) for p in providers]
    isempty(specs) && throw(ArgumentError("a library offers at least one provider"))
    ids = [something(p.id, "$(last(split(id, '.')))_$(p.name)") for p in specs]
    allunique(ids) || throw(ArgumentError("two providers of $id share an id: $(join(ids, ", "))"))
    for p in specs, K in p.kinds
        if p.default || (p.config === nothing && p.config_schema === nothing)
            table_for(K, p.type)   # refuses a struct missing a required method now
        else
            spec = kind(K)
            fns = kind_functions(K)
            missing = [s.name for (i, s) in enumerate(spec.slots) if s.required && !implements(fns[i], p.type, s)]
            isempty(missing) || throw(ArgumentError("$(p.type) has no method of $(join(missing, ", ")), which the $(spec.name) kind requires"))
        end
    end
    return Library(String(id), String(version), specs, meta, C_NULL, 0, Any[], Set{UInt}(), Ptr{Cvoid}[],
        Threads.SpinLock(), false)
end

live_instances(lib::Library) = (lock(lib.lock); try length(lib.instances) finally unlock(lib.lock) end)
add_instance!(lib::Library, ctx::Ptr{Cvoid}) = (lock(lib.lock); try push!(lib.instances, UInt(ctx)) finally unlock(lib.lock) end)
remove_instance!(lib::Library, ctx::Ptr{Cvoid}) = (lock(lib.lock); try delete!(lib.instances, UInt(ctx)) finally unlock(lib.lock) end)
remove_instance!(::Any, ::Ptr{Cvoid}) = nothing
Base.show(io::IO, lib::Library) = print(io, "Guatiao.Library(", lib.id, " ", lib.version, ")")

"Text copied into C memory the descriptor keeps for the process."
function ctext!(keep::Vector{Any}, s::AbstractString)
    b = Vector{UInt8}(String(s))
    push!(keep, b)
    return GStr(isempty(b) ? Ptr{UInt8}(C_NULL) : pointer(b), length(b))
end

function cmalloc!(keep::Vector{Any}, n::Integer)
    b = zeros(UInt8, max(n, 1))
    push!(keep, b)
    return pointer(b)
end

function meta_map!(keep::Vector{Any}, meta)
    meta === nothing && return Ptr{GMap}(C_NULL)
    v = meta isa Value ? meta : Value(meta)
    load_tag(root_node(v)) == TAG_MAP || throw(ArgumentError("meta is a map"))
    push!(keep, v)
    return Ptr{GMap}(root_node(v) + PAYLOAD)
end

"Builds the descriptor once; answers the `guatiao_library_info *`."
function describe!(lib::Library)
    lib.info == C_NULL || return lib.info
    keep = lib.keep
    n = length(lib.providers)
    infos = Ptr{GProviderInfo}(cmalloc!(keep, n * sizeof(GProviderInfo)))
    lib_slot = take_slot!(LIBRARY_SLOTS, lib, "libraries")
    for (at, p) in enumerate(lib.providers)
        j = take_slot!(PROVIDER_SLOTS, (spec = p, library = lib), "providers")
        pid = something(p.id, "$(last(split(lib.id, '.')))_$(p.name)")
        names = Ptr{GStr}(cmalloc!(keep, length(p.kinds) * sizeof(GStr)))
        tables = Ptr{GKindTable}(cmalloc!(keep, length(p.kinds) * sizeof(GKindTable)))
        for (i, K) in enumerate(p.kinds)
            spec = kind(K)
            unsafe_store!(names, ctext!(keep, spec.name), i)
            unsafe_store!(tables, GKindTable(sizeof(GKindTable), spec.size, ctext!(keep, spec.name), table_for(K, p.type)), i)
        end
        configured = p.config !== nothing || p.config_schema !== nothing
        schema = if configured
            s = p.config_schema !== nothing ? Value(p.config_schema) : schema_of_value(p.config)
            push!(keep, s)
            root_node(s)
        else
            Ptr{GValue}(C_NULL)
        end
        ctx = C_NULL
        if p.default
            ctx = hold(Served(p.type(), nothing))
            push!(lib.defaults, ctx)
        end
        info = GProviderInfo(sizeof(GProviderInfo), 0, GKinds(names, length(p.kinds)), ctext!(keep, pid),
            ctext!(keep, p.display_name), schema, C_NULL, ctx, meta_map!(keep, p.meta), ctext!(keep, p.version),
            p.available === nothing ? C_NULL : AVAILABLE_POINTERS[j](),
            GKindTables(tables, length(p.kinds), sizeof(GKindTable)),
            configured ? CREATE_POINTERS[j]() : C_NULL, configured ? destroy_instance_pointer() : C_NULL)
        unsafe_store!(infos, info, at)
    end
    info = Ptr{GLibraryInfo}(cmalloc!(keep, sizeof(GLibraryInfo)))
    unsafe_store!(info, GLibraryInfo(sizeof(GLibraryInfo), GUATIAO_ABI_VERSION, ctext!(keep, lib.id), ctext!(keep, lib.version),
        GProviders(infos, n, sizeof(GProviderInfo)), meta_map!(keep, lib.meta), UNLOAD_POINTERS[lib_slot]()))
    lib.slot = lib_slot
    lib.info = info
    return info
end

schema_of_value(T) = schema(T)

"""
    entry(lib) -> Ptr{Cvoid}

The library's `guatiao_library_entry`, building its descriptor first.
"""
function entry(lib::Library)
    describe!(lib)
    return ENTRY_POINTERS[lib.slot]()
end

function Base.close(lib::Library)
    lib.closed && return nothing
    lib.closed = true
    for ctx in lib.defaults
        obj = release(ctx)
        v = served_value(obj)
        v === nothing || !hasmethod(close, Tuple{typeof(v)}) || close(v)
    end
    empty!(lib.defaults)
    return nothing
end

"""
    register_library(reg, lib_or_module)

Registers a library written in Julia (a `Library`, or a package module
`library_of` reads) under its id, as `register_entry` does a C one, and
keeps it as long as the registry. It is linked, not mapped: `unload!`
answers `WrongKind`, `retire!` takes it out.
"""
function register_library(reg::Registry, lib)
    l = lib isa Module ? library_of(lib) : lib
    push!(reg.libraries, l)
    return register_entry(reg, l.id, entry(l))
end

# ---- a package as a library ----------------------------------------------------------------------

const BY_MODULE = IdDict{Module,Library}()
const BY_MODULE_LOCK = ReentrantLock()

"""
    library_of(mod::Module; version = nothing, meta = nothing) -> Library

The library a package is: its id the module's full name (`Pkg.Sub` for a
submodule), its version the package's (`pkgversion`, from its
`Project.toml`), its providers the `@provider` declarations in the module,
in order. A module loaded from no package is refused unless `version`
says one. Meta, lowest first: the package's `Project.toml` (`authors`,
`uuid`, and `description` as `summary`, `license`, `homepage` and
`repository` as `urls` when it has them), its licence files as notices,
the first paragraph of the module's docstring as `description`, then
`meta`. Built once per module.
"""
function library_of(mod::Module; version = nothing, meta = nothing)
    lock(BY_MODULE_LOCK) do
        found = get(BY_MODULE, mod, nothing)
        found === nothing || return found
        id = join(string.(fullname(mod)), ".")
        v = version !== nothing ? string(version) : begin
            pv = pkgversion(mod)
            pv === nothing && throw(ArgumentError("$id comes from no package, so it has no version: say one with version = \"...\""))
            string(pv)
        end
        isdefined(mod, :__guatiao_providers__) || throw(ArgumentError("$id declares no provider (@provider)"))
        specs = getfield(mod, :__guatiao_providers__)
        isempty(specs) && throw(ArgumentError("$id declares no provider (@provider)"))
        lib = Library(id, v; providers = specs, meta = merge(module_meta(mod), meta === nothing ? Dict{String,Any}() : Dict{String,Any}(meta)))
        BY_MODULE[mod] = lib
        return lib
    end
end

const LICENSE_FILES = ("LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "LICENCE.md", "COPYING", "COPYING.md")

function module_meta(mod::Module)
    meta = Dict{String,Any}()
    dir = pkgdir(mod)
    if dir !== nothing
        project = joinpath(dir, "Project.toml")
        if isfile(project)
            t = TOML.parsefile(project)
            haskey(t, "authors") && (meta["authors"] = [string(a) for a in t["authors"]])
            haskey(t, "uuid") && (meta["uuid"] = string(t["uuid"]))
            haskey(t, "description") && (meta["summary"] = string(t["description"]))
            haskey(t, "license") && (meta["license"] = string(t["license"]))
            urls = Dict{String,Any}()
            for k in ("homepage", "repository")
                haskey(t, k) && (urls[k] = string(t[k]))
            end
            isempty(urls) || (meta["urls"] = urls)
            component = get(t, "name", string(nameof(mod)))
            notices = [Dict("component" => component, "text" => read(joinpath(dir, f), String), "format" => "text")
                       for f in LICENSE_FILES if isfile(joinpath(dir, f))]
            isempty(notices) || (meta["notices"] = notices)
        end
    end
    doc = module_docstring(mod)
    doc === nothing || (meta["description"] = doc)
    return meta
end

"The first paragraph of a module's own docstring, or `nothing`."
function module_docstring(mod::Module)
    try
        md = Base.Docs.meta(mod)
        binding = Base.Docs.Binding(mod, nameof(mod))
        haskey(md, binding) || return nothing
        multi = md[binding]
        isempty(multi.docs) && return nothing
        text = join(first(values(multi.docs)).text, "")
        paragraphs = split(strip(text), r"\n\s*\n")
        # A signature block in an indented first paragraph is skipped.
        for p in paragraphs
            s = strip(p)
            isempty(s) && continue
            startswith(p, "    ") && continue
            return replace(s, r"\s*\n\s*" => " ")
        end
        return nothing
    catch
        return nothing
    end
end
