# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Finding a guatiao library (guatiao itself, or another surface such as
# guatiao_serde), and calling its exports through pointers resolved once.
# Nothing native is touched until the first call that needs it.

using Libdl

mutable struct NativeLib
    basename::String
    path::String
    handle::Ptr{Cvoid}
end

const LOAD_LOCK = ReentrantLock()
const CHOSEN = Dict{String,String}()
const LOADED = Dict{String,NativeLib}()
# Bumped by every reset: a resolved pointer is used only while the
# generation it was resolved in is current, so pointers a package image
# kept, in this package or another, are resolved again.
const GENERATION = Ref(0)
const BINDING_ALLOC = Ref{Ptr{GAlloc}}(C_NULL)

# Each surface's own environment variable (consulted before
# GUATIAO_LIBRARY) and a symbol only it exports.
const SURFACES = Dict{String,NamedTuple{(:variable, :probe),Tuple{String,String}}}(
    "guatiao" => (variable = "", probe = ""),
    "guatiao_serde" => (variable = "GUATIAO_SERDE_LIBRARY", probe = "guatiao_json_parse_ex"),
    "guatiao_intake" => (variable = "GUATIAO_INTAKE_LIBRARY", probe = "guatiao_intake_check"),
)

"The platform's file name for a library of this base name."
library_filename(base::AbstractString) =
    Sys.iswindows() ? "$base.dll" : Sys.isapple() ? "lib$base.dylib" : "lib$base.so"

"""
    use_library(path)

Loads guatiao from `path` (a file exporting the ABI, whatever it is
called, or a directory holding `guatiao.dll` / `libguatiao.so` /
`libguatiao.dylib`) instead of looking for one. Call it before the first
call that needs the library. Only this package's library: GuatiaoSerde
and GuatiaoIntake have their own `use_library`. Naming another file once
one is loaded is an error; naming the same one is not.
"""
use_library(path::AbstractString) = use_library("guatiao", path)

function use_library(basename::AbstractString, path::AbstractString)
    lock(LOAD_LOCK) do
        loaded = get(LOADED, basename, nothing)
        if loaded !== nothing
            same = isdir(path) ? joinpath(path, library_filename(basename)) : String(path)
            realpath(same) == realpath(loaded.path) ||
                error("the $basename library is already loaded from $(loaded.path)")
            return nothing
        end
        CHOSEN[String(basename)] = String(path)
        return nothing
    end
end

function carries(path::String, symbol::String)
    h = Libdl.dlopen(path; throw_error = false)
    h === nothing && return false
    found = Libdl.dlsym_e(h, symbol) != C_NULL
    Libdl.dlclose(h)
    return found
end

# What `use_library` named, the surface's own variable, GUATIAO_LIBRARY,
# then the platform's search. A directory holds the surface's file under
# its platform name; a file named for the surface is taken as it is; a
# GUATIAO_LIBRARY file serves every surface it exports, else the
# surface's own file beside it does.
function resolve_path(basename::String)
    filename = library_filename(basename)
    looked = String[]
    surface = get(SURFACES, basename, (variable = "", probe = ""))
    function named(where::String, route::String, shared::Bool)
        push!(looked, "$route ($where)")
        if isdir(where)
            candidate = joinpath(where, filename)
            return isfile(candidate) ? candidate : nothing
        elseif isfile(where)
            (!shared || isempty(surface.probe) || carries(where, surface.probe)) && return where
            candidate = joinpath(dirname(where), filename)
            return isfile(candidate) ? candidate : nothing
        end
        return nothing
    end
    chosen = get(CHOSEN, basename, nothing)
    if chosen !== nothing
        found = named(chosen, "use_library", false)
        found === nothing || return found
    end
    if !isempty(surface.variable)
        own = get(ENV, surface.variable, "")
        if isempty(own)
            push!(looked, "$(surface.variable) (unset)")
        else
            found = named(own, surface.variable, false)
            found === nothing || return found
        end
    end
    shared = get(ENV, "GUATIAO_LIBRARY", "")
    if isempty(shared)
        push!(looked, "GUATIAO_LIBRARY (unset)")
    else
        found = named(shared, "GUATIAO_LIBRARY", true)
        found === nothing || return found
    end
    found = Libdl.find_library(basename)
    push!(looked, "the platform's search for $filename")
    isempty(found) || return found
    throw(LibraryNotFound(basename, looked))
