# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A value as bytes (the wire encoding), and channels: a schema announced
# before its values, checked by a receiver.

"""
    from_wire(bytes; alloc) -> Value

The value the wire encoding in `bytes` holds. A `GuatiaoError` with
status `BadValue` for bytes that are not exactly one value: truncated, an
unknown tag, a length not in its shortest form, text that is not UTF-8,
a number outside the grammar, a repeated key, anything after the value.
"""
function from_wire(data::AbstractVector{UInt8}; alloc::Ptr{GAlloc} = binding_alloc())
    bytes = Vector{UInt8}(data)
    root = new_root()
    status = GC.@preserve bytes guatiao_wire_decode(alloc, gbytes(bytes), root)
    status == 0 || (free_root(root); check(status, "bytes that are not a value in the wire encoding"))
    return adopt(root, alloc)
end

"The bytes a BYTES node at `out` holds, and `out` freed."
function take_bytes(out::Ptr{GValue})
    try
        b = unsafe_load(payload_ptr(GBuffer, out))
        return b.len == 0 ? UInt8[] : copy(unsafe_wrap(Vector{UInt8}, b.ptr, b.len))
    finally
        free_root(out)
    end
end

"""
    to_wire(n) -> Vector{UInt8}

The node `n` (a `Value` or a view into one) in the wire encoding: a tag
byte per node, LEB128 lengths, a number as its own text, a map in its
order. `from_wire`, in any language, reads the same tree back.
"""
function to_wire(n::Node)
    out = new_root()
    status = @held n guatiao_wire_encode(binding_alloc(), require_node(n), out)
    status == 0 || (free_root(out); check(status))
    return take_bytes(out)
end

"""
    Wire

Channels: a schema announced before its values, so a receiver knows what
each value means and refuses what does not fit it. A frame is
`kind:u8 channel:LEB128 payload` (kind 0 a schema, 1 a value, 2 the
channel's end); the transport says where one ends.

`Wire.announce(channel, schema)`, `Wire.send(channel, value)` and
`Wire.close(channel)` write frames; `Wire.accept(receiver, frame)`
answers a `Wire.Event` or throws a `Wire.Refused`. `Wire.close` writes a
frame; `close(receiver)` is `Base.close`.
"""
module Wire

import ..Guatiao
using ..Guatiao: GValue, GAlloc, GWireEvent, Node, Value, Status, GuatiaoException, BadValue,
    NotFound, WrongKind, binding_alloc, require_node, owner_of, new_root, free_root, adopt, plain,
    check, gbytes, take_bytes, closed_error, ZERO_VALUE, @held,
    guatiao_wire_announce, guatiao_wire_send, guatiao_wire_close, guatiao_wire_receiver_new,
    guatiao_wire_receiver_accept, guatiao_wire_receiver_free, guatiao_value_free

"A frame's kind, by its byte: `GUATIAO_FRAME_SCHEMA` 0, `_VALUE` 1, `_CLOSE` 2."
const KINDS = (:schema, :value, :close)

# A guatiao_wire_event's `refusal`: GUATIAO_WIRE_ACCEPTED 0, then these.
const REFUSALS = (:malformed, :no_schema, :not_a_schema, :invalid)

"""
    Event

What a frame said: `kind` (`:schema`, `:value` or `:close`), `channel`
(a `UInt64`), and `value`: the schema now the channel's, or the value its
schema accepted, as a `Value` the caller closes; `nothing` for a close.
"""
struct Event
    kind::Symbol
    channel::UInt64
    value::Union{Value,Nothing}
end

"""
    Refused

A frame a receiver refused. `refusal` is `:malformed` (bytes that are not
a frame; `at` is the byte of the frame it went wrong at), `:no_schema` (a
value on a channel no schema was announced on), `:not_a_schema` (a schema
frame whose value is not a schema) or `:invalid` (a value its channel's
schema refuses: `key` the field, `expected` what it accepts or `known`
the fields there are). `status` is the status it crossed with
(`BadValue`, `NotFound`, `WrongKind`), `channel` the channel it named (0
when the frame ended before one), `kind` its kind byte. Nothing the
frame carried is quoted.
"""
struct Refused <: GuatiaoException
    refusal::Symbol
    status::Status
    message::String
    channel::UInt64
    kind::UInt32
    at::Int
    key::String
    expected::Union{String,Nothing}
    known::Union{Vector{String},Nothing}
end
Guatiao.status(e::Refused) = e.status
Base.showerror(io::IO, e::Refused) =
    print(io, "Wire.Refused(", e.refusal, ", ", e.status, ", channel ", e.channel, "): ", e.message)

