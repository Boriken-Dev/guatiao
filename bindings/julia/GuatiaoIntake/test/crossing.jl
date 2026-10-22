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
