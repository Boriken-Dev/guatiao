# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
    Guatiao

Julia bindings over the guatiao C ABI: one value model for passing data
between languages, a JSON Schema that describes a value, and a registry
that loads plugin libraries, whatever language wrote them. A pure
consumer of the ABI: point it at a library with `GUATIAO_LIBRARY` or
`Guatiao.use_library`.
"""
module Guatiao

using Dates

include("abi.jl")
include("errors.jl")
include("native.jl")
include("value.jl")
include("typed.jl")
include("time.jl")
include("redact.jl")
include("serde.jl")
include("intake.jl")

export Value, ValueRef, MapRef, ListRef, NumberText, absent, decode, schema, redact,
    GuatiaoError, ParseError, Unparsed, LibraryNotFound, MissingSymbol, FloorMismatch,
    Timestamp, Span, Length, Serde, Intake

function __init__()
    reset_native!()
end

end # module Guatiao