"The frame `build(alloc, out)` writes, as bytes."
function frame(build)
    out = new_root()
    status = build(binding_alloc(), out)
    status == 0 || (free_root(out); check(status))
    return take_bytes(out)
end

"Calls `f` with `x`'s node: a `Value` or view as it is, plain data built for the call."
function with_node(f, x)
    if x isa Node
        return @held x f(require_node(x))
    end
    v = Value(x)
    try
        return GC.@preserve v f(require_node(v))
    finally
        Base.close(v)
    end
end

"""
    announce(channel, schema) -> Vector{UInt8}

The frame that announces `schema` (a `Value`, a view, or plain data) on
`channel`, 0 to `typemax(UInt64)`. A later one replaces it.
"""
announce(channel::Integer, schema) =
    with_node(node -> frame((a, out) -> guatiao_wire_announce(a, UInt64(channel), node, out)), schema)

"""
    send(channel, value) -> Vector{UInt8}

The frame that carries `value` on `channel`.
"""
send(channel::Integer, value) =
    with_node(node -> frame((a, out) -> guatiao_wire_send(a, UInt64(channel), node, out)), value)

"""
    Wire.close(channel) -> Vector{UInt8}

The frame that ends `channel`: a receiver forgets its schema. Not
`Base.close`, which closes a `Receiver`.
"""
close(channel::Integer) = frame((a, out) -> guatiao_wire_close(a, UInt64(channel), out))

"""
    Receiver()

Keeps each channel's schema and checks every value against it.
`close(receiver)` frees it (a finalizer is the backstop); calls on one
receiver are serialised by its lock.
"""
mutable struct Receiver
    handle::Ptr{Cvoid}
    lock::ReentrantLock
    function Receiver()
        handle = guatiao_wire_receiver_new(binding_alloc())
        handle == C_NULL && throw(Guatiao.GuatiaoError(Guatiao.AllocFailed, "the allocator was refused"))
        r = new(handle, ReentrantLock())
        finalizer(release!, r)
        return r
    end
end

function release!(r::Receiver)
    handle = r.handle
    handle == C_NULL && return nothing
    r.handle = C_NULL
    guatiao_wire_receiver_free(handle)
    return nothing
end

Base.close(r::Receiver) = lock(() -> release!(r), r.lock)
Base.isopen(r::Receiver) = r.handle != C_NULL

const VALUE_AT = fieldoffset(GWireEvent, 5)
const MESSAGE_AT = fieldoffset(GWireEvent, 6)
const DETAIL_AT = fieldoffset(GWireEvent, 7)

"""
    accept(receiver, frame) -> Event

Reads one frame: an `Event`, or a `Refused` thrown. A refusal changes
nothing the receiver keeps.
"""
function accept(r::Receiver, data::AbstractVector{UInt8})
    bytes = Vector{UInt8}(data)
    ev = Ptr{GWireEvent}(Libc.calloc(1, sizeof(GWireEvent)))
    ev == C_NULL && throw(OutOfMemoryError())
    value = Ptr{GValue}(ev + VALUE_AT)
    message = Ptr{GValue}(ev + MESSAGE_AT)
    detail = Ptr{GValue}(ev + DETAIL_AT)
    try
        status = lock(r.lock) do
            r.handle == C_NULL && throw(closed_error("receiver"))
            GC.@preserve bytes guatiao_wire_receiver_accept(r.handle, gbytes(bytes), ev)
        end
        e = unsafe_load(ev)
        words = plain(message)
        found = plain(detail)
        taken = nothing
        if status == 0 && e.value.tag != Guatiao.TAG_ABSENT
            root = new_root()
            unsafe_copyto!(root, value, 1)
            unsafe_store!(value, ZERO_VALUE)
            taken = adopt(root, binding_alloc())
        end
        status == 0 && return Event(KINDS[e.kind+1], e.channel, taken)
        text = words isa String ? words : ""
        if 1 <= e.refusal <= length(REFUSALS)
            d = found isa Dict ? found : Dict{String,Any}()
            known = get(d, "known", nothing)
            expected = get(d, "expected", nothing)
            throw(Refused(REFUSALS[e.refusal], Status(status), text, e.channel, e.kind, Int(e.at),
                String(get(d, "key", "")), expected isa String ? expected : nothing,
                known isa Vector ? String[String(k) for k in known] : nothing))
        end
        check(status, text)
        error("unreachable: a refused status throws")
    finally
        # The event is calloc'd: an unwritten one is three ABSENT values.
        guatiao_value_free(value)
        guatiao_value_free(message)
        guatiao_value_free(detail)
        Libc.free(ev)
    end
end

end # module Wire
