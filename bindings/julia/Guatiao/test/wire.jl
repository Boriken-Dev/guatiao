# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# The wire encoding and channels against the shared vectors every
# implementation reads: each value both ways, each malformed input
# refused, the frames the builders write, and a conversation with every
# refusal.

using Guatiao: GWireEvent

const WIRE = read_json(read(joinpath(REPO, "crates", "guatiao", "tests", "vectors", "wire.json"), String))

"A node as the vector file writes a tree."
function describe(n)
    t = Guatiao.tag(n)
    t in (:absent, :null) && return Any[string(t)]
    t === :bool && return Any["bool", n[]]
    t === :number && return Any["number", decode(NumberText, n).text]
    t === :string && return Any["string", n[]]
    t === :bytes && return Any["bytes", bytes2hex(n[])]
    t === :list && return Any["list", Any[describe(item) for item in ListRef(n)]]
    return Any["map", Any[Any[k, describe(MapRef(n)[k])] for k in keys(MapRef(n))]]
end

"The vector file's tree as plain data, numbers as their text."
untree(x::NumberText) = x.text
untree(x::AbstractVector) = Any[untree(i) for i in x]
untree(x) = x

"The value a vector tree names."
function build(tree)
    tag = tree[1]
    tag == "absent" && return Value(absent)
    tag == "null" && return Value(nothing)
    tag == "bool" && return Value(tree[2])
    tag == "number" && return Value(NumberText(tree[2]))
    tag == "string" && return Value(tree[2])
    tag == "bytes" && return Value(hex2bytes(tree[2]))
    if tag == "list"
        v = Value(Any[])
        for item in tree[2]
            child = build(item)
            push!(v, child)
            close(child)
        end
        return v
    end
    v = Value(Dict{String,Any}())
    for pair in tree[2]
        child = build(pair[2])
        v[pair[1]] = child
        close(child)
    end
    return v
end

"The channel a frame names, from its bytes."
function channel_of(frame::Vector{UInt8})
    n, shift = UInt64(0), 0
    for b in frame[2:end]
        n |= UInt64(b & 0x7f) << shift
        b & 0x80 == 0 && break
        shift += 7
    end
    return n
end

@testset "every value round-trips" begin
    @test !isempty(WIRE.values)
    for case in WIRE.values
        bytes = hex2bytes(case.hex)
        v = from_wire(bytes)
        @test describe(v) == untree(case.tree)
        @test to_wire(v) == bytes
        close(v)
        b = build(case.tree)
        @test to_wire(b) == bytes
        close(b)
    end
end

@testset "malformed bytes are refused" begin
    for case in WIRE.malformed
        err = try
            from_wire(hex2bytes(case.hex))
            nothing
        catch e
            e
        end
        @test err isa GuatiaoError && err.status == Guatiao.BadValue
    end
end

@testset "a child encodes alone" begin
    Value(Dict("outer" => Dict("inner" => Any[1, "two"]))) do v
        back = from_wire(to_wire(v["outer"]))
        @test back[] == Dict("inner" => Any[1, "two"])
        close(back)
    end
end

@testset "the frame builders write the documented bytes" begin
    for f in WIRE.frames
        expected = hex2bytes(f.hex)
        @test channel_of(expected) == parse(UInt64, f.channel.text)
        ch = channel_of(expected)
        if f.build == "close"
            @test Wire.close(ch) == expected
            continue
        end
        tree = build(f.tree)
        @test (f.build == "announce" ? Wire.announce(ch, tree) : Wire.send(ch, tree)) == expected
        close(tree)
    end
    @test Wire.send(7, Dict("greeting" => "hello")) == hex2bytes(WIRE.frames[2].hex)
end

@testset "the conversation runs through one receiver" begin
    statuses = Dict("malformed" => Guatiao.BadValue, "no_schema" => Guatiao.NotFound,
        "not_a_schema" => Guatiao.WrongKind, "invalid" => Guatiao.BadValue)
    r = Wire.Receiver()
    for step in WIRE.conversation
        frame = hex2bytes(step.frame)
        channel = haskey(step, :channel) ? parse(UInt64, step.channel.text) : UInt64(0)
        if haskey(step, :event)
            event = Wire.accept(r, frame)
            @test event.kind === Symbol(step.event)
            @test event.channel == channel
            if haskey(step, :tree)
                @test describe(event.value) == untree(step.tree)
                close(event.value)
            else
                @test event.value === nothing
            end
            continue
        end
        err = try
            Wire.accept(r, frame)
            nothing
        catch e
            e
        end
        @test err isa Wire.Refused
        err isa Wire.Refused || continue
        @test err.refusal === Symbol(step.refusal)
        @test Guatiao.status(err) == statuses[step.refusal]
        @test err.channel == channel
        @test !isempty(err.message)
        @test !occursin("hunter2", sprint(showerror, err))
        haskey(step, :kind) && @test err.kind == parse(Int, step.kind.text)
        haskey(step, :at) && @test err.at == parse(Int, step.at.text)
        haskey(step, :key) && @test err.key == step.key
    end
    close(r)
    close(r)
    @test !isopen(r)
    @test_throws InvalidStateException Wire.accept(r, Wire.close(1))
end

@testset "the event and the codes are the header's" begin
    text = read(HEADER, String)
    for (i, kind) in enumerate(Guatiao.Wire.KINDS)
        @test occursin("#define GUATIAO_FRAME_$(uppercase(string(kind))) $(i - 1)\n", text)
    end
    @test occursin("#define GUATIAO_WIRE_ACCEPTED 0\n", text)
    for (i, refusal) in enumerate(Guatiao.Wire.REFUSALS)
        @test occursin("#define GUATIAO_WIRE_$(uppercase(string(refusal))) $i\n", text)
    end
    @test sizeof(GWireEvent) == cld(16 + 3 * sizeof(GValue) + sizeof(Ptr{Cvoid}), 8) * 8
end
