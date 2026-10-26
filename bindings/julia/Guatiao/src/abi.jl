# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The structs and constants of guatiao.h, field for field, in the
# header's order. An immutable isbits struct has C's natural layout, which
# is the header's; the sizes are pinned below in words (a pointer and a
# size_t), as the header states them, and every offset is compared with
# the header's text by the test suite.

const GUATIAO_ABI_VERSION = UInt32(1)

"The width of a pointer and of `size_t`, in bytes."
const WORD = sizeof(Ptr{Cvoid})

const TAG_ABSENT = UInt32(0)
const TAG_NULL = UInt32(1)
const TAG_BOOL = UInt32(2)
const TAG_NUMBER = UInt32(3)
const TAG_STRING = UInt32(4)
const TAG_BYTES = UInt32(5)
const TAG_LIST = UInt32(6)
const TAG_MAP = UInt32(7)

"`guatiao_alloc`: an allocator table; every owned container records one."
struct GAlloc
    struct_size::UInt32
    ctx::Ptr{Cvoid}
    alloc::Ptr{Cvoid}
    free::Ptr{Cvoid}
    release::Ptr{Cvoid}
end

"`guatiao_str`: borrowed UTF-8 text."
struct GStr
    ptr::Ptr{UInt8}
    len::Csize_t
end

"`guatiao_bytes`: borrowed bytes."
struct GBytes
    ptr::Ptr{UInt8}
    len::Csize_t
end

"`guatiao_bytes_mut`: borrowed writable bytes."
struct GBytesMut
    ptr::Ptr{UInt8}
    len::Csize_t
end

"`guatiao_string` (and `guatiao_number`): owned text; `cap == 0` owns nothing."
struct GString
    ptr::Ptr{UInt8}
    len::Csize_t
    cap::Csize_t
    alloc::Ptr{GAlloc}
end

"`guatiao_buffer`: owned bytes."
struct GBuffer
    ptr::Ptr{UInt8}
    len::Csize_t
    cap::Csize_t
    alloc::Ptr{GAlloc}
end

"`guatiao_value`: a tag, then a four-word payload whose live arm the tag names."
struct GValue
    tag::UInt32
    pad::UInt32
    payload::NTuple{4,UInt}
end

"`guatiao_list`: an owned sequence of values."
struct GList
    ptr::Ptr{GValue}
    len::Csize_t
    cap::Csize_t
    alloc::Ptr{GAlloc}
end

"`guatiao_entry`: a key and its value."
struct GEntry
    key::GString
    value::GValue
end

"`guatiao_map`: an owned sequence of entries, in insertion order."
struct GMap
    ptr::Ptr{GEntry}
    len::Csize_t
    cap::Csize_t
    alloc::Ptr{GAlloc}
end

"`guatiao_provider_error`: why a call across a kind failed."
struct GProviderError
    status::UInt32
    message::GString
end

struct GDate
    year::Int32
    month::UInt8
    day::UInt8
end

struct GTime
    hour::UInt8
    minute::UInt8
    second::UInt8
    nanosecond::UInt32
end

struct GDateTime
    date::GDate
    time::GTime
end

struct GTimestamp
    seconds::Int64
    nanoseconds::UInt32
end

struct GSpan
    years::Int64
    months::Int64
    weeks::Int64
    days::Int64
    hours::Int64
    minutes::Int64
    seconds::Int64
    nanoseconds::Int32
end

"`guatiao_text_error`: the byte offset a text was refused at, and what was expected there."
struct GTextError
    at::Csize_t
    expected::GValue
end

"`guatiao_kind_header`: the first eight bytes of every kind table."
struct GKindHeader
    struct_size::UInt32
    floor_hash::UInt32
end

"`guatiao_object`: an object crossing the boundary: its table, the table's size, its ctx."
struct GObject
    table::Ptr{Cvoid}
    size::Csize_t
    ctx::Ptr{Cvoid}
end

struct GKinds
    ptr::Ptr{GStr}
    len::Csize_t
end

struct GKindTable
    struct_size::UInt32
    vtable_size::UInt32
    kind::GStr
    vtable::Ptr{Cvoid}
end

