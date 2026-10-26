# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    GuatiaoSerde

A guatiao value as JSON, TOML or YAML, through the `guatiao_serde`
library: `GuatiaoSerde.parse(text; format = :json)` and
`GuatiaoSerde.emit(x; format = :json, pretty = false)`;
`GuatiaoSerde.parse(Any, text)` reads plain Julia data with no `Value` in
between. TOML and YAML need
a `guatiao_serde` built with them. Built on Guatiao, whose `Value` it
answers and takes.
"""
module GuatiaoSerde

using Guatiao: Guatiao, Value, GuatiaoException, BadValue
using Guatiao.FFI: FFI, @cdef, Node, GValue, GString, GStr, GBytes, GAlloc, ZERO_VALUE, TAG_STRING, load_tag,
    payload_ptr, text_of, gstr, binding_alloc, new_root, free_root, adopt, require_node, owner_of, check,
    guatiao_value_free, read_plain, julia_to_wire

export ParseError

"""
    ParseError

A document that does not parse: `format` (`:json`, `:toml`, `:yaml`),
`line` and `column` counted from 1 (the column in characters; 0 when the
parser did not say), and `message`, which never quotes the document. Its
`Guatiao.status` is `BadValue`.
"""
struct ParseError <: GuatiaoException
    format::Symbol
    line::Int
    column::Int
    message::String
end
function Base.showerror(io::IO, e::ParseError)
    print(io, "ParseError (", e.format, "): ", e.message)
    e.line > 0 && print(io, " at line ", e.line, ", column ", e.column)
end
Guatiao.status(::ParseError) = BadValue

"`guatiao_parse_error`: where a document stopped parsing."
struct GParseError
    format::UInt32
    line::UInt32
    column::UInt32
    message::GValue
end
# Three uint32_t, then a value at its own alignment: 16 + the value on a
# 64-bit platform, 12 + it on a 32-bit one.
let n = (Guatiao.WORD == 8 ? 16 : 12) + sizeof(GValue)
    sizeof(GParseError) == n || error("guatiao_parse_error is $(sizeof(GParseError)) bytes, the header's is $n")
end

const PV = Ptr{GValue}
const PA = Ptr{GAlloc}

@cdef "guatiao_serde" "" :plain guatiao_json_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit_pretty(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_visit(text::GStr, how::UInt32, a::PA, v::Ptr{Cvoid}, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_visit(text::GStr, how::UInt32, a::PA, v::Ptr{Cvoid}, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_visit(text::GStr, how::UInt32, a::PA, v::Ptr{Cvoid}, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_from_wire(b::GBytes, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_from_wire(b::GBytes, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_from_wire(b::GBytes, how::UInt32, a::PA, out::PV)::UInt32

"""
    use_library(path)

Loads `guatiao_serde` from `path` (a file exporting its functions,
whatever it is called, or a directory holding it under its platform file
name) instead of looking for one: before the first call. Without it the
library is found through `GUATIAO_SERDE_LIBRARY`, then `GUATIAO_LIBRARY`
(a directory, or a file exporting it, else the file beside it), then the
platform's search.
"""
use_library(path::AbstractString) = Guatiao.use_library("guatiao_serde", path)

"How a byte string is spelled where the format has none: the low byte of `how`."
const BYTES_DATA_URI = UInt32(0)
const BYTES_BASE64 = UInt32(1)
const BYTES_ARRAY = UInt32(2)
const BYTES_REFUSE = UInt32(3)
"Read a `data:;base64,...` string back as bytes."
const READ_DATA_URIS = UInt32(1) << 8
"Indent for a person to read (JSON's own pretty writer is used for JSON)."
const PRETTY = UInt32(1) << 9

const FORMAT_NAMES = (:json, :toml, :yaml)

function parser(format::Symbol)
    format === :json && return guatiao_json_parse_ex
    format === :toml && return guatiao_toml_parse_ex
    format === :yaml && return guatiao_yaml_parse_ex
    throw(ArgumentError("unknown format: $format"))
end

"""
    parse(text; format = :json, how = 0, alloc) -> Value

