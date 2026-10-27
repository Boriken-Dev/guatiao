# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
The kinds of `include/greeter_kind.h`, declared for Julia.

Written from the header: each slot in the order its table declares it,
its shapes as annotations, each `floor_hash` copied from the `#define`
beside the table. A test compares the literals with the header, so a kind
that changes shape fails there rather than in a call. A host calls a
greeter through `Greeter`; a Julia greeter is a struct with methods for
these functions.
"""
module GreeterKind

using Guatiao: @kind, Lent, Taken, MapRef

export Listener, Conversation, Greeter, Counter, Wide

"`listener_vtable`: what a conversation tells, implemented by the host."
@kind Listener floor_hash = 1867921573 object = true begin
    @required heard(what::String)::Nothing
end

"`conversation_vtable`: one conversation a greeter started."
@kind Conversation floor_hash = 2769624503 object = true begin
    @required @fallible say(what::String)::Nothing
    @required read(dst::Lent{Vector{UInt8}})::Int64
    @required turns()::Int64
end

"""
`greeter_vtable`: greets by name. `greet` is the one slot every provider
fills; the rest are optional, which a provider may leave out.
"""
@kind Greeter floor_hash = 2032737757 begin
    @required @fallible greet(name::String)::MapRef
    shout(name::String)::String
    @fallible start(listener::Taken{Listener})::Conversation
    @fallible greet_into(name::String, listener::Lent{Listener})::Nothing
    @fallible greet_from(config::Taken{MapRef})::MapRef
    @fallible check(config::Lent{MapRef})::Bool
end

"`counter_vtable`: counts how often it was asked."
@kind Counter floor_hash = 1227528937 begin
    @required count()::Int64
end

"`wide_vtable`: answers the 64-bit integer it is given, unchanged."
@kind Wide floor_hash = 3673689684 begin
    @required @fallible echo_i64(x::Int64)::Int64
    @required @fallible echo_u64(x::UInt64)::UInt64
end

end # module GreeterKind
