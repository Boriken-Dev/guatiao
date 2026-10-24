# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# A tree crosses between guatiao_intake and guatiao: built by one library,
# read and freed through the other, every block accounted for.

using Guatiao: MapRef

@testset "a tree the core built is read by intake, and the view refuses once it closes" begin
    c = Counting()
    v = Value(Dict("agent" => [Dict("name" => "a"), Dict("name" => "b")]); alloc = c.alloc)
    found = Intake.at(v, "agent[1].name")
    @test found[] == "b"
    close(v)
    @test_throws InvalidStateException found[]
    @test live(c) == 0
end

@testset "a tree intake built is changed and freed through the core" begin
    schema = Value(read_json("""{"type": "object", "properties": {"name": {"type": "string", "x-section": "who"}}}"""))
    form = Intake.for_schema(schema)
    MapRef(form)["note"] = "added through the core"
    @test MapRef(form)["sections"][] == [Dict("id" => "who")]
    close(form)
    close(schema)
end

@testset "a provider's form, read from its meta, fits its schema" begin
    named = ENV["GUATIAO_LIBRARY"]
    reg = Registry("julia-intake-tests", "0.1")
    Guatiao.scan_dir(reg, isdir(named) ? named : dirname(named); rules = "kind=greeter")
    meta = Guatiao.provider_meta(reg, "derived_greeter_shouter")
    form = meta[Intake.PROVIDER_FORM_KEY]
    schema = Guatiao.provider_config(reg, "derived_greeter_shouter")
    @test Intake.check(schema, form) === nothing
    @test form["fields"]["prefix"]["placeholder"][] == "hey"
    @test form["fields"]["marks"]["unit"][] == "marks"
    @test !Intake.isvisible(schema, form, "marks", Dict("exclaim" => false))
    @test Intake.isvisible(schema, form, "marks", Dict("exclaim" => true))
    close(meta)
    close(reg)
end