end

function native_lib(basename::String)
    return lock(LOAD_LOCK) do
        get!(LOADED, basename) do
            path = resolve_path(basename)
            handle = Libdl.dlopen(path)
            NativeLib(basename, path, handle)
        end
    end
end

function resolve_symbol!(cache::Base.RefValue{Ptr{Cvoid}}, generation::Base.RefValue{Int},
    basename::String, name::String, feature::String)
    lib = native_lib(basename)
    p = Libdl.dlsym_e(lib.handle, name)
    p == C_NULL && throw(MissingSymbol(lib.path, name, feature))
    cache[] = p
    generation[] = GENERATION[]
    return p
end

"""
    exports(basename, name) -> Bool

Whether the `basename` library (`"guatiao"`, `"guatiao_serde"`,
`"guatiao_intake"`), loaded on first use, exports `name`.
"""
function exports(basename::String, name::String)
    return Libdl.dlsym_e(native_lib(basename).handle, name) != C_NULL
end

@inline gc_safe_enter() = ccall(:jl_gc_safe_enter, Int8, ())
@inline gc_safe_leave(state::Int8) = ccall(:jl_gc_safe_leave, Cvoid, (Int8,), state)

"""
    @cdef base feature mode name(arg::CType, ...)::CRet

Defines `name`, calling the export of that name in the `base` library
(`"guatiao"`, `"guatiao_serde"`, ...), found and opened on first use;
`feature` names the build feature it needs (`""` for every build), for
the `MissingSymbol` a library without it raises. `mode` `:safe` makes
the call in a GC-safe region: every Julia argument is converted and
rooted first, since nothing may allocate inside one; `:plain` is an
ordinary `ccall`. Usable from any module.
"""
macro cdef(base, feature, mode, sig)
    call, ret = sig.args
    name = call.args[1]
    argnames = Symbol[]
    argtypes = Any[]
    for a in call.args[2:end]
        push!(argnames, a.args[1])
        push!(argtypes, a.args[2])
    end
    cache = Symbol("_cptr_", name)
    gen = Symbol("_cgen_", name)
    sname = string(name)
    here = @__MODULE__
    current = GlobalRef(here, :GENERATION)
    resolve = GlobalRef(here, :resolve_symbol!)
    enter = GlobalRef(here, :gc_safe_enter)
    leave = GlobalRef(here, :gc_safe_leave)
    lookup = quote
        f = $cache[]
        if f == C_NULL || $gen[] != $current[]
            f = $resolve($cache, $gen, $base, $sname, $feature)
        end
    end
    body = if mode == QuoteNode(:safe)
        cs = [Symbol("c", i) for i in eachindex(argnames)]
        us = [Symbol("u", i) for i in eachindex(argnames)]
        converts = [:($(cs[i]) = Base.cconvert($(argtypes[i]), $(argnames[i]))) for i in eachindex(cs)]
        unsafes = [:($(us[i]) = Base.unsafe_convert($(argtypes[i]), $(cs[i]))) for i in eachindex(cs)]
        quote
            $lookup
            $(converts...)
            GC.@preserve $(cs...) begin
                $(unsafes...)
                state = $enter()
                r = ccall(f, $ret, ($(argtypes...),), $(us...))
                $leave(state)
            end
            r
        end
    else
        quote
            $lookup
            ccall(f, $ret, ($(argtypes...),), $(argnames...))
        end
    end
    return esc(quote
        const $cache = Ref{Ptr{Cvoid}}(C_NULL)
        const $gen = Ref(-1)
        @inline function $name($(argnames...))
            $body
        end
    end)
