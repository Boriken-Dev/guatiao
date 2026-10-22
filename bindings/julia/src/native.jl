# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Finding guatiao, guatiao_serde and guatiao_intake, and calling their
# exports through pointers resolved once. Nothing native is touched until
# the first call that needs it.

using Libdl

mutable struct NativeLib
    basename::String
    path::String
    handle::Ptr{Cvoid}
end

const LOAD_LOCK = ReentrantLock()
const CHOSEN = Ref{Union{Nothing,String}}(nothing)
const LOADED = Dict{String,NativeLib}()
const CACHES = Base.RefValue{Ptr{Cvoid}}[]
const BINDING_ALLOC = Ref{Ptr{GAlloc}}(C_NULL)

"The platform's file name for a library of this base name."
library_filename(base::AbstractString) =
    Sys.iswindows() ? "$base.dll" : Sys.isapple() ? "lib$base.dylib" : "lib$base.so"

"""
    use_library(path)

Loads guatiao from `path` (a file exporting the ABI, whatever it is
called, or a directory holding `guatiao.dll` / `libguatiao.so` /
`libguatiao.dylib`) instead of looking for one. Call it before the first
call that needs the library; `guatiao_serde` and `guatiao_intake` are
then looked for beside it. Naming another file once one is loaded is an
error; naming the same one is not.
"""
function use_library(path::AbstractString)
    lock(LOAD_LOCK) do
        core = get(LOADED, "guatiao", nothing)
        if core !== nothing
            same = isdir(path) ? joinpath(path, library_filename("guatiao")) : String(path)
            realpath(same) == realpath(core.path) ||
                error("the guatiao library is already loaded from $(core.path)")
            return nothing
        end
        CHOSEN[] = String(path)
        return nothing
    end
end

function resolve_path(basename::String)
    filename = library_filename(basename)
    looked = String[]
    named = CHOSEN[] !== nothing ? CHOSEN[] : get(ENV, "GUATIAO_LIBRARY", "")
    if !isempty(named)
        if isdir(named)
            candidate = joinpath(named, filename)
            push!(looked, candidate)
            isfile(candidate) && return candidate
        elseif isfile(named)
            basename == "guatiao" && return named
            candidate = joinpath(dirname(named), filename)
            push!(looked, candidate)
            isfile(candidate) && return candidate
        else
            push!(looked, named)
        end
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

function resolve_symbol!(cache::Base.RefValue{Ptr{Cvoid}}, basename::String, name::String, feature::String)
    lib = native_lib(basename)
    p = Libdl.dlsym_e(lib.handle, name)
    p == C_NULL && throw(MissingSymbol(lib.path, name, feature))
    cache[] = p
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

# `@cdef base feature mode name(arg::CType, ...)::CRet` defines `name`,
# calling the export of that name in the `base` library. `mode` `:safe`
# makes the call in a GC-safe region: every Julia argument is converted
# and rooted first, since nothing may allocate inside one.
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
    sname = string(name)
    lookup = quote
        f = $cache[]
        if f == C_NULL
            f = resolve_symbol!($cache, $base, $sname, $feature)
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
                state = gc_safe_enter()
                r = ccall(f, $ret, ($(argtypes...),), $(us...))
                gc_safe_leave(state)
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
        push!(CACHES, $cache)
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

# ---- guatiao_serde -------------------------------------------------------------
@cdef "guatiao_serde" "" :plain guatiao_json_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit_pretty(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32

# ---- guatiao_intake --------------------------------------------------------------
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
    for c in CACHES
        c[] = C_NULL
    end
    empty!(LOADED)
    CHOSEN[] = nothing
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
