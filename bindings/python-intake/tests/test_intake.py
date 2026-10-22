# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao_intake` against a real `--all-features` build, over a schema
and a form built as plain Python dicts."""

from __future__ import annotations

import pytest

import guatiao_intake as intake
from guatiao import LibraryNotFound, Value, WrongKind

try:
    intake.library()
except LibraryNotFound:
    pytest.skip("no guatiao_intake library built", allow_module_level=True)

_SCHEMA = {
    "type": "object",
    "properties": {
        "name": {"type": "string", "title": "Name"},
    },
    "required": ["name"],
    "additionalProperties": False,
}


def test_check_passes_a_form_that_fits():
    with Value.from_python(_SCHEMA) as schema, Value.from_python({}) as blank:
        assert intake.check(schema, blank) is None


def test_check_reports_an_unknown_field():
    bad_form = {
        "sections": [],
        "fields": {"nope": {}},
    }
    with Value.from_python(_SCHEMA) as schema, Value.from_python(bad_form) as bad:
        error = intake.check(schema, bad)
        assert error is not None
        assert error["kind"] == "unknown_field"


def test_layout_lists_the_declared_field():
    with Value.from_python(_SCHEMA) as schema, Value.from_python({}) as blank:
        sections = intake.layout(schema, blank)
        fields = [key for section in sections for key in section["fields"]]
        assert fields == ["name"]


def test_is_visible_with_no_condition():
    with Value.from_python(_SCHEMA) as schema, Value.from_python({}) as blank:
        with Value.from_python({"name": "ana"}) as values:
            assert intake.is_visible(schema, blank, "name", values) is True


def test_is_visible_unknown_key_is_not_found():
    from guatiao import NotFound

    with Value.from_python(_SCHEMA) as schema, Value.from_python({}) as blank:
        with Value.from_python({}) as values:
            with pytest.raises(NotFound):
                intake.is_visible(schema, blank, "nope", values)


def test_a_path_names_one_place_inside_a_value():
    """`agent[1].name`: a dot is a member, a bracket is a position or a
    key, and which one it is depends on what it is applied to."""
    value = Value.from_python(
        {
            "agent": [{"name": "one"}, {"name": "two"}],
            "env": {"PATH": "/bin", "a.b": "dotted", "1": "keyed"},
        }
    )
    try:
        at = lambda path: intake.at(value, path)

        assert at("agent[0].name").to_python() == "one"
        assert at("agent[1].name").to_python() == "two"
        assert at("env[PATH]").to_python() == "/bin"

        # No quoting needed inside brackets: the `]` ends the segment.
        assert at("env[a.b]").to_python() == "dotted"
        # But a key that would read as a position needs it.
        assert at("env[\"1\"]").to_python() == "keyed"

        # Nothing there, and not a path at all, are the same answer here.
        assert at("agent[9].name") is None
        assert at("nonesuch") is None
        assert at("agent[") is None
    finally:
        value.close()


def test_a_form_can_be_made_out_of_a_schema_and_a_member_carries_its_own():
    """An object is a form: the schema groups the fields, so a renderer
    with no form still has one to draw."""
    schema = Value.from_python(
        {
            "type": "object",
            "properties": {
                "host": {"type": "string", "x-section": "net"},
                "port": {"type": "integer", "x-section": "net"},
                "tls": {
                    "type": "object",
                    "properties": {
                        "verify": {"type": "boolean", "x-section": "trust"},
                    },
                    "additionalProperties": False,
                },
                "label": {"type": "string"},
            },
            "additionalProperties": False,
        }
    )
    try:
        made = intake.for_schema(schema)
        try:
            doc = made.to_python()
            # Ids only, in first-appearance order: what a section is
            # CALLED is a form's business, and a schema has no opinion.
            assert [s["id"] for s in doc["sections"]] == ["net"]
            assert doc["fields"]["tls"]["form"]["sections"] == [{"id": "trust"}]
            # And what it made fits what it was made from.
            assert intake.check(schema, made) is None
        finally:
            made.close()

        # `form_for`: assigned wins, and a field with no members has none.
        blank = Value.from_python({})
        try:
            implied = intake.form_for(blank, schema, "tls")
            assert implied is not None
            implied.close()
            assert intake.form_for(blank, schema, "label") is None
            assert intake.form_for(blank, schema, "nonesuch") is None
        finally:
            blank.close()
    finally:
        schema.close()


_FLAT_SCHEMA = {
    "type": "object",
    "properties": {
        "auth": {
            "type": "object",
            "x-variant-tag": "auth",
            "oneOf": [
                {
                    "type": "object",
                    "properties": {"auth": {"const": "sso"}},
                    "required": ["auth"],
                },
                {
                    "type": "object",
                    "properties": {
                        "auth": {"const": "userpass"},
                        "username": {"type": "string"},
                        "password": {"type": "string", "x-sensitive": True},
                    },
                    "required": ["auth"],
                },
            ],
        },
        "agent": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
            },
        },
        "env": {"type": "object", "additionalProperties": {"type": "string"}},
        "host": {"type": "string", "title": "Host"},
    },
}


def test_resolve_answers_the_field_that_governs_a_flat_key():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        host = intake.resolve(schema, "host")
        assert host is not None
        assert host.as_map()["title"].to_python() == "Host"
        # One level of projection: a key inside an arm answers that field.
        password = intake.resolve(schema, "auth.password")
        assert password is not None
        assert password.as_map()["x-sensitive"].to_python() is True
        assert intake.resolve(schema, "nonesuch") is None


def test_flat_keys_lists_a_variant_field_and_its_arms():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        assert intake.flat_keys(schema, "auth") == [
            "auth",
            "auth.username",
            "auth.password",
        ]
        with pytest.raises(WrongKind):
            intake.flat_keys(schema, "nonesuch")


@pytest.mark.parametrize(
    "key, value, flat",
    [
        (
            "auth",
            {"auth": "userpass", "username": "alice", "password": "s3cret"},
            {
                "auth": "userpass",
                "auth.username": "alice",
                "auth.password": "s3cret",
            },
        ),
        ("auth", {"auth": "sso"}, {"auth": "sso"}),
        (
            "agent",
            [{"name": "one"}, {"name": "two"}],
            {"agent[0].name": "one", "agent[1].name": "two"},
        ),
        (
            "env",
            {"PATH": "/bin", "a.b": "dotted"},
            {"env[PATH]": "/bin", "env[a.b]": "dotted"},
        ),
        ("host", "example.org", {"host": "example.org"}),
    ],
)
def test_a_value_flattens_and_reads_back(key, value, flat):
    with Value.from_python(_FLAT_SCHEMA) as schema, Value.from_python(value) as given:
        assert intake.flatten(schema, key, given) == flat
        back = intake.unflatten(schema, key, flat)
        assert back is not None
        with back:
            assert back.to_python() == value


def test_unflatten_takes_a_value_and_answers_none_when_it_does_not_apply():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        with Value.from_python({"env[PATH]": "/bin"}) as store:
            with intake.unflatten(schema, "env", store) as env:
                assert env.to_python() == {"PATH": "/bin"}
        # A tag that names no arm, a key that names no field, and nothing
        # stored under the field are all "does not apply".
        assert intake.unflatten(schema, "auth", {"auth": "nonesuch"}) is None
        assert intake.unflatten(schema, "nonesuch", {}) is None
        assert intake.unflatten(schema, "host", {}) is None
        with pytest.raises(TypeError):
            intake.unflatten(schema, "host", {"host": 1})


def test_flatten_refuses_a_value_of_the_wrong_shape():
    with Value.from_python(_FLAT_SCHEMA) as schema, Value.from_python([1]) as given:
        with pytest.raises(WrongKind):
            intake.flatten(schema, "auth", given)

def _churn():
    """Reuses freed heap blocks, so a read through a stale address shows."""
    return [Value.from_python({"k" * 40: ["x" * 64] * 8}) for _ in range(200)]


def test_a_resolved_field_refuses_to_be_read_once_its_schema_is_closed():
    schema = Value.from_python(_FLAT_SCHEMA)
    host = intake.resolve(schema, "host")
    schema.close()
    junk = _churn()
    with pytest.raises(ValueError, match="closed"):
        host.to_python()
    for item in junk:
        item.close()


def test_an_at_view_refuses_to_be_read_once_its_value_is_closed():
    value = Value.from_python({"a": {"b": "c" * 50}})
    view = intake.at(value, "a")
    value.close()
    junk = _churn()
    with pytest.raises(ValueError, match="closed"):
        view.to_python()
    for item in junk:
        item.close()


def test_a_temporary_schema_lives_as_long_as_the_field_resolved_from_it():
    import gc

    host = intake.resolve(Value.from_python(_FLAT_SCHEMA), "host")
    gc.collect()
    junk = _churn()
    assert host.as_map()["title"].to_python() == "Host"
    for item in junk:
        item.close()


def test_a_resolved_field_follows_a_change_to_its_schema():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        host = intake.resolve(schema, "host")
        schema.as_map()["properties"].as_map()["host"] = {"title": "Moved"}
        assert host.as_map()["title"].to_python() == "Moved"


def test_nothing_can_be_changed_through_a_borrowed_view():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        host = intake.resolve(schema, "host")
        as_map = host.as_map()
        with pytest.raises(TypeError):
            del as_map["title"]
        with pytest.raises(TypeError):
            as_map["title"] = "x"
        with pytest.raises(TypeError):
            as_map.clear()
        with pytest.raises(TypeError):
            as_map.copy_from(as_map)
        assert host.as_map()["title"].to_python() == "Host"

    with Value.from_python({"tags": ["a", "b"]}) as value:
        tags = intake.at(value, "tags").as_list()
        with pytest.raises(TypeError):
            tags.append("c")
        with pytest.raises(TypeError):
            del tags[0]
        with pytest.raises(TypeError):
            tags[0] = "z"
        with pytest.raises(TypeError):
            tags.insert(0, "z")
        with pytest.raises(TypeError):
            tags.clear()
        assert value.to_python() == {"tags": ["a", "b"]}
        # Reading, and copying out, are not changes.
        with tags.clone() as copy:
            assert copy.to_python() == ["a", "b"]


def test_the_flat_calls_take_a_ref_where_they_take_a_value():
    with Value.from_python(_FLAT_SCHEMA) as schema, Value.from_python(
        {"host": "h", "env": {"PATH": "/bin"}}
    ) as config:
        fields = config.as_map()
        assert intake.flatten(schema, "host", fields["host"]) == {"host": "h"}
        assert intake.flatten(schema, "env", fields["env"]) == {"env[PATH]": "/bin"}
        with Value.from_python({"flat": {"env[PATH]": "/bin"}}) as holder:
            back = intake.unflatten(schema, "env", holder.as_map()["flat"])
            with back:
                assert back.to_python() == {"PATH": "/bin"}


def test_a_store_that_is_not_text_to_text_is_a_type_error_in_every_form():
    with Value.from_python(_FLAT_SCHEMA) as schema:
        with Value.from_python({"host": 1}) as numbers:
            with pytest.raises(TypeError):
                intake.unflatten(schema, "host", numbers)
        with Value.from_python(["host"]) as not_a_map:
            with pytest.raises(TypeError):
                intake.unflatten(schema, "host", not_a_map)
        with pytest.raises(TypeError):
            intake.unflatten(schema, "host", {"host": 1})
        with pytest.raises(TypeError):
            intake.unflatten(schema, "host", ["host"])
        # Nothing stored is not a malformed store.
        with Value.from_python({}) as empty:
            assert intake.unflatten(schema, "host", empty) is None
