# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    GuatiaoSerde

A guatiao value as JSON, TOML or YAML, through the `guatiao_serde`
library: `GuatiaoSerde.parse(text; format = :json)` and
`GuatiaoSerde.emit(x; format = :json, pretty = false)`. TOML and YAML need
a `guatiao_serde` built with them. Built on Guatiao, whose `Value` it
answers and takes.
"""
module GuatiaoSerde

using Guatiao: Guatiao, Value, GuatiaoException, BadValue
using Guatiao.FFI: FFI, @cdef, Node, GValue, GString, GStr, GAlloc, ZERO_VALUE, TAG_STRING, load_tag, payload_ptr,
    text_of, gstr, binding_alloc, new_root, free_root, adopt, require_node, owner_of, check, guatiao_value_free

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
sizeof(GParseError) == 56 || error("guatiao_parse_error is $(sizeof(GParseError)) bytes, the header's is 56")

const PV = Ptr{GValue}
const PA = Ptr{GAlloc}

@cdef "guatiao_serde" "" :plain guatiao_json_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "" :plain guatiao_json_emit_pretty(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "toml" :plain guatiao_toml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_parse_ex(text::GStr, how::UInt32, a::PA, out::PV, err::Ptr{GParseError})::UInt32
@cdef "guatiao_serde" "yaml" :plain guatiao_yaml_emit(v::PV, how::UInt32, a::PA, out::PV)::UInt32

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
    message = GC.@preserve err begin
        p = Ptr{GValue}(Base.unsafe_convert(Ptr{GParseError}, err) + 16)
        m = load_tag(p) == TAG_STRING ? text_of(unsafe_load(payload_ptr(GString, p))) : ""
        guatiao_value_free(p)
        m
    end
    if status != 0
        free_root(root)
        if status == UInt32(BadValue)
            e = err[]
            name = 1 <= e.format + 1 <= 3 ? FORMAT_NAMES[e.format+1] : format
            throw(ParseError(name, Int(e.line), Int(e.column), message))
        end
        check(status, message)
    end
    return adopt(root, alloc)
end

"""
    emit(x; format = :json, pretty = false, how = 0) -> String

`x` (a `Value`, a view, or plain data) written as `format`. A TOML
document must be a map (`WrongKind`).
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
    owned = x isa Node ? nothing : Value(x; alloc)
    keep = x isa Node ? owner_of(x) : owned
    GC.@preserve keep try
        src = require_node(owned === nothing ? x : owned)
        out = new_root()
        try
            check(f(src, h, alloc, out))
            load_tag(out) == TAG_STRING || return ""
            return text_of(unsafe_load(payload_ptr(GString, out)))
        finally
            free_root(out)
        end
    finally
        owned === nothing || close(owned)
    end
end

end # module GuatiaoSerde