end

const PV = Ptr{GValue}
const PA = Ptr{GAlloc}
const P = Ptr{Cvoid}

# ---- values -----------------------------------------------------------------
@cdef "guatiao" "" :plain guatiao_alloc_default(out::PA)::UInt32
@cdef "guatiao" "" :plain guatiao_value_free(v::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_clone(a::PA, src::PV, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_make_local(v::PV, a::PA)::UInt32
@cdef "guatiao" "" :plain guatiao_map_settle_lent(v::PV, a::PA)::UInt32
@cdef "guatiao" "" :plain guatiao_value_null(out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_absent(out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_bool(b::Bool, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_map(a::PA, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_list(a::PA, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_string(a::PA, text::GStr, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_number(a::PA, text::GStr, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_value_bytes(a::PA, b::GBytes, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_map_set(a::PA, node::PV, key::GStr, value::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_map_discard(node::PV, key::GStr)::UInt32
@cdef "guatiao" "" :plain guatiao_map_clear(node::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_map_copy_from(a::PA, dst::PV, src::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_list_push(a::PA, node::PV, value::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_list_discard(node::PV, index::Csize_t)::UInt32
@cdef "guatiao" "" :plain guatiao_list_clear(node::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_schema_validate(schema::PV, config::PV, a::PA, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_schema_redact(schema::PV, value::PV, placeholder::GStr, a::PA, out::PV)::UInt32

# ---- the wire encoding and channels --------------------------------------------
@cdef "guatiao" "" :plain guatiao_wire_encode(a::PA, value::PV, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_wire_decode(a::PA, b::GBytes, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_wire_announce(a::PA, channel::UInt64, schema::PV, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_wire_send(a::PA, channel::UInt64, value::PV, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_wire_close(a::PA, channel::UInt64, out::PV)::UInt32
@cdef "guatiao" "" :plain guatiao_wire_receiver_new(a::PA)::P
@cdef "guatiao" "" :plain guatiao_wire_receiver_accept(recv::P, frame::GBytes, event::Ptr{GWireEvent})::UInt32
@cdef "guatiao" "" :plain guatiao_wire_receiver_free(recv::P)::Cvoid

# ---- dates, times, spans, exact lengths --------------------------------------
for (form, T) in ((:date, GDate), (:time, GTime), (:datetime, GDateTime), (:timestamp, GTimestamp), (:span, GSpan))
    parse_name = Symbol("guatiao_", form, "_parse")
    format_name = Symbol("guatiao_", form, "_format")
    normalize_name = Symbol("guatiao_", form, "_normalize")
    @eval begin
        @cdef "guatiao" "" :plain $(parse_name)(a::PA, text::GStr, out::Ptr{$T}, err::Ptr{GTextError})::UInt32
        @cdef "guatiao" "" :plain $(format_name)(a::PA, fields::Ptr{$T}, out::PV)::UInt32
        @cdef "guatiao" "" :plain $(normalize_name)(a::PA, text::GStr, out::PV, err::Ptr{GTextError})::UInt32
    end
end
@cdef "guatiao" "" :plain guatiao_date_add_span(d::Ptr{GDate}, s::Ptr{GSpan}, out::Ptr{GDate})::UInt32
@cdef "guatiao" "" :plain guatiao_datetime_add_span(d::Ptr{GDateTime}, s::Ptr{GSpan}, out::Ptr{GDateTime})::UInt32
@cdef "guatiao" "" :plain guatiao_duration_read(v::PV, secs::Ptr{UInt64}, nanos::Ptr{UInt32})::UInt32

# ---- the registry: every call that can reach a library is GC-safe ------------
@cdef "guatiao" "load" :plain guatiao_registry_new(id::GStr, version::GStr, a::PA)::P
@cdef "guatiao" "load" :safe guatiao_registry_free(reg::P)::Cvoid
@cdef "guatiao" "load" :safe guatiao_registry_provider_create(reg::P, key::GStr, config::PV, out::Ptr{P}, err::Ptr{GProviderError})::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_provider_destroy(reg::P, key::GStr, instance::P)::Cvoid
@cdef "guatiao" "load" :plain guatiao_registry_host(reg::P)::Ptr{GHostInfo}
@cdef "guatiao" "load" :safe guatiao_registry_retire(reg::P, key::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_unload(reg::P, key::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_unload_unchecked(reg::P, key::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_collect(reg::P, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_add_source_file(reg::P, path::GStr, config::PV, err::Ptr{GProviderError})::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_watch_dir(reg::P, dir::GStr, rules::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_refresh(reg::P, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_keyed_by(reg::P, template::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_libraries_keyed_by(reg::P, template::GStr)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_load_file(reg::P, path::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_register_entry(reg::P, name::GStr, entry::P, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_scan_dir(reg::P, dir::GStr, descending::Bool, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_scan_dir_rules(reg::P, dir::GStr, descending::Bool, rules::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_scan_path(reg::P, spec::GStr, descending::Bool, rules::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_libraries(reg::P, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_providers(reg::P, kind::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_provider(reg::P, key::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_available(reg::P, kind::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_why_not(reg::P, kind::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_provider_available(reg::P, key::GStr, reason::Ptr{GStr})::Bool
@cdef "guatiao" "load" :safe guatiao_registry_set_priority(reg::P, id::GStr, priority::Int32)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_priority(reg::P, id::GStr)::Int32
@cdef "guatiao" "load" :safe guatiao_registry_best(reg::P, kind::GStr, a::PA, out::PV)::UInt32
@cdef "guatiao" "load" :safe guatiao_registry_provider_vtable(reg::P, key::GStr, size::Ptr{Csize_t})::P
@cdef "guatiao" "load" :safe guatiao_registry_provider_table(reg::P, key::GStr, kind::GStr, size::Ptr{Csize_t})::P
@cdef "guatiao" "load" :safe guatiao_registry_provider_ctx(reg::P, key::GStr)::P
@cdef "guatiao" "load" :safe guatiao_registry_provider_config(reg::P, key::GStr)::PV
@cdef "guatiao" "load" :safe guatiao_registry_provider_meta(reg::P, key::GStr, a::PA, out::PV)::UInt32

"""
The allocator this binding builds in when nobody names one: the
`guatiao` library's default, kept for the process.
"""
function binding_alloc()
    a = BINDING_ALLOC[]
    a == C_NULL || return a
    return lock(LOAD_LOCK) do
        a = BINDING_ALLOC[]
        a == C_NULL || return a
        a = Ptr{GAlloc}(Libc.calloc(1, sizeof(GAlloc)))
        status = guatiao_alloc_default(a)
        status == 0 || (Libc.free(a); throw(GuatiaoError(status, "guatiao_alloc_default failed")))
        BINDING_ALLOC[] = a
        return a
    end
end

function reset_native!()
    GENERATION[] += 1
    empty!(LOADED)
    empty!(CHOSEN)
    BINDING_ALLOC[] = C_NULL
    return nothing
end

# Text crossing as a guatiao_str: the caller keeps `s` alive (GC.@preserve)
# for as long as the view is used.
@inline gstr(s::Union{String,SubString{String}}) = GStr(pointer(s), sizeof(s))
@inline gbytes(b::Vector{UInt8}) = GBytes(isempty(b) ? Ptr{UInt8}(C_NULL) : pointer(b), length(b))

"The text a guatiao_string holds."
@inline text_of(s::GString) = s.len == 0 ? "" : unsafe_string(s.ptr, s.len)
@inline text_of(s::GStr) = s.len == 0 ? "" : unsafe_string(s.ptr, s.len)
