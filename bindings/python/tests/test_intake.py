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
