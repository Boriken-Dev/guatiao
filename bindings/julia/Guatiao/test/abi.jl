# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The struct mirrors are the ABI, and a wrong offset is silent corruption:
# each mirrored struct's layout is computed from the committed header's
# text and compared with the Julia struct's, field by field. The tag and
# status constants are read out of the header the same way.

using Guatiao: GAlloc, GStr, GBytes, GBytesMut, GString, GBuffer, GValue, GList, GEntry, GMap,
    GProviderError, GDate, GTime, GDateTime, GTimestamp, GSpan, GTextError, GKindHeader,
    GObject, GKinds, GKindTable, GKindTables, GProviderInfo, GProviders, GLibraryInfo, GHostInfo,
    GWireEvent

const MIRRORS = Dict(
    "guatiao_alloc" => GAlloc, "guatiao_str" => GStr, "guatiao_bytes" => GBytes,
    "guatiao_bytes_mut" => GBytesMut, "guatiao_string" => GString, "guatiao_buffer" => GBuffer,
    "guatiao_value" => GValue, "guatiao_list" => GList, "guatiao_entry" => GEntry, "guatiao_map" => GMap,
    "guatiao_provider_error" => GProviderError,
    "guatiao_date" => GDate, "guatiao_time" => GTime, "guatiao_datetime" => GDateTime,
    "guatiao_timestamp" => GTimestamp, "guatiao_span" => GSpan, "guatiao_text_error" => GTextError,
    "guatiao_kind_header" => GKindHeader, "guatiao_object" => GObject, "guatiao_kinds" => GKinds,
    "guatiao_kind_table" => GKindTable, "guatiao_kind_tables" => GKindTables,
    "guatiao_provider_info" => GProviderInfo, "guatiao_providers" => GProviders,
    "guatiao_library_info" => GLibraryInfo, "guatiao_host_info" => GHostInfo,
    "guatiao_wire_event" => GWireEvent,
)

# A pointer and a size_t are a word wide; a 64-bit integer is 8-aligned on
# every platform the library builds for.
const W = sizeof(Ptr{Cvoid})
const SCALARS = Dict("uint8_t" => 1, "int8_t" => 1, "bool" => 1, "uint16_t" => 2, "int16_t" => 2,
    "uint32_t" => 4, "int32_t" => 4, "uint64_t" => 8, "int64_t" => 8, "size_t" => W, "intptr_t" => W,
    "uintptr_t" => W, "float" => 4, "double" => 8, "guatiao_status" => 4, "guatiao_tag" => 4)

uncomment(text) = replace(text, r"/\*.*?\*/"s => "")

"Every struct and union the headers define: name => (is_union, [(type, field)])."
function header_structs(texts...)
    defs = Dict{String,Tuple{Bool,Vector{Tuple{String,String}}}}()
    for text in texts
        t = uncomment(text)
        for m in eachmatch(r"(?:typedef\s+)?(struct|union)\s+(\w+)\s*\{(.*?)\}\s*\w*\s*;"s, t)
            fields = Tuple{String,String}[]
            for decl in split(m.captures[3], ';')
                d = strip(replace(decl, r"\s+" => " "))
                isempty(d) && continue
                fp = match(r"\(\*\s*(\w+)\)", d)
                if fp !== nothing
                    push!(fields, ("*", fp.captures[1]))
                else
                    parts = match(r"^(.*?)(\w+)$", d)
                    push!(fields, (strip(parts.captures[1]), parts.captures[2]))
                end
            end
            defs[m.captures[2]] = (m.captures[1] == "union", fields)
        end
    end
    return defs
end

"(size, alignment) of a C type spelled in a header."
function c_layout(ctype::AbstractString, defs)
    t = strip(replace(ctype, r"\b(const|struct|union)\b" => ""))
    (occursin('*', t) || t == "guatiao_map_ptr") && return (W, W)
    t == "guatiao_number" && (t = "guatiao_string")
    haskey(SCALARS, t) && return (SCALARS[t], SCALARS[t])
    haskey(defs, t) || error("unknown C type $ctype")
    return struct_layout(t, defs)[1:2]
end

"(size, alignment, field => offset) of a struct as a C compiler lays it out."
function struct_layout(name, defs)
    is_union, fields = defs[name]
    offset = 0
    align = 1
    offsets = Dict{String,Int}()
    size = 0
    for (ctype, field) in fields
        s, a = c_layout(ctype, defs)
        align = max(align, a)
        if is_union
            offsets[field] = 0
            size = max(size, s)
        else
            offset = cld(offset, a) * a
            offsets[field] = offset
            offset += s
        end
    end
    size = cld(is_union ? size : offset, align) * align
    return (size, align, offsets)
end

const HEADER_DEFS = header_structs(read(HEADER, String))

@testset "struct mirrors match the header" begin
    for (cname, T) in sort(collect(MIRRORS); by = first)
        @test haskey(HEADER_DEFS, cname)
        size, _, offsets = struct_layout(cname, HEADER_DEFS)
        @test sizeof(T) == size
        jl = Dict(string(n) => Int(fieldoffset(T, i)) for (i, n) in enumerate(fieldnames(T)))
        for (field, off) in offsets
            name = field == "_pad" ? "pad" : field
            @test get(jl, name, -1) == off
        end
        # Every Julia field but the padding the header leaves unnamed is the header's.
        for n in fieldnames(T)
            @test string(n) == "pad" || haskey(offsets, string(n))
        end
    end
end

@testset "tags and statuses match the header" begin
    text = read(HEADER, String)
    constants = Dict(m.captures[1] => parse(Int, m.captures[2]) for m in eachmatch(r"^\s*GUATIAO_([A-Z_0-9]+) = (\d+),"m, text))
    tags = ("ABSENT", "NULL", "BOOL", "NUMBER", "STRING", "BYTES", "LIST", "MAP")
    for (i, name) in enumerate(tags)
        @test constants[name] == i - 1
    end
    statuses = Dict("OK" => Guatiao.OK, "ERR_BAD_VALUE" => Guatiao.BadValue, "ERR_ALLOC" => Guatiao.AllocFailed,
        "ERR_WRONG_KIND" => Guatiao.WrongKind, "ERR_NOT_FOUND" => Guatiao.NotFound, "ERR_NULL" => Guatiao.NullArgument,
        "ERR_GONE" => Guatiao.Gone, "ERR_UNSUPPORTED" => Guatiao.Unsupported, "ERR_BUSY" => Guatiao.Busy,
        "ERR_INTERNAL" => Guatiao.Internal)
    for (name, s) in statuses
        @test constants[name] == Int(s)
    end
    # The header names no status this binding is missing.
    for name in keys(constants)
        (startswith(name, "ERR_") || name == "OK") && @test haskey(statuses, name)
    end
    @test occursin("#define GUATIAO_ABI_VERSION $(Int(Guatiao.GUATIAO_ABI_VERSION))", text)
end
