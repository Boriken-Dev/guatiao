# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""
Greeters written in Julia: the same kinds the Rust, C and Python greeters
serve, offered by a library any host in this process calls.

The package is the library: its id is the module's name, its version and
authors come from its `Project.toml`, and its providers are the structs
declared with `@provider`, in order.
"""
module JlGreeter

using Guatiao
using Guatiao: GuatiaoError, BadValue, MapRef
using GreeterKind
import GreeterKind: greet, shout, start, greet_into, greet_from, check, count, heard, say, read, turns

"One conversation `start` hands back, holding the listener it was handed."
mutable struct Chat
    listener::Any
    transcript::Vector{String}
end

function say(c::Chat, what::String)
    isempty(what) && throw(GuatiaoError(BadValue, "nothing to say"))
    push!(c.transcript, what)
    heard(c.listener, what)
    return nothing
end

function read(c::Chat, dst::Vector{UInt8})
    text = codeunits(join(c.transcript, "\n"))
    n = min(length(text), length(dst))
    copyto!(dst, 1, text, 1, n)
    return n
end

turns(c::Chat) = length(c.transcript)

"The listener a conversation holds is closed when the conversation is."
Base.close(c::Chat) = close(c.listener)

"One provider serving two kinds: its own one instance."
struct Hello <: Greeter
    asked::Threads.Atomic{Int}
end
Hello() = Hello(Threads.Atomic{Int}(0))

function greet(::Hello, name::String)
    name == "panic" && error("a provider bug, on request")
    isempty(name) && throw(GuatiaoError(BadValue, "nobody to greet"))
    return Dict("greeting" => "hello, $name")
end
shout(h::Hello, name::String) = uppercase(greet(h, name)["greeting"])
start(::Hello, listener) = Chat(listener, String[])
function greet_into(h::Hello, name::String, listener)
    heard(listener, greet(h, name)["greeting"])
    return nothing
end
function greet_from(h::Hello, config::Guatiao.Value)
    name = haskey(MapRef(config), "name") ? config["name"][] : ""
    close(config)
    return greet(h, string(name))
end
function check(::Hello, config::MapRef)
    name = haskey(config, "name") ? string(config["name"][]) : "world"
    config["name"] = name
    config["greeting"] = "hello, $name"
    return !isempty(name)
end
count(h::Hello) = Threads.atomic_add!(h.asked, 1) + 1

@provider Hello serves = (Greeter, Counter) display_name = "Hello, in Julia"

"What a shouter is built from; its schema is read from these fields."
Guatiao.@config struct ShouterConfig
    "What every greeting starts with."
    prefix::String
end
Guatiao.keywords(::Type{ShouterConfig}) = (prefix = (minLength = 1,),)

"Built from a configuration: one instance per configuration."
struct Shouter <: Greeter
    prefix::String
end
Shouter(c::ShouterConfig) = Shouter(c.prefix)

greet(s::Shouter, name::String) = Dict("greeting" => "$(s.prefix) $(uppercase(name))")

@provider Shouter config = ShouterConfig

end # module JlGreeter