`text` read as `format` (`:json`, `:toml`, `:yaml`) into a new `Value`.
Numbers keep their exact text. A document that does not parse is a
`ParseError` (`format`, `line`, `column`, `message`).
"""
function parse(text::AbstractString; format::Symbol = :json, how::Integer = 0, alloc = binding_alloc())
    f = parser(format)
    s = String(text)
    root = new_root()
    err = Ref(GParseError(0, 0, 0, ZERO_VALUE))
    status = GC.@preserve s err f(gstr(s), UInt32(how), alloc, root, err)
    status == 0 || free_root(root)
    parse_failed(status, err, format)
    return adopt(root, alloc)
end

"Frees the parse error's message, and throws for a refused `status`."
function parse_failed(status::UInt32, err::Ref{GParseError}, format::Symbol)
    message = GC.@preserve err begin
        p = Ptr{GValue}(Base.unsafe_convert(Ptr{GParseError}, err) + fieldoffset(GParseError, 4))
        m = load_tag(p) == TAG_STRING ? text_of(unsafe_load(payload_ptr(GString, p))) : ""
        guatiao_value_free(p)
        m
    end
    status == 0 && return nothing
    if status == UInt32(BadValue)
        e = err[]
        name = 1 <= e.format + 1 <= 3 ? FORMAT_NAMES[e.format+1] : format
        throw(ParseError(name, Int(e.line), Int(e.column), message))
    end
    check(status, message)
end

function visitor_call(format::Symbol)
    format === :json && return guatiao_json_visit
    format === :toml && return guatiao_toml_visit
    format === :yaml && return guatiao_yaml_visit
    throw(ArgumentError("unknown format: $format"))
end

"""
    parse(Any, text; format = :json, how = 0)

`text` read as `format` straight into plain Julia data, as
`parse(text)[]` answers it (`nothing`, `Bool`, numbers, `String`,
`Vector{UInt8}`, `Vector{Any}`, `Dict{String,Any}`), with no `Value`
built: the library hands each event to Julia as it reads. Errors as
`parse`.
"""
function parse(::Type{Any}, text::AbstractString; format::Symbol = :json, how::Integer = 0)
    f = visitor_call(format)
    s = String(text)
    err = Ref(GParseError(0, 0, 0, ZERO_VALUE))
    data, status = read_plain() do visitor
        GC.@preserve s err f(gstr(s), UInt32(how), binding_alloc(), visitor, err)
    end
    parse_failed(status, err, format)
    return data
end

"""
    emit(x; format = :json, pretty = false, how = 0) -> String

`x` (a `Value`, a view, or plain data) written as `format`. A TOML
document must be a map (`WrongKind`). Plain data is written as wire
bytes in Julia and crosses once, with no `Value` built.
"""
function emit(x; format::Symbol = :json, pretty::Bool = false, how::Integer = 0, alloc = binding_alloc())
    h = UInt32(how)
    f = if format === :json
        pretty ? guatiao_json_emit_pretty : guatiao_json_emit
    elseif format === :toml
        guatiao_toml_emit
    elseif format === :yaml
        guatiao_yaml_emit
    else
        throw(ArgumentError("unknown format: $format"))
    end
    format !== :json && pretty && (h |= PRETTY)
    x isa Node || return emit_data(x, format, h | (pretty ? PRETTY : UInt32(0)))
    keep = owner_of(x)
    GC.@preserve keep begin
        src = require_node(x)
        out = new_root()
        try
            check(f(src, h, alloc, out))
            load_tag(out) == TAG_STRING || return ""
            return text_of(unsafe_load(payload_ptr(GString, out)))
        finally
            free_root(out)
        end
    end
end

# Plain data written as wire bytes in Julia, crossing once, with no tree.
function emit_data(x, format::Symbol, how::UInt32)
    f = format === :json ? guatiao_json_from_wire : format === :toml ? guatiao_toml_from_wire : guatiao_yaml_from_wire
    data = julia_to_wire(x)
    out = new_root()
    try
        check(GC.@preserve data f(GBytes(isempty(data) ? Ptr{UInt8}(C_NULL) : pointer(data), length(data)), how,
            binding_alloc(), out))
        load_tag(out) == TAG_STRING || return ""
        return text_of(unsafe_load(payload_ptr(GString, out)))
    finally
        free_root(out)
    end
end

end # module GuatiaoSerde
