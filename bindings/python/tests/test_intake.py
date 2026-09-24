# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`guatiao_intake` against a real `--all-features` build, over a schema
and a form built as plain Python dicts."""

from __future__ import annotations

import pytest

from guatiao import _lib

try:
    _lib.form()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao_intake library built", allow_module_level=True)

from guatiao import intake
from guatiao.value import Value

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
    from guatiao.errors import NotFound

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
