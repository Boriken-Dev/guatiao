# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    Status

`guatiao_status`: what an entry point reports. `OK`, `BadValue`,
`AllocFailed`, `WrongKind`, `NotFound`, `NullArgument`, `Gone`,
`Unsupported`, `Busy`, `Internal`.
"""
@enum Status::UInt32 begin
    OK = 0
    BadValue = 7
    AllocFailed = 8
    WrongKind = 9
    NotFound = 10
    NullArgument = 11
    Gone = 12
    Unsupported = 13
    Busy = 14
    Internal = 100
end

"The base of every error a failing `guatiao_status` raises."
abstract type GuatiaoException <: Exception end

"""
    GuatiaoError(status::Status, message = "")

One failing status, and the other side's own words (empty when it gave
none). Thrown by a provider's method, it crosses as that status and
message: `throw(GuatiaoError(BadValue, "nobody to greet"))`. `code` is the
raw number, for a status this binding does not name (`status` is then
`Internal`).
"""
struct GuatiaoError <: GuatiaoException
    status::Status
    message::String
    code::UInt32
end
GuatiaoError(status::Status, message::AbstractString = "") = GuatiaoError(status, String(message), UInt32(status))
function GuatiaoError(code::Integer, message::AbstractString = "")
    c = UInt32(code)
    known = c in _KNOWN_CODES
    return GuatiaoError(known ? Status(c) : Internal, String(message), c)
end
const _KNOWN_CODES = Set(UInt32(s) for s in instances(Status))

function Base.showerror(io::IO, e::GuatiaoError)
    name = UInt32(e.status) == e.code ? string(e.status) : "status $(e.code)"
    print(io, "GuatiaoError: ", name)
    isempty(e.message) || print(io, ": ", e.message)
end

"""
    ParseError

A document that does not parse: `format` (`:json`, `:toml`, `:yaml`),
`line` and `column` counted from 1 (the column in characters; 0 when the
parser did not say), and `message`, which never quotes the document. Its
`status` is `BadValue`.
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

"""
    Unparsed

A date, time, instant or span whose text its grammar refused: `form`
(`:date`, `:time`, `:datetime`, `:timestamp`, `:span`), `at` the byte
offset in the UTF-8 text where reading stopped, `expected` what would
have been accepted there. Neither quotes the text. Its `status` is
`BadValue`.
"""
struct Unparsed <: GuatiaoException
    form::Symbol
    at::Int
    expected::String
end
Base.showerror(io::IO, e::Unparsed) = print(io, "Unparsed ", e.form, ": ", e.expected, " at byte ", e.at)

"The status an error crosses with."
status(e::GuatiaoError) = e.status
status(::ParseError) = BadValue
status(::Unparsed) = BadValue

"""
    LibraryNotFound

No `basename` library was found: `looked` lists every place tried.
"""
struct LibraryNotFound <: Exception
    basename::String
    looked::Vector{String}
end
Base.showerror(io::IO, e::LibraryNotFound) =
    print(io, "LibraryNotFound: no ", e.basename, " library; looked at: ", join(e.looked, ", "))

"""
    MissingSymbol

`symbol` is not exported by the library loaded from `path`: it was built
without `feature` (empty for an export every build has).
"""
struct MissingSymbol <: Exception
    path::String
    symbol::String
    feature::String
end
function Base.showerror(io::IO, e::MissingSymbol)
    print(io, "MissingSymbol: ", e.path, " was not built with ", e.symbol)
    isempty(e.feature) || print(io, " (feature ", e.feature, ")")
end

"""
    FloorMismatch

A table that is not the kind's: smaller than its required slots, or
with another floor hash.
"""
struct FloorMismatch <: Exception
    message::String
end
Base.showerror(io::IO, e::FloorMismatch) = print(io, "FloorMismatch: ", e.message)

closed_error(what::AbstractString = "value") = InvalidStateException("this $what is closed", :closed)
read_only_error() = ArgumentError("a borrowed value is read-only")

"Throws for a failing `status`, with `message` as the other side's words."
@inline function check(status::UInt32, message::AbstractString = "")
    status == 0 && return nothing
    throw(GuatiaoError(status, message))
end
