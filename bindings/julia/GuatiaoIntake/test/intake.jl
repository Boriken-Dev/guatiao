# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

const NAME_SCHEMA = raw"""{"type": "object", "properties": {"name": {"type": "string", "title": "Name"}},
    "required": ["name"], "additionalProperties": false}"""

const FLAT_SCHEMA = raw"""{
  "type": "object",
  "properties": {
    "auth": {"type": "object", "x-variant-tag": "auth", "oneOf": [
      {"type": "object", "properties": {"auth": {"const": "sso"}}, "required": ["auth"]},
      {"type": "object", "properties": {"auth": {"const": "userpass"}, "username": {"type": "string"},
        "password": {"type": "string", "x-sensitive": true}}, "required": ["auth"]}]},
    "agent": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}}}},
    "env": {"type": "object", "additionalProperties": {"type": "string"}},
    "host": {"type": "string", "title": "Host"}
  }
}"""

@testset "checking a form and laying it out" begin
    schema = Value(read_json(NAME_SCHEMA))
    @test Intake.check(schema, Dict()) === nothing
    err = Intake.check(schema, Dict("sections" => [], "fields" => Dict("nope" => Dict())))
    @test err["kind"] == "unknown_field"
    @test [k for s in Intake.layout(schema, Dict()) for k in s["fields"]] == ["name"]
    @test Intake.isvisible(schema, Dict(), "name", Dict("name" => "ana"))
    e = try Intake.isvisible(schema, Dict(), "nope", Dict()) catch e; e end
    @test e isa GuatiaoError && e.status == Guatiao.NotFound
    close(schema)
end

@testset "a path names one place inside a value" begin
    v = Value(Dict("agent" => [Dict("name" => "one"), Dict("name" => "two")],
        "env" => Dict("PATH" => "/bin", "a.b" => "dotted", "1" => "keyed")))
    @test Intake.at(v, "agent[0].name")[] == "one"
    @test Intake.at(v, "agent[1].name")[] == "two"
    @test Intake.at(v, "env[PATH]")[] == "/bin"
    @test Intake.at(v, "env[a.b]")[] == "dotted"
    @test Intake.at(v, "env[\"1\"]")[] == "keyed"
    @test Intake.at(v, "agent[9].name") === nothing
    @test Intake.at(v, "nonesuch") === nothing
    @test Intake.at(v, "agent[") === nothing
    view = Intake.at(v, "agent[0]")
    @test_throws ArgumentError (view["name"] = "x")   # borrowed: read-only
    close(v)
    @test_throws InvalidStateException view[]
end

@testset "a form made out of a schema" begin
    schema = Value(Dict("type" => "object", "additionalProperties" => false, "properties" => Dict(
        "host" => Dict("type" => "string", "x-section" => "net"),
        "tls" => Dict("type" => "object", "additionalProperties" => false,
            "properties" => Dict("verify" => Dict("type" => "boolean", "x-section" => "trust"))),
        "label" => Dict("type" => "string"))))
    made = Intake.for_schema(schema)
    doc = made[]
    @test [s["id"] for s in doc["sections"]] == ["net"]
    @test doc["fields"]["tls"]["form"]["sections"] == [Dict("id" => "trust")]
    @test Intake.check(schema, made) === nothing
    close(made)
    implied = Intake.form_for(Dict(), schema, "tls")
    @test implied isa Value
    close(implied)
    @test Intake.form_for(Dict(), schema, "label") === nothing
    @test Intake.form_for(Dict(), schema, "nonesuch") === nothing
    close(schema)
end

@testset "the flat projection" begin
    schema = Value(read_json(FLAT_SCHEMA))
    host = Intake.resolve(schema, "host")
    @test host["title"][] == "Host"
    @test Intake.resolve(schema, "auth.password")["x-sensitive"][] === true
    @test Intake.resolve(schema, "nonesuch") === nothing
    @test Intake.flat_keys(schema, "auth") == ["auth", "auth.username", "auth.password"]
    @test_throws GuatiaoError Intake.flat_keys(schema, "nonesuch")
    for (key, value, flat) in (
        ("auth", Dict("auth" => "userpass", "username" => "alice", "password" => "s3cret"),
            Dict("auth" => "userpass", "auth.username" => "alice", "auth.password" => "s3cret")),
        ("auth", Dict("auth" => "sso"), Dict("auth" => "sso")),
        ("agent", [Dict("name" => "one"), Dict("name" => "two")], Dict("agent[0].name" => "one", "agent[1].name" => "two")),
        ("env", Dict("PATH" => "/bin", "a.b" => "dotted"), Dict("env[PATH]" => "/bin", "env[a.b]" => "dotted")),
        ("host", "example.org", Dict("host" => "example.org")))
        @test Intake.flatten(schema, key, value) == flat
        back = Intake.unflatten(schema, key, flat)
        @test back[] == value
        close(back)
    end
    store = Value(Dict("env[PATH]" => "/bin"))
    env = Intake.unflatten(schema, "env", store)
    @test env[] == Dict("PATH" => "/bin")
    close(env)
    close(store)
    @test Intake.unflatten(schema, "auth", Dict("auth" => "nonesuch")) === nothing
    @test Intake.unflatten(schema, "nonesuch", Dict{String,String}()) === nothing
    @test Intake.unflatten(schema, "host", Dict{String,String}()) === nothing
    @test_throws ArgumentError Intake.unflatten(schema, "host", Dict("host" => 1))
    @test_throws ArgumentError Intake.unflatten(schema, "host", Value(Dict("host" => 1)))
    @test_throws GuatiaoError Intake.flatten(schema, "auth", [1])
    close(schema)
    @test_throws InvalidStateException host[]
end

@testset "redaction" begin
    schema = raw"""{"type": "object", "properties": {"user": {"type": "string"},
      "password": {"type": "string", "writeOnly": true, "x-sensitive": true},
      "tls": {"type": "object", "properties": {"ca": {"type": "string"}, "key": {"$ref": "#/$defs/Key"}}},
      "recovery": {"type": "array", "items": {"type": "string", "x-sensitive": true}}},
      "$defs": {"Key": {"type": "string", "x-sensitive": true}}}"""
    s = Value(read_json(schema))
    login = Dict("user" => "ana", "password" => "hunter2", "tls" => Dict("ca" => "/ca", "key" => "k"),
        "recovery" => ["r1", "r2"], "extra" => "?")
    shown = redact(s, login)
    @test shown[] == Dict("user" => "ana", "password" => "[redacted]", "tls" => Dict("ca" => "/ca", "key" => "[redacted]"),
        "recovery" => ["[redacted]", "[redacted]"], "extra" => "[redacted]")
    close(shown)
    Value(Dict("inner" => login)) do doc
        @test redact(s, doc["inner"]; placeholder = "***")["password"][] == "***"
    end
    @test_throws GuatiaoError redact([1], login)
    close(s)
end