struct GKindTables
    ptr::Ptr{GKindTable}
    len::Csize_t
    stride::Csize_t
end

"`guatiao_provider_info`: one thing a library offers."
struct GProviderInfo
    struct_size::UInt32
    vtable_size::UInt32
    kinds::GKinds
    id::GStr
    display_name::GStr
    config::Ptr{GValue}
    vtable::Ptr{Cvoid}
    ctx::Ptr{Cvoid}
    meta::Ptr{GMap}
    version::GStr
    available::Ptr{Cvoid}
    tables::GKindTables
    create::Ptr{Cvoid}
    destroy::Ptr{Cvoid}
    kind_descriptions::GStr
end

struct GProviders
    ptr::Ptr{GProviderInfo}
    len::Csize_t
    stride::Csize_t
end

"`guatiao_library_info`: what a library says about itself."
struct GLibraryInfo
    struct_size::UInt32
    abi_version::UInt32
    id::GStr
    version::GStr
    providers::GProviders
    meta::Ptr{GMap}
    unload::Ptr{Cvoid}
end

"`guatiao_host_info`: what a host says about itself; read, never written here."
struct GHostInfo
    struct_size::UInt32
    abi_version::UInt32
    host_id::GStr
    host_version::GStr
    alloc::Ptr{GAlloc}
    meta::Ptr{GMap}
    services::Ptr{Cvoid}
end

"`guatiao_wire_event`: what one channel frame said, or why it was refused; its three values are the caller's."
struct GWireEvent
    channel::UInt64
    kind::UInt32
    refusal::UInt32
    at::Csize_t
    value::GValue
    message::GValue
    detail::GValue
end

const PAYLOAD = 8   # offset of the payload in a guatiao_value
const ENTRY_VALUE = 4 * WORD  # offset of the value in a guatiao_entry
const OWNED_ALLOC = 3 * WORD  # offset of the allocator in an owned string, buffer, list or map

let W = WORD, V = 8 + 4 * WORD
    for (T, n) in ((GAlloc, 5W), (GStr, 2W), (GBytes, 2W), (GBytesMut, 2W), (GString, 4W),
        (GBuffer, 4W), (GValue, V), (GList, 4W), (GEntry, 4W + V), (GMap, 4W),
        (GProviderError, 5W), (GDate, 8), (GTime, 8), (GDateTime, 16),
        (GTimestamp, 16), (GSpan, 64), (GTextError, W + V), (GKindHeader, 8), (GObject, 3W),
        (GKinds, 2W), (GKindTable, 8 + 3W), (GKindTables, 3W), (GProviderInfo, 8 + 20W),
        (GProviders, 3W), (GLibraryInfo, 8 + 9W), (GHostInfo, 8 + 7W),
        (GWireEvent, cld(16 + W + 3V, 8) * 8))
        sizeof(T) == n || error("$(T) is $(sizeof(T)) bytes, the header's is $n on this platform")
    end
end

# Views over a node's payload arms; `p` addresses a guatiao_value.
@inline payload_ptr(::Type{T}, p::Ptr{GValue}) where {T} = Ptr{T}(p + PAYLOAD)
@inline load_tag(p::Ptr{GValue}) = p == C_NULL ? TAG_ABSENT : unsafe_load(Ptr{UInt32}(p))

"A guatiao_value holding `arm` (a map, a list, a string, a buffer) under `tag`."
@inline function node_of(tag::UInt32, arm::T) where {T}
    r = Ref(GValue(tag, 0, (0, 0, 0, 0)))
    GC.@preserve r unsafe_store!(Ptr{T}(Base.unsafe_convert(Ptr{GValue}, r) + PAYLOAD), arm)
    return r[]
end

"The payload arm of `v` read as `T`."
@inline function arm_of(::Type{T}, v::GValue) where {T}
    r = Ref(v)
    GC.@preserve r unsafe_load(Ptr{T}(Base.unsafe_convert(Ptr{GValue}, r) + PAYLOAD))
end

const ZERO_VALUE = GValue(TAG_ABSENT, 0, (0, 0, 0, 0))
const NO_STRING = GString(C_NULL, 0, 0, C_NULL)
